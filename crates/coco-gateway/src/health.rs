//! Health check endpoint.

use axum::Json;
use serde_json::json;

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}
