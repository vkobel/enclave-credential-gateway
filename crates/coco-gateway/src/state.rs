//! Application state shared across handlers.

use crate::profile::RouteEntry;
use crate::registry::TokenRegistry;
use axum::body::Body;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use std::collections::HashMap;
use std::sync::Arc;
use zeroize::Zeroizing;

pub type HttpsClient = Client<HttpsConnector<HttpConnector>, Body>;

pub struct AppState {
    pub phantom_token: Option<Zeroizing<String>>,
    pub token_registry: Option<Arc<TokenRegistry>>,
    pub admin_token: Zeroizing<String>,
    pub routes: Vec<(String, RouteEntry)>,
    /// Maps alias path prefix → canonical route key (e.g. "api" → "github").
    pub route_aliases: HashMap<String, String>,
    pub https_client: HttpsClient,
}

impl AppState {
    /// Resolves a request path prefix to its canonical route key, following aliases.
    pub fn resolve_route_key<'a>(&'a self, prefix: &'a str) -> &'a str {
        self.route_aliases.get(prefix).map(|s| s.as_str()).unwrap_or(prefix)
    }
}