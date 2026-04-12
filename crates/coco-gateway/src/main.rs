//! CoCo Credential Gateway — Phase 1a (plain infrastructure)
//!
//! Phantom-token reverse proxy: agents authenticate with COCO_PHANTOM_TOKEN,
//! the gateway validates it, strips it, injects the real upstream credential,
//! and forwards the request via TLS.
//!
//! Routes are loaded from a JSON profile file at startup:
//!   1. $COCO_PROFILE (env var)
//!   2. /etc/coco/profile.json (default path)
//!   3. Built-in defaults (openai / anthropic / github)

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
use nono_proxy::token::constant_time_eq;
use rustls;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// Profile schema (tasks 1.1)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Profile {
    routes: std::collections::HashMap<String, ProfileRoute>,
}

#[derive(Debug, Deserialize)]
struct ProfileRoute {
    upstream: String,
    credential_env: String,
    #[serde(default = "default_inject_header")]
    inject_header: String,
    #[serde(default = "default_credential_format")]
    credential_format: String,
}

fn default_inject_header() -> String {
    "Authorization".to_string()
}

fn default_credential_format() -> String {
    "Bearer {}".to_string()
}

// ---------------------------------------------------------------------------
// Runtime route entry (task 1.2)
// ---------------------------------------------------------------------------

struct RouteEntry {
    upstream: String,
    credential_env: String,
    inject_header: String,
    credential_format: String,
}

struct AppState {
    phantom_token: Zeroizing<String>,
    routes: Vec<(String, RouteEntry)>,
    https_client:
        Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, Body>,
}

// ---------------------------------------------------------------------------
// Profile loading (task 2.1)
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

    let routes: Vec<(String, RouteEntry)> = profile
        .routes
        .into_iter()
        .map(|(prefix, r)| {
            (
                prefix.trim_matches('/').to_string(),
                RouteEntry {
                    upstream: r.upstream,
                    credential_env: r.credential_env,
                    inject_header: r.inject_header,
                    credential_format: r.credential_format,
                },
            )
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
                credential_env: "OPENAI_API_KEY".to_string(),
                inject_header: "Authorization".to_string(),
                credential_format: "Bearer {}".to_string(),
            },
        ),
        (
            "anthropic".to_string(),
            RouteEntry {
                upstream: "https://api.anthropic.com".to_string(),
                credential_env: "ANTHROPIC_API_KEY".to_string(),
                inject_header: "x-api-key".to_string(),
                credential_format: "{}".to_string(),
            },
        ),
        (
            "github".to_string(),
            RouteEntry {
                upstream: "https://api.github.com".to_string(),
                credential_env: "GITHUB_TOKEN".to_string(),
                inject_header: "Authorization".to_string(),
                credential_format: "Bearer {}".to_string(),
            },
        ),
    ]
}

// ---------------------------------------------------------------------------
// Startup (task 2.2)
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Install the ring crypto provider for rustls (required before any TLS operations)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Required: phantom token
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

    // Optional: listen port (default 8080)
    let port: u16 = std::env::var("COCO_LISTEN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    // Load routes from profile or built-ins
    let (routes, profile_path) = load_profile();
    match profile_path {
        Some(p) => info!("Loaded {} route(s) from profile at {}", routes.len(), p),
        None => info!("No profile found, using built-in defaults ({} routes)", routes.len()),
    }

    // Build HTTPS client with webpki roots
    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();

    let https_client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector);

    let state = Arc::new(AppState {
        phantom_token,
        routes,
        https_client,
    });

    // Wire router: middleware → handler for all paths
    let app = Router::new()
        .fallback(proxy_handler)
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    // Bind and start
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        });

    info!("coco-gateway listening on {}", addr);
    axum::serve(listener, app).await.expect("server error");
}

// ---------------------------------------------------------------------------
// Phantom token validation middleware
// ---------------------------------------------------------------------------

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let proxy_auth = req
        .headers()
        .get("proxy-authorization")
        .map(|v| v.as_bytes().to_owned());

    let valid = match &proxy_auth {
        None => false,
        Some(bytes) => validate_proxy_authorization(bytes, &state.phantom_token),
    };

    if !valid {
        warn!("Invalid or missing Proxy-Authorization");
        return (
            StatusCode::PROXY_AUTHENTICATION_REQUIRED,
            "407 Proxy Authentication Required",
        )
            .into_response();
    }

    next.run(req).await
}

fn validate_proxy_authorization(header_bytes: &[u8], token: &Zeroizing<String>) -> bool {
    let Ok(header_str) = std::str::from_utf8(header_bytes) else {
        return false;
    };

    let lower = header_str.to_lowercase();

    if let Some(rest) = lower.strip_prefix("bearer ") {
        let candidate = &header_str[header_str.len() - rest.len()..];
        return constant_time_eq(candidate.trim().as_bytes(), token.as_bytes());
    }

    if let Some(rest) = lower.strip_prefix("basic ") {
        let encoded = &header_str[header_str.len() - rest.len()..];
        return validate_basic_auth(encoded.trim(), token);
    }

    false
}

fn validate_basic_auth(encoded: &str, token: &Zeroizing<String>) -> bool {
    use base64::Engine;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded_str) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let password = match decoded_str.split_once(':') {
        Some((_, pw)) => pw,
        None => return false,
    };
    constant_time_eq(password.as_bytes(), token.as_bytes())
}

// ---------------------------------------------------------------------------
// Proxy handler (task 3.1 — per-route inject_header + credential_format)
// ---------------------------------------------------------------------------

async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let path = req.uri().path();

    // Extract prefix (first path segment after leading slash)
    let stripped = path.trim_start_matches('/');
    let prefix = stripped.split('/').next().unwrap_or("");

    // Look up route — 404 for unknown prefix
    let route_entry = state.routes.iter().find(|(p, _)| p == prefix);
    let (_, entry) = match route_entry {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    };

    // Resolve credential — 503 if absent
    let credential = match std::env::var(&entry.credential_env) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            warn!("Upstream credential {} not set", entry.credential_env);
            return (StatusCode::SERVICE_UNAVAILABLE, "503 Service Unavailable").into_response();
        }
    };

    // Build upstream path: strip /<prefix>
    let upstream_path = &path[prefix.len() + 1..];
    let upstream_path = if upstream_path.is_empty() { "/" } else { upstream_path };

    // Build upstream URI
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("{}{}{}", entry.upstream, upstream_path, query);
    let upstream_uri: Uri = match upstream_url.parse() {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to parse upstream URI {}: {}", upstream_url, e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    // Build credential header value using per-route format
    let credential_value = entry.credential_format.replace("{}", &credential);

    // Clone and mutate headers: strip Proxy-Authorization, inject credential header
    let method = req.method().clone();
    let mut headers = req.headers().clone();
    headers.remove("proxy-authorization");
    headers.remove("host");
    if let Ok(header_name) = HeaderName::from_bytes(entry.inject_header.as_bytes()) {
        headers.insert(
            header_name,
            HeaderValue::from_str(&credential_value)
                .unwrap_or_else(|_| HeaderValue::from_static("")),
        );
    }

    // Set upstream Host
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

    // Forward and stream response
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
