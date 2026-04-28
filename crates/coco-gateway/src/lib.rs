//! Shared logic for coco-gateway — exported for integration tests.

pub mod admin;
pub mod auth;
pub mod health;
pub mod profile;
pub mod proxy;
pub mod registry;
pub mod state;

// Re-exports for convenience
pub use auth::PhantomAuth;
pub use profile::{
    is_git_smart_http, CredentialSource, GitProtocolRoute, InjectMode, ProfileRoute, RouteEntry,
    RouteMatcher,
};
pub use registry::{TokenRecord, TokenRegistry, TokenStatus};
pub use state::{resolve_route, AppState, HttpsClient, ResolvedRoute};

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Constant-time byte comparison. Returns `false` immediately on length
/// mismatch (length is not secret); otherwise compares in constant time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.ct_eq(b).into()
}

/// Validate a header value that may be `Bearer <token>`, `token <token>`
/// (GitHub CLI legacy format), or the raw token.
pub fn validate_bearer_or_raw(header_bytes: &[u8], token: &Zeroizing<String>) -> bool {
    let Ok(s) = std::str::from_utf8(header_bytes) else {
        return false;
    };
    let lower = s.to_lowercase();
    let candidate = if let Some(rest) = lower.strip_prefix("bearer ") {
        &s[s.len() - rest.len()..]
    } else if let Some(rest) = lower.strip_prefix("token ") {
        &s[s.len() - rest.len()..]
    } else {
        s
    };
    constant_time_eq(candidate.trim().as_bytes(), token.as_bytes())
}
