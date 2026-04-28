//! Application state shared across handlers.

use crate::profile::{is_git_smart_http, RouteEntry, RouteMatcher};
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

/// A successful route resolution: the matched entry plus the path slice that
/// should be forwarded upstream (after consuming any prefix and applying
/// `strip_prefix`).
#[derive(Debug, Clone)]
pub struct ResolvedRoute<'a> {
    pub entry: &'a RouteEntry,
    pub upstream_path: &'a str,
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

    /// Resolves a full request path to a route entry and the upstream path.
    ///
    /// Order of resolution:
    /// 1. Non-prefix matchers (currently only `GitSmartHttp`) are checked
    ///    first; they consume the full path verbatim.
    /// 2. Otherwise the first path segment is matched against route keys
    ///    (existing behaviour). The matched prefix is stripped, and the
    ///    entry's `strip_prefix` is applied if set.
    pub fn resolve<'a>(&'a self, path: &'a str) -> Option<ResolvedRoute<'a>> {
        resolve_route(&self.routes, path)
    }
}

/// Free-function form of [`AppState::resolve`] for unit testing without
/// needing to construct a full [`AppState`].
pub fn resolve_route<'a>(
    routes: &'a [(String, RouteEntry)],
    path: &'a str,
) -> Option<ResolvedRoute<'a>> {
    for (_, entry) in routes {
        if entry.matcher == RouteMatcher::GitSmartHttp && is_git_smart_http(path) {
            return Some(ResolvedRoute {
                entry,
                upstream_path: path,
            });
        }
    }

    let stripped = path.trim_start_matches('/');
    let prefix = stripped.split('/').next().unwrap_or("");
    let entry = routes
        .iter()
        .find(|(key, _)| key == prefix)
        .map(|(_, entry)| entry)?;
    if entry.matcher != RouteMatcher::Prefix {
        return None;
    }

    let after_prefix = &path[prefix.len() + 1..];
    let stripped = if let Some(sp) = entry.strip_prefix.as_deref() {
        after_prefix.strip_prefix(sp).unwrap_or(after_prefix)
    } else {
        after_prefix
    };
    let upstream_path = if stripped.is_empty() { "/" } else { stripped };

    Some(ResolvedRoute {
        entry,
        upstream_path,
    })
}
