//! Phantom token authentication middleware.

use crate::profile::CredentialSource;
use crate::registry::TokenRecord;
use crate::registry::TokenRegistry;
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

fn push_unique(candidate: String, candidates: &mut Vec<String>) {
    if !candidate.is_empty() && !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn header_candidate_tokens(value: &str, include_raw: bool) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some((scheme, rest)) = parse_auth_scheme(value) {
        match scheme {
            "bearer" | "token" => push_unique(rest.to_string(), &mut candidates),
            "basic" => {
                let Ok(decoded) = BASE64_STANDARD.decode(rest) else {
                    return candidates;
                };
                let Ok(text) = std::str::from_utf8(&decoded) else {
                    return candidates;
                };
                // git/gh credential helpers vary on which slot holds the token
                // (`x-access-token:<tok>`, `<tok>:x-oauth-basic`, `oauth2:<tok>`,
                // ...). Try both halves; non-token values just miss the registry.
                if let Some((u, p)) = text.split_once(':') {
                    push_unique(u.to_string(), &mut candidates);
                    push_unique(p.to_string(), &mut candidates);
                } else {
                    push_unique(text.to_string(), &mut candidates);
                }
            }
            _ => {}
        }
    } else if include_raw {
        push_unique(value.trim().to_string(), &mut candidates);
    }
    candidates
}

pub fn extract_candidate_tokens(req: &Request<Body>) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for value in req.headers().values() {
        let Ok(s) = value.to_str() else { continue };
        for candidate in header_candidate_tokens(s, false) {
            push_unique(candidate, &mut candidates);
        }
    }
    candidates
}

async fn find_registry_auth(
    registry: &TokenRegistry,
    candidates: Vec<(String, String, Option<usize>)>,
) -> Option<PhantomAuth> {
    for (candidate, header, preferred_source) in candidates {
        if let Some(record) = registry.validate(&candidate).await {
            return Some(PhantomAuth {
                header,
                preferred_source,
                token_record: Some(record),
            });
        }
    }

    None
}

fn registry_auth_candidates(
    req: &Request<Body>,
    sources: &[CredentialSource],
) -> Vec<(String, String, Option<usize>)> {
    let mut candidates = Vec::new();
    for src in sources {
        let header_lower = src.inject_header.to_lowercase();
        let Some(v) = req.headers().get(header_lower.as_str()) else {
            continue;
        };
        let Ok(s) = v.to_str() else { continue };
        for candidate in header_candidate_tokens(s, true) {
            candidates.push((candidate, header_lower.clone(), None));
        }
    }

    for candidate in extract_candidate_tokens(req) {
        candidates.push((candidate, "authorization".to_string(), None));
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
        let candidates = registry_auth_candidates(&req, sources);
        if let Some(auth) = find_registry_auth(registry, candidates).await {
            let record = auth
                .token_record
                .clone()
                .expect("registry auth includes token record");
            if !record.allows_route(canonical) {
                warn!(
                    "{} {} — 403 token '{}' not scoped for route '{}'",
                    method, path, record.name, canonical
                );
                return (StatusCode::FORBIDDEN, "403 Forbidden — token scope denied")
                    .into_response();
            }
            info!("{} {} — auth ok (token: '{}')", method, path, record.name);
            req.extensions_mut().insert(auth);
            return next.run(req).await;
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
        warn!(
            "{} {} — 401 missing or invalid token (git smart-HTTP)",
            method, path
        );
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

#[cfg(test)]
mod tests {
    use super::find_registry_auth;
    use crate::profile::CredentialSource;
    use crate::registry::TokenRegistry;
    use axum::{body::Body, http::Request};
    use tempfile::TempDir;

    fn anthropic_api_key_source() -> CredentialSource {
        CredentialSource {
            env: "ANTHROPIC_API_KEY".to_string(),
            inject_header: "x-api-key".to_string(),
            format: "{}".to_string(),
            prefix: None,
            reject_prefixes: vec![],
            extra_headers: std::collections::BTreeMap::new(),
            basic_user: None,
        }
    }

    #[tokio::test]
    async fn registry_auth_accepts_raw_route_header_token() {
        let dir = TempDir::new().unwrap();
        let registry = TokenRegistry::load_or_create(dir.path().join("tokens.json"))
            .await
            .unwrap();
        let (_record, token) = registry
            .create_token("claude".to_string(), vec!["anthropic".to_string()], false)
            .await
            .unwrap();
        let req = Request::builder()
            .uri("/anthropic/v1/messages")
            .header("x-api-key", token)
            .body(Body::empty())
            .unwrap();

        let candidates = super::registry_auth_candidates(&req, &[anthropic_api_key_source()]);
        let auth = find_registry_auth(&registry, candidates).await.unwrap();

        assert_eq!(auth.header, "x-api-key");
        assert_eq!(auth.preferred_source, None);
        assert_eq!(auth.token_record.unwrap().name, "claude");
    }

    #[tokio::test]
    async fn registry_auth_prefers_valid_route_header_over_conflicting_authorization() {
        let dir = TempDir::new().unwrap();
        let registry = TokenRegistry::load_or_create(dir.path().join("tokens.json"))
            .await
            .unwrap();
        let (_record, token) = registry
            .create_token("claude".to_string(), vec!["anthropic".to_string()], false)
            .await
            .unwrap();
        let req = Request::builder()
            .uri("/anthropic/v1/messages")
            .header("authorization", "Bearer claude-ai-session-token")
            .header("x-api-key", token)
            .body(Body::empty())
            .unwrap();

        let candidates = super::registry_auth_candidates(&req, &[anthropic_api_key_source()]);
        let auth = find_registry_auth(&registry, candidates).await.unwrap();

        assert_eq!(auth.header, "x-api-key");
        assert_eq!(auth.preferred_source, None);
        assert_eq!(auth.token_record.unwrap().name, "claude");
    }
}
