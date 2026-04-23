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
pub use profile::{CredentialSource, InjectMode, ProfileRoute, RouteEntry};
pub use registry::{TokenRecord, TokenRegistry, TokenStatus};
pub use state::{AppState, HttpsClient};

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Constant-time byte comparison. Returns `false` immediately on length
/// mismatch (length is not secret); otherwise compares in constant time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.ct_eq(b).into()
}

/// Validate a `Proxy-Authorization` header against the phantom token.
///
/// Accepts:
///   - `Bearer <token>`
///   - `Basic base64(user:<token>)`
pub fn validate_proxy_authorization(header_bytes: &[u8], token: &Zeroizing<String>) -> bool {
    let Ok(header_str) = std::str::from_utf8(header_bytes) else {
        return false;
    };
    let lower = header_str.to_lowercase();
    if let Some(rest) = lower.strip_prefix("bearer ") {
        let candidate = &header_str[header_str.len() - rest.len()..];
        return constant_time_eq(candidate.trim().as_bytes(), token.as_bytes());
    }
    if let Some(rest) = lower.strip_prefix("basic ") {
        let encoded = &header_str[header_str.len() - rest.len()..];
        return validate_basic_auth(encoded.trim(), token);
    }
    false
}

/// Validate a header value that may be `Bearer <token>` or the raw token.
///
/// Used for per-route credential headers (e.g., `x-api-key: <phantom>` or
/// `Authorization: Bearer <phantom>`).
pub fn validate_bearer_or_raw(header_bytes: &[u8], token: &Zeroizing<String>) -> bool {
    let Ok(s) = std::str::from_utf8(header_bytes) else {
        return false;
    };
    let lower = s.to_lowercase();
    let candidate = if let Some(rest) = lower.strip_prefix("bearer ") {
        &s[s.len() - rest.len()..]
    } else {
        s
    };
    constant_time_eq(candidate.trim().as_bytes(), token.as_bytes())
}

fn validate_basic_auth(encoded: &str, token: &Zeroizing<String>) -> bool {
    use base64::Engine;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded_str) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let password = match decoded_str.split_once(':') {
        Some((_, pw)) => pw,
        None => return false,
    };
    constant_time_eq(password.as_bytes(), token.as_bytes())
}
