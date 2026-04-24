//! Admin API for token management.

use crate::state::AppState;

use axum::{
    extract::{Path, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::warn;
use uuid::Uuid;

const UNRESTRICTED_SCOPE_WARNING: &str = "Empty token scope allows all current and future routes.";

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scope: Vec<String>,
}

#[derive(Serialize)]
struct TokenResponse {
    id: Uuid,
    name: String,
    scope: Vec<String>,
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

    let (record, token_value) = registry.create_token(req.name, req.scope).await;
    let warning = record
        .is_unrestricted()
        .then(|| UNRESTRICTED_SCOPE_WARNING.to_string());
    if let Some(warning) = &warning {
        warn!("Created unrestricted token '{}': {}", record.name, warning);
    }

    Json(TokenResponse {
        id: record.id,
        name: record.name,
        scope: record.scope,
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

    if registry.revoke_token(id).await {
        (StatusCode::OK, "Token revoked").into_response()
    } else {
        (StatusCode::NOT_FOUND, "Token not found").into_response()
    }
}
