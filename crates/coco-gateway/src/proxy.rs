//! Reverse proxy handler.

use crate::auth::PhantomAuth;
use crate::profile::{CredentialSource, InjectMode};
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

    let resolved = resolve_credential(&entry.credential_sources, phantom_auth.preferred_source);
    let (src, credential) = match resolved {
        Some(r) => r,
        None => {
            warn!(
                "No credential available for route '{}'",
                entry.canonical_route
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

    let upstream_uri: Uri = match upstream_url.parse() {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to parse upstream URI {}: {}", upstream_url, e);
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
        match req.into_body().collect().await {
            Ok(b) => Body::from(b.to_bytes()),
            Err(e) => {
                error!("Failed to read request body: {}", e);
                return StatusCode::BAD_GATEWAY.into_response();
            }
        }
    };

    let mut upstream_req = Request::builder().method(&method).uri(upstream_uri);
    for (k, v) in &headers {
        upstream_req = upstream_req.header(k, v);
    }
    let upstream_req = match upstream_req.body(body) {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to build upstream request: {}", e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    match state.https_client.request(upstream_req).await {
        Ok(resp) => {
            let status = resp.status();
            info!("{} {} → {} [{}]", method, path, upstream_url, status);

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
            error!("Upstream request failed: {}", e);
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
    let matches = |src: &CredentialSource, v: &str| -> bool {
        !v.is_empty()
            && src.prefix.as_deref().is_none_or(|p| v.starts_with(p))
            && !src.reject_prefixes.iter().any(|p| v.starts_with(p))
    };

    if let Some(i) = preferred {
        if let Some(src) = sources.get(i) {
            if let Some(v) = get_env(&src.env) {
                if matches(src, &v) {
                    return Some((src, v));
                }
            }
        }
    }
    sources.iter().find_map(|src| {
        get_env(&src.env)
            .filter(|v| matches(src, v))
            .map(|v| (src, v))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        insert_extra_headers, is_upgrade_request, remove_client_credential_headers,
        resolve_credential_with,
    };
    use crate::profile::CredentialSource;
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, Request},
    };
    use std::collections::BTreeMap;

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
        headers.insert("x-api-key", HeaderValue::from_static("ccgw_test"));
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
        apikey.reject_prefixes = vec!["ccgw_".to_string()];
        let sources = [apikey];

        let resolved = resolve_credential_with(&sources, None, |env| match env {
            "ANTHROPIC_API_KEY" => Some("ccgw_client_phantom".to_string()),
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
}
