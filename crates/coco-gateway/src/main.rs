//! CoCo Credential Gateway — Phase 1a (plain infrastructure)
//!
//! Phantom-token reverse proxy: agents authenticate with COCO_PHANTOM_TOKEN,
//! the gateway validates it, strips it, injects the real upstream credential,
//! and forwards the request via TLS.
//!
//! Routes are loaded from a JSON profile file at startup:
//!   1. $COCO_PROFILE (env var)
//!   2. /etc/coco/profile.json (default path)
//!   3. Built-in defaults (openai / anthropic / github / httpbin)
//!
//! ## Phantom token auth
//!
//! The client sends the phantom token in the SAME HEADER it would use for a real
//! credential. For example, Claude Code with `ANTHROPIC_API_KEY=<phantom>` sends
//! `x-api-key: <phantom>`; the gateway validates and replaces with the real key.
//!
//! Accepted locations (checked in order):
//!   1. Proxy-Authorization: Bearer <token>   — universal fallback / existing scripts
//!   2. Each route's configured `inject_header` containing the phantom token
//!
//! ## Claude Code local flow
//!
//!   # Gateway side (has the real Anthropic API key)
//!   export COCO_PHANTOM_TOKEN=my-secret
//!   export ANTHROPIC_API_KEY=sk-ant-...
//!   docker compose up -d
//!
//!   # Claude Code side (no real credentials)
//!   export ANTHROPIC_BASE_URL=http://localhost:8080/anthropic
//!   export ANTHROPIC_API_KEY=my-secret   # phantom token as the "API key"
//!   claude chat                          # gateway injects real cred server-side

use coco_gateway::{validate_proxy_authorization, validate_bearer_or_raw};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderName, HeaderValue, Request, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use http_body_util::BodyExt;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use rustls;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// Credential source — one entry in a route's ordered fallback list
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialSource {
    pub env: String,
    pub inject_header: String,
    #[serde(default = "default_credential_format")]
    pub format: String,
    /// Optional value prefix; this source only matches if the env value starts with it.
    /// Lets one env var (e.g. ANTHROPIC_API_KEY) route to different headers based on
    /// token shape (sk-ant-oat... → Authorization: Bearer; sk-ant-api... → x-api-key).
    #[serde(default)]
    pub prefix: Option<String>,
}

fn default_inject_header() -> String {
    "Authorization".to_string()
}

fn default_credential_format() -> String {
    "Bearer {}".to_string()
}

// ---------------------------------------------------------------------------
// Profile schema
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Profile {
    routes: std::collections::HashMap<String, ProfileRoute>,
}

/// A route as it appears in profile.json.
/// Two forms (backwards-compatible):
///   - Legacy: `credential_env` + `inject_header` + `credential_format`
///   - Multi-source: `credential_sources` list (takes precedence when present)
#[derive(Debug, Deserialize)]
struct ProfileRoute {
    upstream: String,
    #[serde(default)]
    credential_env: Option<String>,
    #[serde(default = "default_inject_header")]
    inject_header: String,
    #[serde(default = "default_credential_format")]
    credential_format: String,
    #[serde(default)]
    credential_sources: Vec<CredentialSource>,
}

// ---------------------------------------------------------------------------
// Runtime route entry
// ---------------------------------------------------------------------------

pub struct RouteEntry {
    upstream: String,
    /// Ordered credential sources; first available env var wins at inject time.
    credential_sources: Vec<CredentialSource>,
}

impl RouteEntry {
    fn from_profile(route: ProfileRoute) -> Self {
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
        RouteEntry { upstream: route.upstream, credential_sources: sources }
    }
}

struct AppState {
    phantom_token: Zeroizing<String>,
    routes: Vec<(String, RouteEntry)>,
    https_client:
        Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, Body>,
}

// ---------------------------------------------------------------------------
// Profile loading
// ---------------------------------------------------------------------------

