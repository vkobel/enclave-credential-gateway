//! Profile loading and route definitions.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

const EMBEDDED_PROFILE_PATH: &str = "profiles/coco.yaml";
const EMBEDDED_PROFILE_YAML: &str = include_str!("../../../profiles/coco.yaml");

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialSource {
    pub env: String,
    pub inject_header: String,
    #[serde(default = "default_credential_format")]
    pub format: String,
    #[serde(default)]
    pub prefix: Option<String>,
    /// When set, the credential is injected as HTTP Basic auth:
    /// `Basic base64("<basic_user>:<credential>")`. The `format` field is ignored.
    #[serde(default)]
    pub basic_user: Option<String>,
}

fn default_credential_format() -> String {
    "Bearer {}".to_string()
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InjectMode {
    #[default]
    Header,
    UrlPath,
}

/// How a request path is matched to a [`RouteEntry`].
#[derive(Debug, Clone, Default, PartialEq)]
pub enum RouteMatcher {
    /// First path segment must equal the route key (default behaviour).
    #[default]
    Prefix,
    /// Path matches `/<owner>/<repo>.git/(info/refs|git-upload-pack|git-receive-pack)`.
    /// Used for the GitHub git smart-HTTP endpoint, which is served on a different
    /// host than the REST API and does not share a stable URL prefix.
    GitSmartHttp,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Profile {
    pub(crate) routes: BTreeMap<String, ProfileRoute>,
    #[serde(default, rename = "tools")]
    pub(crate) _tools: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileRoute {
    pub upstream: String,
    #[serde(default)]
    pub credential_sources: Vec<CredentialSource>,
    #[serde(default)]
    pub aliases: Vec<RouteAlias>,
    #[serde(default)]
    pub inject_mode: InjectMode,
    #[serde(default)]
    pub url_path_prefix: Option<String>,
    /// Optional companion route for tools that serve a non-API protocol on a
    /// separate host. Currently only used by `github` to proxy git smart-HTTP
    /// requests at `github.com` while the REST API stays on `api.github.com`.
    #[serde(default)]
    pub git_protocol: Option<GitProtocolRoute>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteAlias {
    pub prefix: String,
    #[serde(default)]
    pub strip_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitProtocolRoute {
    pub upstream: String,
    /// When set, overrides the parent route's `credential_sources` for git
    /// smart-HTTP requests. Useful when the git host requires different auth
    /// (e.g. Basic) than the REST API host (e.g. Bearer).
    #[serde(default)]
    pub credential_sources: Option<Vec<CredentialSource>>,
}

#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub canonical_route: String,
    pub upstream: String,
    pub credential_sources: Vec<CredentialSource>,
    pub strip_prefix: Option<String>,
    pub inject_mode: InjectMode,
    pub url_path_prefix: Option<String>,
    pub matcher: RouteMatcher,
}

impl RouteEntry {
    pub fn from_profile(prefix: &str, route: ProfileRoute) -> Self {
        if route.inject_mode == InjectMode::UrlPath && route.url_path_prefix.is_none() {
            warn!(
                "Route '{}' has inject_mode=url_path but no url_path_prefix — credential will be injected without a path prefix",
                prefix
            );
        }
        RouteEntry {
            canonical_route: prefix.to_string(),
            upstream: route.upstream,
            credential_sources: route.credential_sources,
            strip_prefix: None,
            inject_mode: route.inject_mode,
            url_path_prefix: route.url_path_prefix,
            matcher: RouteMatcher::Prefix,
        }
    }

    fn from_alias(canonical_route: &str, route: &RouteEntry, alias: RouteAlias) -> Self {
        RouteEntry {
            canonical_route: canonical_route.to_string(),
            upstream: route.upstream.clone(),
            credential_sources: route.credential_sources.clone(),
            strip_prefix: alias.strip_prefix,
            inject_mode: route.inject_mode.clone(),
            url_path_prefix: route.url_path_prefix.clone(),
            matcher: RouteMatcher::Prefix,
        }
    }

    fn from_git_protocol(parent: &RouteEntry, git: GitProtocolRoute) -> Self {
        let credential_sources = git
            .credential_sources
            .unwrap_or_else(|| parent.credential_sources.clone());
        RouteEntry {
            canonical_route: parent.canonical_route.clone(),
            upstream: git.upstream,
            credential_sources,
            strip_prefix: None,
            inject_mode: InjectMode::Header,
            url_path_prefix: None,
            matcher: RouteMatcher::GitSmartHttp,
        }
    }
}

/// Returns true when `path` is a git smart-HTTP request of the form
/// `/<owner>/<repo>.git/(info/refs|git-upload-pack|git-receive-pack)`.
///
/// Deliberately narrow: dumb-HTTP requests under `.git/objects/...` are
/// rejected, as is any path that doesn't have exactly two segments before
/// the `.git/` suffix.
pub fn is_git_smart_http(path: &str) -> bool {
    let trimmed = path.trim_start_matches('/');
    let Some((repo_part, suffix)) = trimmed.split_once(".git/") else {
        return false;
    };
    if repo_part.split('/').count() != 2 {
        return false;
    }
    if repo_part.split('/').any(|s| s.is_empty()) {
        return false;
    }
    matches!(suffix, "info/refs" | "git-upload-pack" | "git-receive-pack")
}

pub fn load_profile() -> Vec<(String, RouteEntry)> {
    load_embedded_routes()
}

pub fn load_embedded_routes() -> Vec<(String, RouteEntry)> {
    try_load_routes_from_str(EMBEDDED_PROFILE_PATH, EMBEDDED_PROFILE_YAML)
        .expect("embedded profile manifest must be valid")
}

pub fn try_load_routes_from_str(
    source: &str,
    contents: &str,
) -> Result<Vec<(String, RouteEntry)>, String> {
    let profile: Profile = serde_yaml::from_str(contents)
        .map_err(|e| format!("Failed to parse profile at {}: {}", source, e))?;

    let mut original_keys = BTreeSet::new();
    for prefix in profile.routes.keys() {
        let key = normalize_route_key(prefix);
        if !original_keys.insert(key.clone()) {
            return Err(format!(
                "Profile route '{}' normalizes to duplicate key '{}'",
                prefix, key
            ));
        }
    }

    let mut routes = BTreeMap::new();
    for (prefix, route) in profile.routes {
        let key = normalize_route_key(&prefix);
        let aliases = route.aliases.clone();
        let git_protocol = route.git_protocol.clone();
        let entry = RouteEntry::from_profile(&key, route);
        insert_route(&mut routes, key.clone(), entry.clone())?;
        for alias in aliases {
            let alias_key = normalize_route_key(&alias.prefix);
            let alias_entry = RouteEntry::from_alias(&key, &entry, alias);
            insert_route(&mut routes, alias_key, alias_entry)?;
        }
        if let Some(git) = git_protocol {
            let git_key = format!("__git__{}", key);
            let git_entry = RouteEntry::from_git_protocol(&entry, git);
            insert_route(&mut routes, git_key, git_entry)?;
        }
    }

    Ok(routes.into_iter().collect())
}

fn normalize_route_key(key: &str) -> String {
    key.trim_matches('/').to_string()
}

fn insert_route(
    routes: &mut BTreeMap<String, RouteEntry>,
    key: String,
    entry: RouteEntry,
) -> Result<(), String> {
    if routes.insert(key.clone(), entry).is_some() {
        return Err(format!(
            "Profile route or alias normalizes to duplicate key '{}'",
            key
        ));
    }
    Ok(())
}
