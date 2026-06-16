//! RAM-only registry of named service tokens (real upstream API keys).
//!
//! Values are stored in `Zeroizing<String>` and never printed or logged.

use std::collections::HashMap;
use std::sync::RwLock;
use zeroize::Zeroizing;

/// A real upstream API key bound to a specific route/service id.
pub struct ServiceToken {
    pub service: String,
    pub value: Zeroizing<String>,
}

/// Thread-safe, RAM-only store of named service tokens, keyed by name.
#[derive(Default)]
pub struct CredStore {
    inner: RwLock<HashMap<String, ServiceToken>>,
}

impl CredStore {
    /// Insert or overwrite a named service token.
    pub fn register(&self, name: String, service: String, value: Zeroizing<String>) {
        let mut map = self.inner.write().expect("credstore write lock poisoned");
        map.insert(name, ServiceToken { service, value });
    }

    /// Remove a named token. Returns `true` if the name was present.
    pub fn remove(&self, name: &str) -> bool {
        let mut map = self.inner.write().expect("credstore write lock poisoned");
        map.remove(name).is_some()
    }

    /// Return a clone of the token value for the given name, or `None`.
    pub fn get(&self, name: &str) -> Option<String> {
        let map = self.inner.read().expect("credstore read lock poisoned");
        map.get(name).map(|t| t.value.as_str().to_string())
    }

    /// Return the token value only if its `service` field matches `expected_service`.
    /// Returns `None` when the name is absent **or** the service does not match.
    /// Never returns the value on a service mismatch.
    pub fn get_checked(&self, name: &str, expected_service: &str) -> Option<String> {
        let map = self.inner.read().expect("credstore read lock poisoned");
        map.get(name)
            .filter(|t| t.service == expected_service)
            .map(|t| t.value.as_str().to_string())
    }

    /// Return the `service` field for the named token, or `None` if absent.
    /// Never returns the token value.
    pub fn service_of(&self, name: &str) -> Option<String> {
        let map = self.inner.read().expect("credstore read lock poisoned");
        map.get(name).map(|t| t.service.clone())
    }

    /// Convenience: return the value of the token whose **name equals `service`**,
    /// provided its `service` field also matches `service`. Used as resolution
    /// rule 2 (service-default): a well-known name such as "github" resolves
    /// automatically when a token named "github" is registered for the "github"
    /// service.
    pub fn get_for_service(&self, service: &str) -> Option<String> {
        let map = self.inner.read().expect("credstore read lock poisoned");
        map.get(service)
            .filter(|t| t.service == service)
            .map(|t| t.value.as_str().to_string())
    }

    /// List all registered tokens as `(name, service)` pairs, sorted by name.
    /// Values are intentionally excluded.
    pub fn list(&self) -> Vec<(String, String)> {
        let map = self.inner.read().expect("credstore read lock poisoned");
        let mut items: Vec<(String, String)> = map
            .iter()
            .map(|(name, t)| (name.clone(), t.service.clone()))
            .collect();
        items.sort_by(|(a, _), (b, _)| a.cmp(b));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::CredStore;
    use zeroize::Zeroizing;

    fn store_with(entries: &[(&str, &str, &str)]) -> CredStore {
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

    #[test]
    fn get_returns_none_for_missing_name() {
        let store = CredStore::default();
        assert!(store.get("nope").is_none());
    }

    #[test]
    fn register_and_get_round_trips() {
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert_eq!(store.get("gh-prod").as_deref(), Some("ghp_secret"));
    }

    #[test]
    fn register_overwrites_existing_name() {
        let store = store_with(&[("gh-prod", "github", "ghp_old")]);
        store.register(
            "gh-prod".to_string(),
            "github".to_string(),
            Zeroizing::new("ghp_new".to_string()),
        );
        assert_eq!(store.get("gh-prod").as_deref(), Some("ghp_new"));
    }

    #[test]
    fn remove_returns_true_when_present_and_false_when_absent() {
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert!(store.remove("gh-prod"));
        assert!(!store.remove("gh-prod"));
        assert!(store.get("gh-prod").is_none());
    }

    #[test]
    fn list_returns_name_service_pairs_sorted_by_name() {
        let store = store_with(&[
            ("z-token", "openai", "sk-z"),
            ("a-token", "anthropic", "sk-a"),
            ("m-token", "github", "ghp_m"),
        ]);
        let items = store.list();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], ("a-token".to_string(), "anthropic".to_string()));
        assert_eq!(items[1], ("m-token".to_string(), "github".to_string()));
        assert_eq!(items[2], ("z-token".to_string(), "openai".to_string()));
    }

    #[test]
    fn get_for_service_returns_value_when_name_equals_service() {
        let store = store_with(&[("github", "github", "ghp_default")]);
        assert_eq!(
            store.get_for_service("github").as_deref(),
            Some("ghp_default")
        );
    }

    #[test]
    fn get_for_service_returns_none_when_name_differs_from_service() {
        // name "gh-prod" != service "github" — no default
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert!(store.get_for_service("github").is_none());
    }

    #[test]
    fn get_for_service_returns_none_when_service_field_mismatches() {
        // name == "github" but service is "openai" — verify field check
        let store = store_with(&[("github", "openai", "sk-wrong")]);
        assert!(store.get_for_service("github").is_none());
    }

    #[test]
    fn service_of_returns_service_for_registered_name() {
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert_eq!(store.service_of("gh-prod").as_deref(), Some("github"));
    }

    #[test]
    fn service_of_returns_none_for_absent_name() {
        let store = CredStore::default();
        assert!(store.service_of("missing").is_none());
    }

    #[test]
    fn get_checked_returns_value_when_service_matches() {
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert_eq!(
            store.get_checked("gh-prod", "github").as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn get_checked_returns_none_on_service_mismatch() {
        let store = store_with(&[("gh-prod", "github", "ghp_secret")]);
        assert!(store.get_checked("gh-prod", "openai").is_none());
    }

    #[test]
    fn get_checked_returns_none_for_absent_name() {
        let store = CredStore::default();
        assert!(store.get_checked("missing", "github").is_none());
    }
}
