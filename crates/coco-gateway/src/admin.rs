//! Admin API for token management.

use crate::state::AppState;

use crate::registry::TokenCreateError;
use axum::{
    extract::{Path, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path as FsPath};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::{error, warn};
use uuid::Uuid;

const UNRESTRICTED_SCOPE_WARNING: &str = "Empty token scope allows all current and future routes.";

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub all_routes: bool,
}

#[derive(Serialize)]
struct TokenResponse {
    id: Uuid,
    name: String,
    scope: Vec<String>,
    all_routes: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

#[derive(Serialize)]
struct TokenListResponse {
    id: Uuid,
    name: String,
    scope: Vec<String>,
    all_routes: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    status: String,
}

pub fn admin_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin/tokens", post(create_token).get(list_tokens))
        .route("/admin/tokens/{id}", delete(revoke_token))
        .layer(axum::middleware::from_fn_with_state(
            state,
            admin_auth_middleware,
        ))
}

async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match auth_header {
        Some(token) if constant_time_compare(token, &state.admin_token) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "401 Unauthorized").into_response(),
    }
}

fn constant_time_compare(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

async fn create_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenRequest>,
) -> Response {
    let registry = match &state.token_registry {
        Some(r) => r,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Token registry not configured",
            )
                .into_response()
        }
    };

    let known_routes = state.known_routes();
    if let Err(message) = validate_token_name(&req.name) {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }
    if let Err(message) = validate_token_scope(&req.scope, req.all_routes, &known_routes) {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }

    let (record, token_value) = match registry
        .create_token(req.name, req.scope, req.all_routes)
        .await
    {
        Ok(created) => created,
        Err(TokenCreateError::DuplicateName { name }) => {
            return (
                StatusCode::CONFLICT,
                format!("token name '{}' already exists", name),
            )
                .into_response()
        }
        Err(TokenCreateError::Persist { source }) => {
            error!("Failed to persist created token: {}", source);
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to persist token").into_response();
        }
    };
    let warning = record
        .is_all_routes()
        .then(|| UNRESTRICTED_SCOPE_WARNING.to_string());
    if let Some(warning) = &warning {
        warn!("Created unrestricted token '{}': {}", record.name, warning);
    }

    Json(TokenResponse {
        id: record.id,
        name: record.name,
        scope: record.scope,
        all_routes: record.all_routes,
        created_at: record.created_at,
        token: token_value,
        warning,
    })
    .into_response()
}

async fn list_tokens(State(state): State<Arc<AppState>>) -> Response {
    let registry = match &state.token_registry {
        Some(r) => r,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Token registry not configured",
            )
                .into_response()
        }
    };

    let tokens = registry.list_tokens().await;
    let items: Vec<TokenListResponse> = tokens
        .into_iter()
        .map(|r| TokenListResponse {
            id: r.id,
            name: r.name,
            scope: r.scope,
            all_routes: r.all_routes,
            created_at: r.created_at,
            status: match r.status {
                crate::registry::TokenStatus::Active => "active".to_string(),
                crate::registry::TokenStatus::Revoked => "revoked".to_string(),
            },
        })
        .collect();

    Json(items).into_response()
}

async fn revoke_token(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> Response {
    let registry = match &state.token_registry {
        Some(r) => r,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Token registry not configured",
            )
                .into_response()
        }
    };

    match registry.revoke_token(id).await {
        Ok(true) => (StatusCode::OK, "Token revoked").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(error) => {
            error!("Failed to persist revoked token: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to persist token").into_response()
        }
    }
}

fn validate_token_name(name: &str) -> Result<(), String> {
    let mut components = FsPath::new(name).components();
    let first = components.next();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || components.next().is_some()
        || !matches!(first, Some(Component::Normal(component)) if component == name)
    {
        return Err(
            "token name must be a single safe path component (not empty, '.', '..', or a path)"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_token_scope(
    scope: &[String],
    all_routes: bool,
    known_routes: &[String],
) -> Result<(), String> {
    if all_routes && !scope.is_empty() {
        return Err("use either scope or all_routes, not both".to_string());
    }
    if scope.is_empty() && !all_routes {
        return Err(
            "scope must be non-empty (or pass all_routes=true for unrestricted)".to_string(),
        );
    }

    let unknown: Vec<_> = scope
        .iter()
        .filter(|route| !known_routes.contains(route))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "unknown route(s): {} (known: {})",
            unknown.join(", "),
            known_routes.join(", ")
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_token_name, validate_token_scope};

    fn known_routes() -> Vec<String> {
        ["anthropic", "github", "openai"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn rejects_empty_scope_without_all_routes() {
        let error = validate_token_scope(&[], false, &known_routes()).unwrap_err();
        assert!(error.contains("scope must be non-empty"));
    }

    #[test]
    fn rejects_token_names_that_are_paths() {
        for name in ["", ".", "..", "../escape", "nested/name", r"nested\name"] {
            let error = validate_token_name(name).unwrap_err();
            assert!(error.contains("token name must be a single safe path component"));
        }

        validate_token_name("laptop token").unwrap();
        validate_token_name("laptop-1").unwrap();
    }

    #[test]
    fn rejects_unknown_scope() {
        let error =
            validate_token_scope(&["anthroppapa".to_string()], false, &known_routes()).unwrap_err();
        assert!(error.contains("unknown route(s): anthroppapa"));
        assert!(error.contains("known: anthropic, github, openai"));
    }

    #[test]
    fn accepts_explicit_all_routes() {
        validate_token_scope(&[], true, &known_routes()).unwrap();
    }
}
