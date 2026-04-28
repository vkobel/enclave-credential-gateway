//! Phantom token authentication middleware.

use crate::profile::CredentialSource;
use crate::registry::TokenRecord;
use crate::state::AppState;
use crate::validate_bearer_or_raw;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::sync::Arc;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct PhantomAuth {
    pub header: String,
    pub preferred_source: Option<usize>,
    pub token_record: Option<TokenRecord>,
}

/// Splits an `Authorization` header value into `(scheme_lowercased, rest_original_case)`.
///
/// Lowercases only the scheme prefix for comparison so that the returned token
/// preserves its original case — important for tokens with mixed-case bytes.
fn parse_auth_scheme(value: &str) -> Option<(&'static str, &str)> {
    const SCHEMES: &[&str] = &["bearer ", "token ", "basic "];
    let bytes = value.as_bytes();
    for scheme in SCHEMES {
        if bytes.len() >= scheme.len()
            && bytes[..scheme.len()].eq_ignore_ascii_case(scheme.as_bytes())
        {
            return Some((scheme.trim_end(), value[scheme.len()..].trim()));
        }
    }
    None
}

pub fn extract_candidate_tokens(req: &Request<Body>) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    let push = |c: String, list: &mut Vec<String>| {
        if !c.is_empty() && !list.contains(&c) {
            list.push(c);
        }
    };
    for value in req.headers().values() {
        let Ok(s) = value.to_str() else { continue };
        let Some((scheme, rest)) = parse_auth_scheme(s) else {
            continue;
        };
        match scheme {
            "bearer" | "token" => push(rest.to_string(), &mut candidates),
            "basic" => {
                let Ok(decoded) = BASE64_STANDARD.decode(rest) else {
                    continue;
                };
                let Ok(text) = std::str::from_utf8(&decoded) else {
                    continue;
                };
                // git/gh credential helpers vary on which slot holds the token
                // (`x-access-token:<tok>`, `<tok>:x-oauth-basic`, `oauth2:<tok>`,
                // …). Try both halves; non-token values just miss the registry.
                if let Some((u, p)) = text.split_once(':') {
                    push(u.to_string(), &mut candidates);
                    push(p.to_string(), &mut candidates);
                } else {
                    push(text.to_string(), &mut candidates);
                }
            }
            _ => {}
        }
    }
    candidates
}

pub fn find_phantom_auth(
    req: &Request<Body>,
    token: &Zeroizing<String>,
    sources: &[CredentialSource],
) -> Option<PhantomAuth> {
    for (i, src) in sources.iter().enumerate() {
        let header_lower = src.inject_header.to_lowercase();
        if let Some(v) = req.headers().get(header_lower.as_str()) {
            if validate_bearer_or_raw(v.as_bytes(), token) {
                return Some(PhantomAuth {
                    header: header_lower,
                    preferred_source: Some(i),
                    token_record: None,
                });
            }
        }
    }

    if let Some(v) = req.headers().get("authorization") {
        if validate_bearer_or_raw(v.as_bytes(), token) {
            return Some(PhantomAuth {
                header: "authorization".to_string(),
                preferred_source: None,
                token_record: None,
            });
        }
    }

    None
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let resolved = state.resolve(&path);
    let canonical = resolved
        .as_ref()
        .map(|r| r.entry.canonical_route.as_str())
        .unwrap_or_else(|| path.trim_start_matches('/').split('/').next().unwrap_or(""));
    let sources: &[CredentialSource] = resolved
        .as_ref()
        .map(|r| r.entry.credential_sources.as_slice())
        .unwrap_or(&[]);

    // 1. Try registry tokens first
    if let Some(registry) = &state.token_registry {
        let candidates = extract_candidate_tokens(&req);
        for candidate in candidates {
            if let Some(record) = registry.validate(&candidate).await {
                if !record.allows_route(canonical) {
                    warn!(
                        "{} {} — 403 token '{}' not scoped for route '{}'",
                        method, path, record.name, canonical
                    );
                    return (StatusCode::FORBIDDEN, "403 Forbidden — token scope denied")
                        .into_response();
                }
                info!("{} {} — auth ok (token: '{}')", method, path, record.name);
                let auth = PhantomAuth {
                    header: "authorization".to_string(),
                    preferred_source: None,
                    token_record: Some(record),
                };
                req.extensions_mut().insert(auth);
                return next.run(req).await;
            }
        }
    }

    // 2. Fallback to COCO_PHANTOM_TOKEN
    if let Some(ref phantom) = state.phantom_token {
        if let Some(auth) = find_phantom_auth(&req, phantom, sources) {
            info!("{} {} — auth ok (phantom token)", method, path);
            req.extensions_mut().insert(auth);
            return next.run(req).await;
        }
    }

    // Git smart-HTTP uses 401 + WWW-Authenticate to challenge credentials;
    // 407 is treated as a proxy error and git does not retry with credentials.
    if crate::profile::is_git_smart_http(&path) {
        warn!("{} {} — 401 missing or invalid token (git smart-HTTP)", method, path);
        return (
            StatusCode::UNAUTHORIZED,
            [(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static(r#"Basic realm="coco-gateway""#),
            )],
            "401 Unauthorized",
        )
            .into_response();
    }

    warn!("{} {} — 407 missing or invalid token", method, path);
    (
        StatusCode::PROXY_AUTHENTICATION_REQUIRED,
        "407 Proxy Authentication Required",
    )
        .into_response()
}
