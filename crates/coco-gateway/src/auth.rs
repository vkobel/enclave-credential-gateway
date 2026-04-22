//! Phantom token authentication middleware.

use crate::profile::CredentialSource;
use crate::registry::TokenRecord;
use crate::state::AppState;
use crate::{validate_proxy_authorization, validate_bearer_or_raw};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use std::sync::Arc;
use tracing::warn;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct PhantomAuth {
    pub header: String,
    pub preferred_source: Option<usize>,
    pub token_record: Option<TokenRecord>,
}

pub fn extract_candidate_tokens(req: &Request<Body>) -> Vec<String> {
    let mut candidates = Vec::new();

    if let Some(v) = req.headers().get("proxy-authorization") {
        if let Ok(s) = v.to_str() {
            let lower = s.to_lowercase();
            if let Some(rest) = lower.strip_prefix("bearer ") {
                candidates.push(rest.trim().to_string());
            } else if let Some(rest) = lower.strip_prefix("basic ") {
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(rest.trim()) {
                    if let Ok(decoded_str) = std::str::from_utf8(&decoded) {
                        if let Some((_, pw)) = decoded_str.split_once(':') {
                            candidates.push(pw.to_string());
                        }
                    }
                }
            }
        }
    }

    for value in req.headers().values() {
        if let Ok(s) = value.to_str() {
            let lower = s.to_lowercase();
            if let Some(rest) = lower.strip_prefix("bearer ") {
                let candidate = rest.trim().to_string();
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            } else if !s.contains(':') && !lower.starts_with("basic ") && !lower.starts_with("bearer ") {
                let candidate = s.trim().to_string();
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
    }

    candidates
}

pub fn find_phantom_auth(
    req: &Request<Body>,
    token: &Zeroizing<String>,
    sources: &[CredentialSource],
) -> Option<PhantomAuth> {
    if let Some(v) = req.headers().get("proxy-authorization") {
        if validate_proxy_authorization(v.as_bytes(), token) {
            return Some(PhantomAuth {
                header: "proxy-authorization".to_string(),
                preferred_source: None,
                token_record: None,
            });
        }
    }

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

    None
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let prefix = path.trim_start_matches('/').split('/').next().unwrap_or("");
    let sources: &[CredentialSource] = state
        .routes
        .iter()
        .find(|(p, _)| p == prefix)
        .map(|(_, entry)| entry.credential_sources.as_slice())
        .unwrap_or(&[]);

    // 1. Try registry tokens first
    if let Some(registry) = &state.token_registry {
        let candidates = extract_candidate_tokens(&req);
        for candidate in candidates {
            if let Some(record) = registry.validate(&candidate).await {
                if !record.scope.is_empty() && !record.scope.iter().any(|s| s == prefix) {
                    return (StatusCode::FORBIDDEN, "403 Forbidden — token scope denied").into_response();
                }
                let auth = PhantomAuth {
                    header: "proxy-authorization".to_string(),
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
            req.extensions_mut().insert(auth);
            return next.run(req).await;
        }
    }

    warn!("Invalid or missing phantom token");
    (StatusCode::PROXY_AUTHENTICATION_REQUIRED, "407 Proxy Authentication Required").into_response()
}