fn load_profile() -> (Vec<(String, RouteEntry)>, Option<String>) {
    let path = std::env::var("COCO_PROFILE")
        .ok()
        .unwrap_or_else(|| "/etc/coco/profile.json".to_string());

    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (builtin_routes(), None);
        }
        Err(e) => {
            error!("Failed to read profile at {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let profile: Profile = match serde_json::from_str(&contents) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to parse profile at {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let routes = profile
        .routes
        .into_iter()
        .map(|(prefix, r)| (prefix.trim_matches('/').to_string(), RouteEntry::from_profile(r)))
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
            },
        ),
    ]
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let phantom_token = match std::env::var("COCO_PHANTOM_TOKEN") {
        Ok(t) if !t.is_empty() => Zeroizing::new(t),
        Ok(_) => {
            error!("COCO_PHANTOM_TOKEN is set but empty — refusing to start");
            std::process::exit(1);
        }
        Err(_) => {
            error!("COCO_PHANTOM_TOKEN is not set — refusing to start");
            std::process::exit(1);
        }
    };

    let port: u16 = std::env::var("COCO_LISTEN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    let (routes, profile_path) = load_profile();
    match profile_path {
        Some(p) => info!("Loaded {} route(s) from profile at {}", routes.len(), p),
        None => info!("No profile found, using built-in defaults ({} routes)", routes.len()),
    }

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();

    let https_client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector);

    let state = Arc::new(AppState { phantom_token, routes, https_client });

    let app = Router::new()
        .fallback(proxy_handler)
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        error!("Failed to bind to {}: {}", addr, e);
        std::process::exit(1);
    });

    info!("coco-gateway listening on {}", addr);
    axum::serve(listener, app).await.expect("server error");
}

// ---------------------------------------------------------------------------
// Phantom token auth
// ---------------------------------------------------------------------------

/// Result of phantom token authentication.
///
/// Records which header carried the phantom token and, if it came from a
/// route's credential source header, which source index matched. Both are
/// used by the proxy handler to strip the right header and prefer the right
/// credential source for injection.
#[derive(Clone)]
struct PhantomAuth {
    /// The header name to strip from the forwarded request.
    header: String,
    /// If the phantom came from a credential source header, its index.
    /// None means it came from Proxy-Authorization → use first available source.
    preferred_source: Option<usize>,
}

/// Find and validate the phantom token from the incoming request.
///
/// Checks in order:
/// 1. `Proxy-Authorization: Bearer <token>` — universal fallback
/// 2. Each credential source's `inject_header` containing `<token>` or `Bearer <token>`
///
/// Returns `Some(PhantomAuth)` on success, `None` if no valid token found.
fn find_phantom_auth(
    req: &Request<Body>,
    token: &Zeroizing<String>,
    sources: &[CredentialSource],
) -> Option<PhantomAuth> {
    // 1. Proxy-Authorization (universal fallback — keeps test scripts working)
    if let Some(v) = req.headers().get("proxy-authorization") {
        if validate_proxy_authorization(v.as_bytes(), token) {
            return Some(PhantomAuth {
                header: "proxy-authorization".to_string(),
                preferred_source: None,
            });
        }
    }

    // 2. Each credential source's inject_header
    for (i, src) in sources.iter().enumerate() {
        let header_lower = src.inject_header.to_lowercase();
        if let Some(v) = req.headers().get(header_lower.as_str()) {
            if validate_bearer_or_raw(v.as_bytes(), token) {
                return Some(PhantomAuth {
                    header: header_lower,
                    preferred_source: Some(i),
                });
            }
        }
    }

    None
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    // Extract path prefix to look up route's credential sources
    let path = req.uri().path();
    let prefix = path.trim_start_matches('/').split('/').next().unwrap_or("");
    let sources: &[CredentialSource] = state
        .routes
        .iter()
        .find(|(p, _)| p == prefix)
        .map(|(_, entry)| entry.credential_sources.as_slice())
        .unwrap_or(&[]);

    match find_phantom_auth(&req, &state.phantom_token, sources) {
        Some(auth) => {
            req.extensions_mut().insert(auth);
            next.run(req).await
        }
        None => {
            warn!("Invalid or missing phantom token");
            (StatusCode::PROXY_AUTHENTICATION_REQUIRED, "407 Proxy Authentication Required")
                .into_response()
        }
    }
}


