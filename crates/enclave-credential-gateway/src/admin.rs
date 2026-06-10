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
use std::collections::HashMap;
use std::path::{Component, Path as FsPath};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::{error, info, warn};
use uuid::Uuid;
use zeroize::Zeroizing;

const UNRESTRICTED_SCOPE_WARNING: &str = "Empty token scope allows all current and future routes.";

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub all_routes: bool,
    #[serde(default)]
    pub creds: HashMap<String, String>,
}

/// Request body for `POST /admin/creds`. Does NOT derive Debug — value must never be printed.
#[derive(Deserialize)]
pub struct RegisterCredRequest {
    pub name: String,
    pub service: String,
    pub value: String,
}

#[derive(Serialize)]
struct CredEntry {
    name: String,
    service: String,
}

#[derive(Serialize)]
struct CredListResponse {
    creds: Vec<CredEntry>,
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
        .route("/admin/creds", post(register_cred).get(list_creds))
        .route("/admin/creds/{name}", delete(delete_cred))
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
    if let Err(message) =
        validate_token_creds(&req.creds, &req.scope, req.all_routes, &state.cred_store)
    {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }

    let (record, token_value) = match registry
        .create_token(req.name, req.scope, req.all_routes, req.creds)
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

// POST is an upsert: re-registering an existing name overwrites the value (rotation).
async fn register_cred(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterCredRequest>,
) -> Response {
    let known_routes = state.known_routes();
    if let Err(message) = validate_cred_name(&req.name) {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }
    if !known_routes.contains(&req.service) {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "unknown service '{}' (known: {})",
                req.service,
                known_routes.join(", ")
            ),
        )
            .into_response();
    }
    let value = Zeroizing::new(req.value);
    state
        .cred_store
        .register(req.name.clone(), req.service.clone(), value);
    info!(
        "registered cred name='{}' service='{}'",
        req.name, req.service
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn list_creds(State(state): State<Arc<AppState>>) -> Response {
    let creds = state
        .cred_store
        .list()
        .into_iter()
        .map(|(name, service)| CredEntry { name, service })
        .collect();
    Json(CredListResponse { creds }).into_response()
}

async fn delete_cred(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    // 204 regardless of presence: idempotent delete (vs tokens' 404) is deliberate.
    state.cred_store.remove(&name);
    info!("deleted cred name='{}'", name);
    StatusCode::NO_CONTENT.into_response()
}

fn validate_cred_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("cred name must be 1–128 characters".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("cred name must contain only [A-Za-z0-9_-]".to_string());
    }
    Ok(())
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

fn validate_token_creds(
    creds: &HashMap<String, String>,
    scope: &[String],
    all_routes: bool,
    cred_store: &crate::credstore::CredStore,
) -> Result<(), String> {
    for (route, cred_name) in creds {
        if !all_routes && !scope.contains(route) {
            return Err(format!(
                "creds key '{}' is not within the token scope",
                route
            ));
        }
        match cred_store.service_of(cred_name) {
            None => {
                return Err(format!(
                    "creds entry '{}' → '{}': cred name not found in store",
                    route, cred_name
                ));
            }
            Some(ref stored_service) if stored_service != route => {
                return Err(format!(
                    "creds entry '{}' → '{}': cred is for service '{}', not '{}'",
                    route, cred_name, stored_service, route
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_cred_name, validate_token_creds, validate_token_name, validate_token_scope,
    };
    use crate::credstore::CredStore;
    use std::collections::HashMap;
    use zeroize::Zeroizing;

    fn make_store(entries: &[(&str, &str, &str)]) -> CredStore {
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

    #[test]
    fn cred_name_rejects_empty() {
        let error = validate_cred_name("").unwrap_err();
        assert!(error.contains("1–128"));
    }

    #[test]
    fn cred_name_rejects_too_long() {
        let long = "a".repeat(129);
        let error = validate_cred_name(&long).unwrap_err();
        assert!(error.contains("1–128"));
    }

    #[test]
    fn cred_name_rejects_invalid_chars() {
        for name in ["gh prod", "gh/prod", "gh.prod", "gh@prod"] {
            let error = validate_cred_name(name).unwrap_err();
            assert!(
                error.contains("[A-Za-z0-9_-]"),
                "expected char error for {:?}, got: {}",
                name,
                error
            );
        }
    }

    #[test]
    fn cred_name_accepts_valid() {
        for name in ["gh-prod", "gh_prod", "GH-PROD-1", "a", "A1-b_C"] {
            validate_cred_name(name).unwrap();
        }
    }

    #[test]
    fn validate_token_creds_rejects_key_outside_scope() {
        let store = make_store(&[("gh-prod", "github", "ghp_secret")]);
        let creds: HashMap<String, String> = [("github".to_string(), "gh-prod".to_string())].into();
        let error =
            validate_token_creds(&creds, &["openai".to_string()], false, &store).unwrap_err();
        assert!(
            error.contains("github"),
            "error should name the offending key; got: {error}"
        );
        assert!(
            error.contains("not within the token scope"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn validate_token_creds_allows_key_when_all_routes() {
        let store = make_store(&[("gh-prod", "github", "ghp_secret")]);
        let creds: HashMap<String, String> = [("github".to_string(), "gh-prod".to_string())].into();
        // scope is empty but all_routes=true — key "github" is not in scope slice, yet must pass
        validate_token_creds(&creds, &[], true, &store).unwrap();
    }

    #[test]
    fn validate_token_creds_rejects_missing_cred_name() {
        let store = make_store(&[]); // store is empty
        let creds: HashMap<String, String> =
            [("openai".to_string(), "nonexistent-cred".to_string())].into();
        let error =
            validate_token_creds(&creds, &["openai".to_string()], false, &store).unwrap_err();
        assert!(
            error.contains("nonexistent-cred"),
            "error should name the missing cred; got: {error}"
        );
        assert!(
            error.contains("not found in store"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn validate_token_creds_accepts_valid_entry() {
        let store = make_store(&[("sk-prod", "openai", "sk-real")]);
        let creds: HashMap<String, String> = [("openai".to_string(), "sk-prod".to_string())].into();
        validate_token_creds(&creds, &["openai".to_string()], false, &store).unwrap();
    }

    #[test]
    fn validate_token_creds_rejects_service_mismatch() {
        // "gh-prod" is a github cred, but the token scope/key is "openai"
        let store = make_store(&[("gh-prod", "github", "ghp_secret")]);
        let creds: HashMap<String, String> = [("openai".to_string(), "gh-prod".to_string())].into();
        let error =
            validate_token_creds(&creds, &["openai".to_string()], false, &store).unwrap_err();
        assert!(
            error.contains("gh-prod"),
            "error should name the offending entry; got: {error}"
        );
        assert!(
            error.contains("github"),
            "error should name the actual service; got: {error}"
        );
        assert!(
            error.contains("openai"),
            "error should name the expected service; got: {error}"
        );
    }
}
