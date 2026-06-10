//! Reverse proxy handler.

use crate::auth::PhantomAuth;
use crate::profile::{CredentialSource, InjectMode, RouteEntry};
use crate::state::AppState;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderName, HeaderValue, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use http_body_util::BodyExt;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::io::copy_bidirectional;
use tracing::{error, info, warn};

pub async fn proxy_handler(State(state): State<Arc<AppState>>, mut req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let client_upgrade = is_upgrade_request(&req).then(|| hyper::upgrade::on(&mut req));

    let resolved_route = match state.resolve(&path) {
        Some(r) => r,
        None => {
            return (StatusCode::NOT_FOUND, "404 Not Found").into_response();
        }
    };
    let entry = resolved_route.entry;
    let upstream_path = resolved_route.upstream_path;

    let phantom_auth = req
        .extensions()
        .get::<PhantomAuth>()
        .cloned()
        .unwrap_or(PhantomAuth {
            header: "authorization".to_string(),
            preferred_source: None,
            token_record: None,
        });

    let resolved = resolve_route_credential(&state, entry, &phantom_auth);
    let (src, credential) = match resolved {
        Some(r) => r,
        None => {
            warn!(
                method = %method,
                path = %path,
                canonical_route = %entry.canonical_route,
                "no upstream credential available"
            );
            return (StatusCode::SERVICE_UNAVAILABLE, "503 Service Unavailable").into_response();
        }
    };

    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    let upstream_url = match &entry.inject_mode {
        InjectMode::Header => format!("{}{}{}", entry.upstream, upstream_path, query),
        InjectMode::UrlPath => {
            let pfx = entry.url_path_prefix.as_deref().unwrap_or("");
            format!(
                "{}{}{}{}{}",
                entry.upstream, pfx, credential, upstream_path, query
            )
        }
    };

    // For UrlPath mode the credential is embedded in the URL; use a redacted
    // form for all log output so real keys never appear in the log stream.
    let log_url = match &entry.inject_mode {
        InjectMode::UrlPath => {
            let pfx = entry.url_path_prefix.as_deref().unwrap_or("");
            format!(
                "{}{}[REDACTED]{}{}",
                entry.upstream, pfx, upstream_path, query
            )
        }
        InjectMode::Header => upstream_url.clone(),
    };

    let upstream_uri: Uri = match upstream_url.parse() {
        Ok(u) => u,
        Err(e) => {
            error!(
                method = %method,
                path = %path,
                canonical_route = %entry.canonical_route,
                upstream = %log_url,
                error = %e,
                "failed to parse upstream URI"
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let mut headers = req.headers().clone();

    remove_client_credential_headers(
        &mut headers,
        &entry.credential_sources,
        phantom_auth.header.as_str(),
    );
    headers.remove("host");

    // Only inject credential into headers for Header mode
    if entry.inject_mode == InjectMode::Header {
        let inject_value = if let Some(ref user) = src.basic_user {
            let encoded = BASE64_STANDARD.encode(format!("{}:{}", user, credential));
            format!("Basic {}", encoded)
        } else {
            src.format.replace("{}", &credential)
        };
        if let Ok(header_name) = HeaderName::from_bytes(src.inject_header.as_bytes()) {
            headers.insert(
                header_name,
                HeaderValue::from_str(&inject_value)
                    .unwrap_or_else(|_| HeaderValue::from_static("")),
            );
        }
        insert_extra_headers(&mut headers, src);
    }

    let upstream_host = upstream_uri.host().unwrap_or("");
    headers.insert(
        "host",
        HeaderValue::from_str(upstream_host).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    let body = if client_upgrade.is_some() {
        Body::empty()
    } else {
        req.into_body()
    };

    let mut upstream_req = Request::builder().method(&method).uri(upstream_uri);
    for (k, v) in &headers {
        upstream_req = upstream_req.header(k, v);
    }
    let upstream_req = match upstream_req.body(body) {
        Ok(r) => r,
        Err(e) => {
            error!(
                method = %method,
                path = %path,
                canonical_route = %entry.canonical_route,
                error = %e,
                "failed to build upstream request"
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    match state.https_client.request(upstream_req).await {
        Ok(resp) => {
            let status = resp.status();
            info!(
                method = %method,
                path = %path,
                canonical_route = %entry.canonical_route,
                upstream = %log_url,
                status = status.as_u16(),
                "upstream response"
            );

            if status == StatusCode::SWITCHING_PROTOCOLS {
                if let Some(client_upgrade) = client_upgrade {
                    return upgrade_response(resp, client_upgrade);
                }
            }

            let resp_headers = resp.headers().clone();
            let body = Body::new(resp.into_body().map_err(std::io::Error::other));
            let mut response = Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = resp_headers;
            response
        }
        Err(e) => {
            error!(
                method = %method,
                path = %path,
                canonical_route = %entry.canonical_route,
                upstream = %log_url,
                error = %e,
                "upstream request failed"
            );
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

fn is_upgrade_request(req: &Request<Body>) -> bool {
    req.headers()
        .get(axum::http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        && req.headers().contains_key(axum::http::header::UPGRADE)
}

fn upgrade_response(
    mut upstream_resp: hyper::Response<hyper::body::Incoming>,
    client_upgrade: OnUpgrade,
) -> Response {
    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();
    let upstream_upgrade = hyper::upgrade::on(&mut upstream_resp);

    tokio::spawn(async move {
        let (client, upstream) = match tokio::try_join!(client_upgrade, upstream_upgrade) {
            Ok(upgrades) => upgrades,
            Err(e) => {
                warn!("WebSocket upgrade failed: {}", e);
                return;
            }
        };

        let mut client = TokioIo::new(client);
        let mut upstream = TokioIo::new(upstream);
        if let Err(e) = copy_bidirectional(&mut client, &mut upstream).await {
            warn!("WebSocket tunnel closed with error: {}", e);
        }
    });

    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn remove_client_credential_headers(
    headers: &mut axum::http::HeaderMap,
    sources: &[CredentialSource],
    phantom_header: &str,
) {
    for src in sources {
        headers.remove(src.inject_header.as_str());
    }
    headers.remove(phantom_header);
}

fn insert_extra_headers(headers: &mut axum::http::HeaderMap, src: &CredentialSource) {
    for (name, value) in &src.extra_headers {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            warn!("Skipping invalid extra header name '{}'", name);
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(value) else {
            warn!("Skipping invalid extra header value for '{}'", name);
            continue;
        };

        if let Some(existing) = headers.get(&header_name) {
            let Ok(existing) = existing.to_str() else {
                headers.append(header_name, header_value);
                continue;
            };
            if existing
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(value))
            {
                continue;
            }
            let combined = format!("{}, {}", existing, value);
            match HeaderValue::from_str(&combined) {
                Ok(combined) => {
                    headers.insert(header_name, combined);
                }
                Err(_) => {
                    headers.append(header_name, header_value);
                }
            };
        } else {
            headers.insert(header_name, header_value);
        }
    }
}

/// Resolve the upstream credential for a proxied request using the priority chain:
///
/// 1. Explicit per-token binding: if `phantom_auth.token_record` has a `creds`
///    entry for the route, look up that name in the CredStore. If the binding
///    exists but the name is absent from the store → `None` (hard failure, no
///    fall-through). The credential source is selected by prefix/reject_prefixes
///    matching (same predicate as the env path). If the stored value matches no
///    source → `None`; no fall-through to rules 2/3.
/// 2. Service-default: `cred_store.get_for_service(route_id)` — a token whose
///    name matches the route id and whose service field also matches. Source
///    selection uses the same predicate. A registered service token whose value
///    matches no source returns `None` without falling through to env — a
///    registered token takes precedence over env even when it cannot be injected.
///    Falls through to rule 3 only on a complete store miss.
/// 3. Env-var path: existing `resolve_credential` behaviour.
pub fn resolve_route_credential<'a>(
    state: &'a AppState,
    entry: &'a RouteEntry,
    phantom_auth: &PhantomAuth,
) -> Option<(&'a CredentialSource, String)> {
    resolve_route_credential_with(&state.cred_store, entry, phantom_auth, |env| {
        std::env::var(env).ok()
    })
}

fn source_accepts(src: &CredentialSource, value: &str) -> bool {
    !value.is_empty()
        && src.prefix.as_deref().is_none_or(|p| value.starts_with(p))
        && !src.reject_prefixes.iter().any(|p| value.starts_with(p))
}

fn resolve_route_credential_with<'a>(
    cred_store: &crate::credstore::CredStore,
    entry: &'a RouteEntry,
    phantom_auth: &PhantomAuth,
    get_env: impl FnMut(&str) -> Option<String>,
) -> Option<(&'a CredentialSource, String)> {
    let route_id = entry.canonical_route.as_str();

    // Selects the first credential source whose prefix/reject_prefixes accept `value`.
    let pick_source = |value: &str| -> Option<&'a CredentialSource> {
        entry
            .credential_sources
            .iter()
            .find(|src| source_accepts(src, value))
    };

    // Rule 1: explicit per-token binding.
    if let Some(record) = &phantom_auth.token_record {
        if let Some(cred_name) = record.creds.get(route_id) {
            // Binding exists — must resolve or fail; no fall-through.
            match cred_store.get(cred_name) {
                None => {
                    tracing::warn!(
                        route = route_id,
                        binding = %cred_name,
                        "explicit credential binding is absent from the store"
                    );
                    return None;
                }
                Some(value) => {
                    let src = pick_source(&value)?;
                    return Some((src, value));
                }
            }
        }
    }

    // Rule 2: service-default (name == route_id and service matches).
    // A registered service token takes precedence over env; if its value matches
    // no source we return None rather than silently falling through to env.
    if let Some(value) = cred_store.get_for_service(route_id) {
        let src = pick_source(&value)?;
        return Some((src, value));
    }

    // Rule 3: env-var resolution (unchanged).
    resolve_credential_with(
        &entry.credential_sources,
        phantom_auth.preferred_source,
        get_env,
    )
}

pub fn resolve_credential(
    sources: &[CredentialSource],
    preferred: Option<usize>,
) -> Option<(&CredentialSource, String)> {
    resolve_credential_with(sources, preferred, |env| std::env::var(env).ok())
}

fn resolve_credential_with(
    sources: &[CredentialSource],
    preferred: Option<usize>,
    mut get_env: impl FnMut(&str) -> Option<String>,
) -> Option<(&CredentialSource, String)> {
    if let Some(i) = preferred {
        if let Some(src) = sources.get(i) {
            if let Some(v) = get_env(&src.env) {
                if source_accepts(src, &v) {
                    return Some((src, v));
                }
            }
        }
    }
    sources.iter().find_map(|src| {
        get_env(&src.env)
            .filter(|v| source_accepts(src, v))
            .map(|v| (src, v))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        insert_extra_headers, is_upgrade_request, remove_client_credential_headers,
        resolve_credential_with, resolve_route_credential_with,
    };
    use crate::auth::PhantomAuth;
    use crate::credstore::CredStore;
    use crate::profile::{CredentialSource, InjectMode, RouteEntry, RouteMatcher};
    use crate::registry::{TokenRecord, TokenStatus};
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, Request},
    };
    use chrono::Utc;
    use std::collections::BTreeMap;
    use uuid::Uuid;
    use zeroize::Zeroizing;

    fn source(header: &str) -> CredentialSource {
        CredentialSource {
            env: "TOKEN".to_string(),
            inject_header: header.to_string(),
            format: "{}".to_string(),
            prefix: None,
            reject_prefixes: vec![],
            extra_headers: BTreeMap::new(),
            basic_user: None,
        }
    }

    fn anthropic_source(header: &str, prefix: Option<&str>) -> CredentialSource {
        CredentialSource {
            env: "ANTHROPIC_API_KEY".to_string(),
            inject_header: header.to_string(),
            format: if header == "Authorization" {
                "Bearer {}".to_string()
            } else {
                "{}".to_string()
            },
            prefix: prefix.map(str::to_string),
            reject_prefixes: vec![],
            extra_headers: BTreeMap::new(),
            basic_user: None,
        }
    }

    #[test]
    fn detects_http_upgrade_requests() {
        let req = Request::builder()
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();

        assert!(is_upgrade_request(&req));

        let req = Request::builder()
            .header("connection", "keep-alive")
            .body(Body::empty())
            .unwrap();

        assert!(!is_upgrade_request(&req));
    }

    #[test]
    fn removes_all_route_credential_headers_before_injecting_upstream_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer claude-ai"),
        );
        headers.insert("x-api-key", HeaderValue::from_static("gate_test"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        remove_client_credential_headers(
            &mut headers,
            &[source("Authorization"), source("x-api-key")],
            "x-api-key",
        );

        assert!(!headers.contains_key("authorization"));
        assert!(!headers.contains_key("x-api-key"));
        assert!(headers.contains_key("content-type"));
    }

    #[test]
    fn removes_matched_proxy_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "proxy-authorization",
            HeaderValue::from_static("Bearer ccgw"),
        );
        headers.insert("authorization", HeaderValue::from_static("Bearer upstream"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        remove_client_credential_headers(
            &mut headers,
            &[source("Authorization")],
            "proxy-authorization",
        );

        assert!(!headers.contains_key("proxy-authorization"));
        assert!(!headers.contains_key("authorization"));
        assert!(headers.contains_key("content-type"));
    }

    #[test]
    fn inserts_extra_headers_for_selected_credential_source() {
        let mut source = anthropic_source("Authorization", Some("sk-ant-oat"));
        source
            .extra_headers
            .insert("anthropic-beta".to_string(), "oauth-2025-04-20".to_string());
        let mut headers = HeaderMap::new();

        insert_extra_headers(&mut headers, &source);

        assert_eq!(headers.get("anthropic-beta").unwrap(), "oauth-2025-04-20");
    }

    #[test]
    fn extra_headers_append_without_duplicate_beta_values() {
        let mut source = anthropic_source("Authorization", Some("sk-ant-oat"));
        source
            .extra_headers
            .insert("anthropic-beta".to_string(), "oauth-2025-04-20".to_string());
        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_static("other-beta-2025-01-01"),
        );

        insert_extra_headers(&mut headers, &source);
        insert_extra_headers(&mut headers, &source);

        assert_eq!(
            headers.get("anthropic-beta").unwrap(),
            "other-beta-2025-01-01, oauth-2025-04-20"
        );
    }

    #[test]
    fn unpreferred_anthropic_resolution_uses_secret_prefix() {
        let sources = [
            anthropic_source("Authorization", Some("sk-ant-oat")),
            anthropic_source("x-api-key", None),
        ];

        let resolved = resolve_credential_with(&sources, None, |env| match env {
            "ANTHROPIC_API_KEY" => Some("sk-ant-oat-local".to_string()),
            _ => None,
        })
        .expect("credential should resolve");

        assert_eq!(resolved.0.inject_header, "Authorization");
        assert_eq!(resolved.1, "sk-ant-oat-local");
    }

    #[test]
    fn anthropic_resolution_rejects_phantom_tokens_as_upstream_secrets() {
        let mut apikey = anthropic_source("x-api-key", None);
        apikey.reject_prefixes = vec!["gate_".to_string()];
        let sources = [apikey];

        let resolved = resolve_credential_with(&sources, None, |env| match env {
            "ANTHROPIC_API_KEY" => Some("gate_client_phantom".to_string()),
            _ => None,
        });

        assert!(resolved.is_none());
    }

    #[test]
    fn preferred_anthropic_api_key_source_does_not_steal_oauth_tokens() {
        let auth = anthropic_source("Authorization", Some("sk-ant-oat"));
        let mut apikey = anthropic_source("x-api-key", None);
        apikey.reject_prefixes = vec!["sk-ant-oat".to_string()];
        let sources = [auth, apikey];

        let resolved = resolve_credential_with(&sources, Some(1), |env| match env {
            "ANTHROPIC_API_KEY" => Some("sk-ant-oat-local".to_string()),
            _ => None,
        })
        .expect("credential should resolve");

        assert_eq!(resolved.0.inject_header, "Authorization");
    }

    // ---- resolve_route_credential_with tests ----

    fn route_entry(canonical_route: &str) -> RouteEntry {
        RouteEntry {
            canonical_route: canonical_route.to_string(),
            upstream: "https://example.com".to_string(),
            credential_sources: vec![CredentialSource {
                env: format!("{}_API_KEY", canonical_route.to_uppercase()),
                inject_header: "authorization".to_string(),
                format: "Bearer {}".to_string(),
                prefix: None,
                reject_prefixes: vec![],
                extra_headers: BTreeMap::new(),
                basic_user: None,
            }],
            strip_prefix: None,
            inject_mode: InjectMode::Header,
            url_path_prefix: None,
            matcher: RouteMatcher::Prefix,
        }
    }

    fn phantom_auth_no_record() -> PhantomAuth {
        PhantomAuth {
            header: "authorization".to_string(),
            preferred_source: None,
            token_record: None,
        }
    }

    fn phantom_auth_with_binding(route: &str, cred_name: &str) -> PhantomAuth {
        let mut creds = std::collections::HashMap::new();
        creds.insert(route.to_string(), cred_name.to_string());
        PhantomAuth {
            header: "authorization".to_string(),
            preferred_source: None,
            token_record: Some(TokenRecord {
                id: Uuid::new_v4(),
                name: "test-token".to_string(),
                scope: vec![route.to_string()],
                all_routes: false,
                created_at: Utc::now(),
                status: TokenStatus::Active,
                token_hash: "abc".to_string(),
                creds,
            }),
        }
    }

    fn cred_store_with(entries: &[(&str, &str, &str)]) -> CredStore {
        let store = CredStore::default();
        for (name, service, value) in entries {
            store.register(
                name.to_string(),
                service.to_string(),
                Zeroizing::new(value.to_string()),
            );
        }
        store
    }

    #[test]
    fn env_fallback_when_store_empty() {
        let store = CredStore::default();
        let entry = route_entry("github");
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        })
        .expect("should resolve via env");

        assert_eq!(resolved.1, "ghp_env_token");
    }

    #[test]
    fn service_default_wins_over_env() {
        let store = cred_store_with(&[("github", "github", "ghp_store_default")]);
        let entry = route_entry("github");
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        })
        .expect("should resolve via service-default");

        assert_eq!(resolved.1, "ghp_store_default");
    }

    #[test]
    fn explicit_binding_wins_over_service_default_and_env() {
        let store = cred_store_with(&[
            ("github", "github", "ghp_service_default"),
            ("gh-prod", "github", "ghp_explicit"),
        ]);
        let entry = route_entry("github");
        let auth = phantom_auth_with_binding("github", "gh-prod");

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        })
        .expect("should resolve via explicit binding");

        assert_eq!(resolved.1, "ghp_explicit");
    }

    #[test]
    fn explicit_binding_missing_from_store_returns_none_even_when_env_would_resolve() {
        let store = CredStore::default(); // gh-prod not registered
        let entry = route_entry("github");
        let auth = phantom_auth_with_binding("github", "gh-prod");

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        });

        assert!(
            resolved.is_none(),
            "binding exists but cred absent: must not fall through to env"
        );
    }

    #[test]
    fn after_remove_service_default_falls_back_to_env() {
        let store = cred_store_with(&[("github", "github", "ghp_store_default")]);
        store.remove("github");
        let entry = route_entry("github");
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        })
        .expect("should fall back to env after remove");

        assert_eq!(resolved.1, "ghp_env_token");
    }

    #[test]
    fn after_remove_explicit_binding_also_falls_back_to_none() {
        // Token has an explicit binding "gh-prod" → "github", but gh-prod was removed.
        let store = cred_store_with(&[("gh-prod", "github", "ghp_explicit")]);
        store.remove("gh-prod");
        let entry = route_entry("github");
        let auth = phantom_auth_with_binding("github", "gh-prod");

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "GITHUB_API_KEY" => Some("ghp_env_token".to_string()),
            _ => None,
        });

        assert!(resolved.is_none(), "removed binding must not fall through");
    }

    #[test]
    fn service_default_only_matches_name_equal_to_route_id() {
        // "gh-prod" is bound to "github" service but its name != "github".
        let store = cred_store_with(&[("gh-prod", "github", "ghp_named")]);
        let entry = route_entry("github");
        let auth = phantom_auth_no_record();

        // No explicit binding, no service-default (name≠route), no env → None.
        let resolved = resolve_route_credential_with(&store, &entry, &auth, |_env| None);

        assert!(
            resolved.is_none(),
            "service-default must require name == route_id"
        );
    }

    // ---- multi-source source-selection tests ----

    fn anthropic_route_entry() -> RouteEntry {
        // Mirrors the real anthropic profile: source 0 = Bearer/OAuth (sk-ant-oat prefix),
        // source 1 = x-api-key (no prefix, but rejects gate_ and sk-ant-oat prefixes).
        let mut src0 = CredentialSource {
            env: "ANTHROPIC_API_KEY".to_string(),
            inject_header: "Authorization".to_string(),
            format: "Bearer {}".to_string(),
            prefix: Some("sk-ant-oat".to_string()),
            reject_prefixes: vec![],
            extra_headers: BTreeMap::new(),
            basic_user: None,
        };
        src0.extra_headers
            .insert("anthropic-beta".to_string(), "oauth-2025-04-20".to_string());
        let src1 = CredentialSource {
            env: "ANTHROPIC_API_KEY".to_string(),
            inject_header: "x-api-key".to_string(),
            format: "{}".to_string(),
            prefix: None,
            reject_prefixes: vec!["gate_".to_string(), "sk-ant-oat".to_string()],
            extra_headers: BTreeMap::new(),
            basic_user: None,
        };
        RouteEntry {
            canonical_route: "anthropic".to_string(),
            upstream: "https://api.anthropic.com".to_string(),
            credential_sources: vec![src0, src1],
            strip_prefix: None,
            inject_mode: InjectMode::Header,
            url_path_prefix: None,
            matcher: RouteMatcher::Prefix,
        }
    }

    #[test]
    fn stored_plain_api_key_selects_x_api_key_source_on_multi_source_route() {
        // A plain `sk-ant-api03-...` key should resolve via source 1 (x-api-key),
        // not source 0 (Bearer/OAuth) which requires `sk-ant-oat` prefix.
        let plain_key = "sk-ant-api03-stored";
        let store = cred_store_with(&[("anthropic", "anthropic", plain_key)]);
        let entry = anthropic_route_entry();
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |_| None)
            .expect("plain key should resolve via x-api-key source");

        assert_eq!(
            resolved.0.inject_header, "x-api-key",
            "wrong source selected"
        );
        assert_eq!(resolved.1, plain_key);
    }

    #[test]
    fn stored_oauth_token_selects_authorization_source_on_multi_source_route() {
        let oauth_token = "sk-ant-oat-stored";
        let store = cred_store_with(&[("anthropic", "anthropic", oauth_token)]);
        let entry = anthropic_route_entry();
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |_| None)
            .expect("OAuth token should resolve via Authorization source");

        assert_eq!(resolved.0.inject_header, "Authorization");
        assert_eq!(resolved.1, oauth_token);
    }

    #[test]
    fn stored_phantom_gate_token_is_rejected_by_all_sources() {
        // A gate_ token accidentally registered as a service credential must be
        // rejected — no source in anthropic accepts the gate_ prefix.
        let phantom = "gate_phantom_accidental";
        let store = cred_store_with(&[("anthropic", "anthropic", phantom)]);
        let entry = anthropic_route_entry();
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |_| None);
        assert!(
            resolved.is_none(),
            "gate_ token in store must not be forwarded upstream"
        );
    }

    #[test]
    fn explicit_binding_with_stored_phantom_token_is_rejected() {
        let phantom = "gate_phantom_explicit";
        let store = cred_store_with(&[("my-cred", "anthropic", phantom)]);
        let entry = anthropic_route_entry();
        let auth = phantom_auth_with_binding("anthropic", "my-cred");

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |_| None);
        assert!(
            resolved.is_none(),
            "explicit binding with gate_ value must not be forwarded upstream"
        );
    }

    #[test]
    fn service_default_non_matching_value_does_not_fall_through_to_env() {
        // Service-default entry exists but its value (a phantom) matches no source.
        // The env would resolve fine, but we must NOT fall through.
        let phantom = "gate_phantom_service";
        let store = cred_store_with(&[("anthropic", "anthropic", phantom)]);
        let entry = anthropic_route_entry();
        let auth = phantom_auth_no_record();

        let resolved = resolve_route_credential_with(&store, &entry, &auth, |env| match env {
            "ANTHROPIC_API_KEY" => Some("sk-ant-api03-env".to_string()),
            _ => None,
        });
        assert!(
            resolved.is_none(),
            "registered service token with bad value must not fall through to env"
        );
    }
}
