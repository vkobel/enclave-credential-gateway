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
use std::sync::Arc;
use tracing::{error, info, warn};

pub async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

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
    }

    let upstream_host = upstream_uri.host().unwrap_or("");
    headers.insert(
        "host",
        HeaderValue::from_str(upstream_host).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    let body_bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let mut upstream_req = Request::builder().method(&method).uri(upstream_uri);
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
            info!("{} {} → {} [{}]", method, path, upstream_url, status);
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

pub fn resolve_credential(
    sources: &[CredentialSource],
    preferred: Option<usize>,
) -> Option<(&CredentialSource, String)> {
    let matches = |src: &CredentialSource, v: &str| -> bool {
        !v.is_empty() && src.prefix.as_deref().is_none_or(|p| v.starts_with(p))
    };

    if let Some(i) = preferred {
        if let Some(src) = sources.get(i) {
            if let Ok(v) = std::env::var(&src.env) {
                if matches(src, &v) {
                    return Some((src, v));
                }
            }
        }
    }
    sources.iter().find_map(|src| {
        std::env::var(&src.env)
            .ok()
            .filter(|v| matches(src, v))
            .map(|v| (src, v))
    })
}

#[cfg(test)]
mod tests {
    use super::remove_client_credential_headers;
    use crate::profile::CredentialSource;
    use axum::http::{HeaderMap, HeaderValue};

    fn source(header: &str) -> CredentialSource {
        CredentialSource {
            env: "TOKEN".to_string(),
            inject_header: header.to_string(),
            format: "{}".to_string(),
            prefix: None,
            basic_user: None,
        }
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
}
