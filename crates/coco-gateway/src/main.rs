//! CoCo Credential Gateway — startup and router assembly.

use coco_gateway::admin::admin_router;
use coco_gateway::auth::auth_middleware;
use coco_gateway::health::health_handler;
use coco_gateway::profile::load_profile;
use coco_gateway::proxy::proxy_handler;
use coco_gateway::registry::TokenRegistry;
use coco_gateway::AppState;

use axum::{middleware, routing::get, Router};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Admin token is always required
    let admin_token = match std::env::var("COCO_ADMIN_TOKEN") {
        Ok(t) if !t.is_empty() => Zeroizing::new(t),
        Ok(_) => {
            error!("COCO_ADMIN_TOKEN is set but empty — refusing to start");
            std::process::exit(1);
        }
        Err(_) => {
            error!("COCO_ADMIN_TOKEN is not set — refusing to start");
            std::process::exit(1);
        }
    };

    // Phantom token is optional if a registry is configured
    let phantom_token = match std::env::var("COCO_PHANTOM_TOKEN") {
        Ok(t) if !t.is_empty() => Some(Zeroizing::new(t)),
        Ok(_) => {
            error!("COCO_PHANTOM_TOKEN is set but empty");
            std::process::exit(1);
        }
        Err(_) => None,
    };

    // Load token registry if configured
    let tokens_path = std::env::var("COCO_TOKENS_FILE")
        .ok()
        .unwrap_or_else(|| "./tokens.json".to_string());
    let tokens_pathbuf = PathBuf::from(&tokens_path);

    let token_registry = if tokens_pathbuf.exists() || std::env::var("COCO_TOKENS_FILE").is_ok() {
        match TokenRegistry::load_or_create(tokens_pathbuf).await {
            Ok(reg) => {
                info!("Token registry loaded from {}", tokens_path);
                Some(Arc::new(reg))
            }
            Err(e) => {
                error!("Failed to load token registry from {}: {}", tokens_path, e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Require at least one auth method
    if phantom_token.is_none() && token_registry.is_none() {
        error!("Neither COCO_PHANTOM_TOKEN nor token registry configured — refusing to start");
        std::process::exit(1);
    }

    let port: u16 = std::env::var("COCO_LISTEN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    let routes = load_profile();
    info!(
        "Loaded {} embedded route(s) from profiles/routes.json",
        routes.len()
    );

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();

    let https_client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector);

    let state = Arc::new(AppState {
        phantom_token,
        token_registry,
        admin_token,
        routes,
        https_client,
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .merge(admin_router(state.clone()))
        .merge(
            Router::new()
                .fallback(proxy_handler)
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    auth_middleware,
                ))
                .with_state(state.clone()),
        )
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        });

    info!("coco-gateway listening on {}", addr);
    axum::serve(listener, app).await.expect("server error");
}
