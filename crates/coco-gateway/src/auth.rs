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
use std::sync::Arc;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct PhantomAuth {
    pub header: String,
    pub preferred_source: Option<usize>,
    pub token_record: Option<TokenRecord>,
}

pub fn extract_candidate_tokens(req: &Request<Body>) -> Vec<String> {
    let mut candidates = Vec::new();
    for value in req.headers().values() {
        if let Ok(s) = value.to_str() {
            let lower = s.to_lowercase();
            let candidate = if let Some(rest) = lower.strip_prefix("bearer ") {
                Some(rest.trim().to_string())
            } else {
                // `gh` CLI sends `Authorization: token <value>` (GitHub legacy format)
                lower.strip_prefix("token ").map(|rest| rest.trim().to_string())
            };
            if let Some(c) = candidate {
                if !candidates.contains(&c) {
                    candidates.push(c);
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
    let prefix = path.trim_start_matches('/').split('/').next().unwrap_or("");
    let canonical = state.canonical_route_key(prefix);
    let sources: &[CredentialSource] = state
        .route_entry(prefix)
        .map(|entry| entry.credential_sources.as_slice())
        .unwrap_or(&[]);

    // 1. Try registry tokens first
    if let Some(registry) = &state.token_registry {
        let candidates = extract_candidate_tokens(&req);
        for candidate in candidates {
            if let Some(record) = registry.validate(&candidate).await {
                if !record.scope.is_empty() && !record.scope.iter().any(|s| s == canonical) {
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

    warn!("{} {} — 407 missing or invalid token", method, path);
    (
        StatusCode::PROXY_AUTHENTICATION_REQUIRED,
        "407 Proxy Authentication Required",
    )
        .into_response()
}
