//! Profile loading and route definitions.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
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

pub(crate) fn default_inject_header() -> String {
    "Authorization".to_string()
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
    QueryParam,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Profile {
    pub(crate) routes: HashMap<String, ProfileRoute>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileRoute {
    #[serde(default)]
    pub canonical: Option<String>,
    pub upstream: String,
    #[serde(default)]
    pub credential_env: Option<String>,
    #[serde(default = "default_inject_header")]
    pub inject_header: String,
    #[serde(default = "default_credential_format")]
    pub credential_format: String,
    #[serde(default)]
    pub credential_sources: Vec<CredentialSource>,
    #[serde(default)]
    pub strip_prefix: Option<String>,
    #[serde(default)]
    pub inject_mode: InjectMode,
    #[serde(default)]
    pub url_path_prefix: Option<String>,
    #[serde(default)]
    pub inject_param: Option<String>,
    /// Alternate path prefix that also routes here (e.g. "api" for the github route,
    /// since `gh` CLI sends /api/v3/... when GH_HOST is set).
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub canonical_route: String,
    pub upstream: String,
    pub credential_sources: Vec<CredentialSource>,
    pub strip_prefix: Option<String>,
    pub inject_mode: InjectMode,
    pub url_path_prefix: Option<String>,
    pub inject_param: Option<String>,
}

impl RouteEntry {
    pub fn from_profile(prefix: &str, route: ProfileRoute) -> Self {
        if route.inject_mode == InjectMode::UrlPath && route.url_path_prefix.is_none() {
            warn!(
                "Route '{}' has inject_mode=url_path but no url_path_prefix — credential will be injected without a path prefix",
                prefix
            );
        }
        let sources = if !route.credential_sources.is_empty() {
            route.credential_sources
        } else if let Some(env) = route.credential_env {
            vec![CredentialSource {
                env,
                inject_header: route.inject_header,
                format: route.credential_format,
                prefix: None,
            }]
        } else {
            vec![]
        };
        RouteEntry {
            canonical_route: route.canonical.unwrap_or_else(|| prefix.to_string()),
            upstream: route.upstream,
            credential_sources: sources,
            strip_prefix: route.strip_prefix,
            inject_mode: route.inject_mode,
            url_path_prefix: route.url_path_prefix,
            inject_param: route.inject_param,
        }
    }
}

pub fn load_profile() -> (Vec<(String, RouteEntry)>, Option<String>) {
    if let Ok(path) = std::env::var("COCO_PROFILE") {
        let routes = load_profile_from_path(&path);
        return (routes, Some(path));
    }

    let legacy_path = "/etc/coco/profile.json";
    if Path::new(legacy_path).exists() {
        let routes = load_profile_from_path(legacy_path);
        return (routes, Some(legacy_path.to_string()));
    }

    (
        load_embedded_routes(),
        Some(format!("embedded manifest {}", EMBEDDED_PROFILE_PATH)),
    )
}

pub fn load_embedded_routes() -> Vec<(String, RouteEntry)> {
    load_routes_from_str(EMBEDDED_PROFILE_PATH, EMBEDDED_PROFILE_JSON)
}

fn load_profile_from_path(path: &str) -> Vec<(String, RouteEntry)> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to read profile at {}: {}", path, e);
            std::process::exit(1);
        }
    };

    load_routes_from_str(path, &contents)
}

fn load_routes_from_str(source: &str, contents: &str) -> Vec<(String, RouteEntry)> {
    let profile: Profile = match serde_json::from_str(contents) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to parse profile at {}: {}", source, e);
            std::process::exit(1);
        }
    };

    let mut routes = BTreeMap::new();
    for (prefix, route) in profile.routes {
        let key = prefix.trim_matches('/').to_string();
        let alias = route.alias.clone().map(|a| a.trim_matches('/').to_string());
        let entry = RouteEntry::from_profile(&key, route);
        routes.insert(key.clone(), entry.clone());

        if let Some(alias_key) = alias {
            if !alias_key.is_empty() && alias_key != key {
                routes.entry(alias_key).or_insert(entry.clone());
            }
        }
    }

    routes.into_iter().collect()
}
