//! Profile loading and route definitions.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

const EMBEDDED_PROFILE_PATH: &str = "profiles/routes.json";
const EMBEDDED_PROFILE_JSON: &str = include_str!("../../../profiles/routes.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialSource {
    pub env: String,
    pub inject_header: String,
    #[serde(default = "default_credential_format")]
    pub format: String,
    #[serde(default)]
    pub prefix: Option<String>,
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

#[derive(Debug, Deserialize)]
pub(crate) struct Profile {
    pub(crate) routes: BTreeMap<String, ProfileRoute>,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteAlias {
    pub prefix: String,
    #[serde(default)]
    pub strip_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub canonical_route: String,
    pub upstream: String,
    pub credential_sources: Vec<CredentialSource>,
    pub strip_prefix: Option<String>,
    pub inject_mode: InjectMode,
    pub url_path_prefix: Option<String>,
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
        }
    }
}

pub fn load_profile() -> Vec<(String, RouteEntry)> {
    load_embedded_routes()
}

pub fn load_embedded_routes() -> Vec<(String, RouteEntry)> {
    try_load_routes_from_str(EMBEDDED_PROFILE_PATH, EMBEDDED_PROFILE_JSON)
        .expect("embedded profile manifest must be valid")
}

pub fn try_load_routes_from_str(
    source: &str,
    contents: &str,
) -> Result<Vec<(String, RouteEntry)>, String> {
    let profile: Profile = serde_json::from_str(contents)
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
        let entry = RouteEntry::from_profile(&key, route);
        insert_route(&mut routes, key.clone(), entry.clone())?;
        for alias in aliases {
            let alias_key = normalize_route_key(&alias.prefix);
            let alias_entry = RouteEntry::from_alias(&key, &entry, alias);
            insert_route(&mut routes, alias_key, alias_entry)?;
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
