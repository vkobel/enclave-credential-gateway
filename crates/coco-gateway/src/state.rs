//! Application state shared across handlers.

use crate::profile::RouteEntry;
use crate::registry::TokenRegistry;
use axum::body::Body;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use std::sync::Arc;
use zeroize::Zeroizing;

pub type HttpsClient = Client<HttpsConnector<HttpConnector>, Body>;

pub struct AppState {
    pub phantom_token: Option<Zeroizing<String>>,
    pub token_registry: Option<Arc<TokenRegistry>>,
    pub admin_token: Zeroizing<String>,
    pub routes: Vec<(String, RouteEntry)>,
    pub https_client: HttpsClient,
}

impl AppState {
    pub fn route_entry(&self, prefix: &str) -> Option<&RouteEntry> {
        self.routes
            .iter()
            .find(|(key, _)| key == prefix)
            .map(|(_, entry)| entry)
    }

    /// Resolves a request path prefix to its canonical route key.
    pub fn canonical_route_key<'a>(&'a self, prefix: &'a str) -> &'a str {
        self.route_entry(prefix)
            .map(|entry| entry.canonical_route.as_str())
            .unwrap_or(prefix)
    }
}