// ---------------------------------------------------------------------------
// Proxy handler
// ---------------------------------------------------------------------------

async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let path = req.uri().path();

    // Extract prefix
    let stripped = path.trim_start_matches('/');
    let prefix = stripped.split('/').next().unwrap_or("");

    // Route lookup — 404 for unknown prefix
    let (_, entry) = match state.routes.iter().find(|(p, _)| p == prefix) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    };

    // Retrieve phantom auth result set by middleware
    let phantom_auth = req.extensions().get::<PhantomAuth>().cloned().unwrap_or(PhantomAuth {
        header: "proxy-authorization".to_string(),
        preferred_source: None,
    });

    // Resolve credential: prefer the source whose header matched the phantom,
    // then fall through to first available source.
    let resolved = resolve_credential(&entry.credential_sources, phantom_auth.preferred_source);
    let (src, credential) = match resolved {
        Some(r) => r,
        None => {
            warn!("No credential available for route '{}'", prefix);
            return (StatusCode::SERVICE_UNAVAILABLE, "503 Service Unavailable").into_response();
        }
    };

    // Build upstream path
    let upstream_path = &path[prefix.len() + 1..];
    let upstream_path = if upstream_path.is_empty() { "/" } else { upstream_path };

    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("{}{}{}", entry.upstream, upstream_path, query);
    let upstream_uri: Uri = match upstream_url.parse() {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to parse upstream URI {}: {}", upstream_url, e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    // Mutate headers: strip phantom auth header(s), set upstream host, inject credential
    let method = req.method().clone();
    let mut headers = req.headers().clone();

    headers.remove(phantom_auth.header.as_str());
    headers.remove("proxy-authorization"); // always clean up
    headers.remove("host");

    let inject_value = src.format.replace("{}", &credential);
    if let Ok(header_name) = HeaderName::from_bytes(src.inject_header.as_bytes()) {
        headers.insert(
            header_name,
            HeaderValue::from_str(&inject_value).unwrap_or_else(|_| HeaderValue::from_static("")),
        );
    }

    let upstream_host = upstream_uri.host().unwrap_or("");
    headers.insert(
        "host",
        HeaderValue::from_str(upstream_host).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    // Collect body
    let body_bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    // Build and send upstream request
    let mut upstream_req = Request::builder().method(method).uri(upstream_uri);
    for (k, v) in &headers {
        upstream_req = upstream_req.header(k, v);
    }
    let upstream_req = match upstream_req.body(Body::from(body_bytes)) {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to build upstream request: {}", e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    match state.https_client.request(upstream_req).await {
        Ok(resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let body = Body::new(
                resp.into_body()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
            );
            let mut response = Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = resp_headers;
            response
        }
        Err(e) => {
            error!("Upstream request failed: {}", e);
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Resolve a credential from the sources list.
///
/// If `preferred` is Some(i), try source[i] first (its env var was the one
/// that carried the phantom token). Falls through to the first available source.
fn resolve_credential(
    sources: &[CredentialSource],
    preferred: Option<usize>,
) -> Option<(&CredentialSource, String)> {
    let matches = |src: &CredentialSource, v: &str| -> bool {
        !v.is_empty() && src.prefix.as_deref().is_none_or(|p| v.starts_with(p))
    };

    // Try preferred source first
    if let Some(i) = preferred {
        if let Some(src) = sources.get(i) {
            if let Ok(v) = std::env::var(&src.env) {
                if matches(src, &v) {
                    return Some((src, v));
                }
            }
        }
    }
    // Fall through to first matching source
    sources.iter().find_map(|src| {
        std::env::var(&src.env).ok().filter(|v| matches(src, v)).map(|v| (src, v))
    })
}
