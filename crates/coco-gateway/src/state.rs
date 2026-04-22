//! Application state shared across handlers.

use crate::profile::RouteEntry;
use crate::registry::TokenRegistry;
use axum::body::Body;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
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