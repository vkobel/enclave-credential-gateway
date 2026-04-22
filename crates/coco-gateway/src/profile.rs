//! Profile loading and route definitions.

use serde::Deserialize;
use std::collections::HashMap;
use tracing::warn;

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
}

pub struct RouteEntry {
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
    let path = std::env::var("COCO_PROFILE")
        .ok()
        .unwrap_or_else(|| "/etc/coco/profile.json".to_string());

    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (builtin_routes(), None);
        }
        Err(e) => {
            tracing::error!("Failed to read profile at {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let profile: Profile = match serde_json::from_str(&contents) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to parse profile at {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let routes = profile
        .routes
        .into_iter()
        .map(|(prefix, r)| {
            let key = prefix.trim_matches('/').to_string();
            let entry = RouteEntry::from_profile(&key, r);
            (key, entry)
        })
        .collect();

    (routes, Some(path))
}

fn builtin_routes() -> Vec<(String, RouteEntry)> {
    vec![
        (
            "openai".to_string(),
            RouteEntry {
                upstream: "https://api.openai.com".to_string(),
                credential_sources: vec![CredentialSource {
                    env: "OPENAI_API_KEY".to_string(),
                    inject_header: "Authorization".to_string(),
                    format: "Bearer {}".to_string(),
                    prefix: None,
                }],
                strip_prefix: None,
                inject_mode: InjectMode::Header,
                url_path_prefix: None,
                inject_param: None,
            },
        ),
        (
            "anthropic".to_string(),
            RouteEntry {
                upstream: "https://api.anthropic.com".to_string(),
                credential_sources: vec![
                    CredentialSource {
                        env: "ANTHROPIC_API_KEY".to_string(),
                        inject_header: "Authorization".to_string(),
                        format: "Bearer {}".to_string(),
                        prefix: Some("sk-ant-oat".to_string()),
                    },
                    CredentialSource {
                        env: "ANTHROPIC_API_KEY".to_string(),
                        inject_header: "x-api-key".to_string(),
                        format: "{}".to_string(),
                        prefix: None,
                    },
                ],
                strip_prefix: None,
                inject_mode: InjectMode::Header,
                url_path_prefix: None,
                inject_param: None,
            },
        ),
        (
            "github".to_string(),
            RouteEntry {
                upstream: "https://api.github.com".to_string(),
                credential_sources: vec![CredentialSource {
                    env: "GITHUB_TOKEN".to_string(),
                    inject_header: "Authorization".to_string(),
                    format: "Bearer {}".to_string(),
                    prefix: None,
                }],
                strip_prefix: None,
                inject_mode: InjectMode::Header,
                url_path_prefix: None,
                inject_param: None,
            },
        ),
        (
            "httpbin".to_string(),
            RouteEntry {
                upstream: "https://httpbin.org".to_string(),
                credential_sources: vec![CredentialSource {
                    env: "HTTPBIN_TOKEN".to_string(),
                    inject_header: "Authorization".to_string(),
                    format: "Bearer {}".to_string(),
                    prefix: None,
                }],
                strip_prefix: None,
                inject_mode: InjectMode::Header,
                url_path_prefix: None,
                inject_param: None,
            },
        ),
    ]
}