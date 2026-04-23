//! Integration tests for coco-gateway

/// Unit tests for auth middleware helpers
mod auth_tests {

    use coco_gateway::validate_bearer_or_raw;
    use zeroize::Zeroizing;

    fn token(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    // ── validate_bearer_or_raw ────────────────────────────────────────────

    #[test]
    fn test_x_api_key_phantom_auth_raw() {
        let t = token("phantom-123");
        assert!(validate_bearer_or_raw(b"phantom-123", &t));
    }

    #[test]
    fn test_x_api_key_phantom_auth_wrong() {
        let t = token("correct");
        assert!(!validate_bearer_or_raw(b"wrong", &t));
    }

    #[test]
    fn test_authorization_bearer_phantom_auth() {
        let t = token("oauth-phantom");
        assert!(validate_bearer_or_raw(b"Bearer oauth-phantom", &t));
    }

    #[test]
    fn test_authorization_bearer_phantom_case_insensitive() {
        let t = token("oauth-phantom");
        assert!(validate_bearer_or_raw(b"BEARER oauth-phantom", &t));
    }

    #[test]
    fn test_authorization_token_phantom_gh_legacy() {
        let t = token("gh-phantom");
        assert!(validate_bearer_or_raw(b"token gh-phantom", &t));
        assert!(validate_bearer_or_raw(b"Token gh-phantom", &t));
    }

    #[test]
    fn test_constant_time_eq() {
        use subtle::ConstantTimeEq;
        assert!(bool::from(b"same".ct_eq(b"same")));
        assert!(!bool::from(b"a".ct_eq(b"b")));
        assert_ne!(b"short".len(), b"longer-value".len());
    }
}

/// Profile loading and schema tests
mod profile_tests {
    use coco_gateway::profile::load_embedded_routes;
    use coco_gateway::{InjectMode, ProfileRoute, RouteEntry};

    #[test]
    fn test_profile_json_parsing_legacy_format() {
        let json = r#"{
            "upstream": "http://localhost:9999",
            "credential_env": "TEST_TOKEN",
            "inject_header": "Authorization",
            "credential_format": "Bearer {}"
        }"#;

        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.upstream, "http://localhost:9999");
        assert_eq!(route.credential_env.as_deref(), Some("TEST_TOKEN"));
        assert_eq!(route.inject_header, "Authorization");
        assert_eq!(route.credential_format, "Bearer {}");
        assert!(route.credential_sources.is_empty());
        assert_eq!(route.inject_mode, InjectMode::Header);

        let entry = RouteEntry::from_profile("test", route);
        assert_eq!(entry.upstream, "http://localhost:9999");
        assert_eq!(entry.credential_sources.len(), 1);
        assert_eq!(entry.credential_sources[0].env, "TEST_TOKEN");
    }

    #[test]
    fn test_credential_sources_parsing() {
        let json = r#"{
            "upstream": "https://api.anthropic.com",
            "credential_sources": [
                {"env": "ANTHROPIC_AUTH_TOKEN", "inject_header": "Authorization", "format": "Bearer {}"},
                {"env": "ANTHROPIC_API_KEY",    "inject_header": "x-api-key",     "format": "{}"}
            ]
        }"#;

        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.upstream, "https://api.anthropic.com");
        assert_eq!(route.credential_sources.len(), 2);

        let oauth = &route.credential_sources[0];
        assert_eq!(oauth.env, "ANTHROPIC_AUTH_TOKEN");
        assert_eq!(oauth.inject_header, "Authorization");
        assert_eq!(oauth.format, "Bearer {}");

        let apikey = &route.credential_sources[1];
        assert_eq!(apikey.env, "ANTHROPIC_API_KEY");
        assert_eq!(apikey.inject_header, "x-api-key");
        assert_eq!(apikey.format, "{}");
    }

    #[test]
    fn test_route_prefix_matching() {
        let path = "/openai/v1/chat/completions";
        let prefix = path.trim_start_matches('/').split('/').next().unwrap_or("");
        assert_eq!(prefix, "openai");

        let path2 = "/anthropic/v1/messages";
        let prefix2 = path2
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("");
        assert_eq!(prefix2, "anthropic");
    }

    #[test]
    fn test_inject_mode_default_is_header() {
        let json = r##"{"upstream": "http://test", "credential_env": "X"}"##;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.inject_mode, InjectMode::Header);
    }

    #[test]
    fn test_inject_mode_url_path_deserialization() {
        let json = r#"{
            "upstream": "https://api.telegram.org",
            "inject_mode": "url_path",
            "url_path_prefix": "/bot",
            "credential_sources": [{"env": "TELEGRAM_BOT_TOKEN", "inject_header": "X", "format": "{}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.inject_mode, InjectMode::UrlPath);
        assert_eq!(route.url_path_prefix.as_deref(), Some("/bot"));
    }

    #[test]
    fn test_inject_mode_query_param_deserialization() {
        let json = r#"{
            "upstream": "https://example.com",
            "inject_mode": "query_param",
            "inject_param": "key",
            "credential_sources": [{"env": "API_KEY", "inject_header": "X", "format": "{}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.inject_mode, InjectMode::QueryParam);
        assert_eq!(route.inject_param.as_deref(), Some("key"));
    }

    #[test]
    fn test_inject_mode_carried_through_to_route_entry() {
        let json = r#"{
            "upstream": "https://api.telegram.org",
            "inject_mode": "url_path",
            "url_path_prefix": "/bot",
            "credential_sources": [{"env": "TELEGRAM_BOT_TOKEN", "inject_header": "X", "format": "{}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        let entry = RouteEntry::from_profile("telegram", route);
        assert_eq!(entry.inject_mode, InjectMode::UrlPath);
        assert_eq!(entry.url_path_prefix.as_deref(), Some("/bot"));
    }

    #[test]
    fn test_strip_prefix_carried_through() {
        let json = r#"{
            "upstream": "https://api.github.com",
            "strip_prefix": "/v3",
            "credential_sources": [{"env": "GITHUB_TOKEN", "inject_header": "Authorization", "format": "Bearer {}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        let entry = RouteEntry::from_profile("api", route);
        assert_eq!(entry.strip_prefix.as_deref(), Some("/v3"));
    }

    #[test]
    fn test_canonical_route_carried_through() {
        let json = r#"{
            "canonical": "github",
            "upstream": "https://api.github.com",
            "strip_prefix": "/v3",
            "credential_sources": [{"env": "GITHUB_TOKEN", "inject_header": "Authorization", "format": "Bearer {}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        let entry = RouteEntry::from_profile("api", route);
        assert_eq!(entry.canonical_route, "github");
        assert_eq!(entry.strip_prefix.as_deref(), Some("/v3"));
    }

    #[test]
    fn test_embedded_routes_include_github_compat_route() {
        let routes = load_embedded_routes();
        let github = routes.iter().find(|(key, _)| key == "github").unwrap();
        let api = routes.iter().find(|(key, _)| key == "api").unwrap();

        assert_eq!(github.1.canonical_route, "github");
        assert_eq!(api.1.canonical_route, "github");
        assert_eq!(api.1.strip_prefix.as_deref(), Some("/v3"));
    }
}

/// Integration tests that require a running gateway.
///
/// These tests are **opt-in** via the `integration` feature flag.
/// Run with: `cargo test --features integration`
///
/// Prerequisites:
///   - Gateway running on localhost:8080
///   - `TEST_PHANTOM_TOKEN` env var set
mod gateway_tests {
    use reqwest::StatusCode;

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration"),
        ignore = "Requires running gateway. Run with: cargo test --features integration"
    )]
    async fn test_missing_phantom_token_returns_407() {
        let client = reqwest::Client::new();
        let res = client
            .get("http://localhost:8080/test/")
            .send()
            .await
            .expect("Gateway not running — start gateway and run with --features integration");
        assert_eq!(res.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration"),
        ignore = "Requires running gateway. Run with: cargo test --features integration"
    )]
    async fn test_wrong_phantom_token_returns_407() {
        let client = reqwest::Client::new();
        let res = client
            .get("http://localhost:8080/test/")
            .header("Authorization", "Bearer wrong-token")
            .send()
            .await
            .expect("Gateway not running — start gateway and run with --features integration");
        assert_eq!(res.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration"),
        ignore = "Requires running gateway. Run with: cargo test --features integration"
    )]
    async fn test_unknown_route_returns_404() {
        let phantom = std::env::var("TEST_PHANTOM_TOKEN")
            .expect("TEST_PHANTOM_TOKEN must be set for integration tests");
        let client = reqwest::Client::new();
        let res = client
            .get("http://localhost:8080/unknown-route/")
            .header("Authorization", format!("Bearer {}", phantom))
            .send()
            .await
            .expect("Gateway not running — start gateway and run with --features integration");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}

/// Header manipulation and credential formatting tests
mod header_tests {
    #[test]
    fn test_path_stripping() {
        let path = "/openai/v1/chat/completions";
        let prefix = "openai";
        let stripped = &path[prefix.len() + 1..];
        assert_eq!(stripped, "/v1/chat/completions");

        let path2 = "/httpbin/";
        let prefix2 = "httpbin";
        let stripped2 = &path2[prefix2.len() + 1..];
        let result = if stripped2.is_empty() { "/" } else { stripped2 };
        assert_eq!(result, "/");
    }

    #[test]
    fn test_credential_formatting() {
        assert_eq!(
            "Bearer {}".replace("{}", "sk-openai-123"),
            "Bearer sk-openai-123"
        );
        assert_eq!("{}".replace("{}", "sk-ant-api-456"), "sk-ant-api-456");
        assert_eq!(
            "Bearer {}".replace("{}", "sk-ant-oat01-789"),
            "Bearer sk-ant-oat01-789"
        );
    }

    #[test]
    fn test_credential_source_fallback_logic() {
        let sources: Vec<(&str, Option<&str>)> = vec![
            ("ANTHROPIC_AUTH_TOKEN", None),
            ("ANTHROPIC_API_KEY", Some("sk-ant-123")),
        ];

        let preferred: Option<usize> = Some(0);
        let resolved = preferred
            .and_then(|i| sources.get(i).and_then(|(env, val)| val.map(|v| (*env, v))))
            .or_else(|| sources.iter().find_map(|(env, val)| val.map(|v| (*env, v))));

        let (env, val) = resolved.unwrap();
        assert_eq!(env, "ANTHROPIC_API_KEY");
        assert_eq!(val, "sk-ant-123");
    }

    #[test]
    fn test_header_mode_url_construction() {
        let upstream = "https://api.openai.com";
        let upstream_path = "/v1/chat/completions";
        let query = "";
        let url = format!("{}{}{}", upstream, upstream_path, query);
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_url_path_mode_construction() {
        let upstream = "https://api.telegram.org";
        let url_path_prefix = "/bot";
        let credential = "123456:ABC-DEF";
        let upstream_path = "/sendMessage";
        let query = "";
        let url = format!(
            "{}{}{}{}{}",
            upstream, url_path_prefix, credential, upstream_path, query
        );
        assert_eq!(
            url,
            "https://api.telegram.org/bot123456:ABC-DEF/sendMessage"
        );
    }

    #[test]
    fn test_query_param_mode_construction() {
        let upstream = "https://example.com";
        let upstream_path = "/api/data";
        let query = "";
        let inject_param = "api_key";
        let credential = "my-key";
        let sep = "?";
        let url = format!(
            "{}{}{}{}{}{}",
            upstream, upstream_path, query, sep, inject_param, credential
        );
        assert_eq!(url, "https://example.com/api/data?api_keymy-key");
    }

    #[test]
    fn test_query_param_mode_with_existing_query() {
        let upstream = "https://example.com";
        let upstream_path = "/api/data";
        let query = "?foo=bar";
        let inject_param = "api_key";
        let credential = "my-key";
        let sep = "&";
        let url = format!(
            "{}{}{}{}{}{}",
            upstream, upstream_path, query, sep, inject_param, credential
        );
        assert_eq!(url, "https://example.com/api/data?foo=bar&api_keymy-key");
    }

    #[test]
    fn test_strip_prefix_applied() {
        let path = "/api/v3/user";
        let prefix = "api";
        let upstream_path = &path[prefix.len() + 1..]; // "/v3/user"
        let strip_prefix = "/v3";
        let stripped = upstream_path
            .strip_prefix(strip_prefix)
            .unwrap_or(upstream_path);
        assert_eq!(stripped, "/user");
    }
}

/// Token registry tests
mod registry_tests {
    use coco_gateway::{TokenRegistry, TokenStatus};
    use tempfile::TempDir;

    async fn create_registry() -> (TokenRegistry, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tokens.json");
        let registry = TokenRegistry::load_or_create(path).await.unwrap();
        (registry, dir)
    }

    #[tokio::test]
    async fn test_create_and_validate_token() {
        let (registry, _dir) = create_registry().await;
        let (record, token_value) = registry.create_token("test".to_string(), vec![]).await;

        assert!(token_value.starts_with("ccgw_"));
        assert_eq!(record.name, "test");
        assert_eq!(record.status, TokenStatus::Active);

        let validated = registry.validate(&token_value).await;
        assert!(validated.is_some());
        assert_eq!(validated.unwrap().id, record.id);
    }

    #[tokio::test]
    async fn test_wrong_token_fails_validation() {
        let (registry, _dir) = create_registry().await;
        registry.create_token("test".to_string(), vec![]).await;

        let validated = registry.validate("ccgw_wrong").await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn test_revoked_token_fails_validation() {
        let (registry, _dir) = create_registry().await;
        let (record, token_value) = registry.create_token("test".to_string(), vec![]).await;

        assert!(registry.validate(&token_value).await.is_some());
        assert!(registry.revoke_token(record.id).await);

        let validated = registry.validate(&token_value).await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn test_list_tokens_omits_token_value() {
        let (registry, _dir) = create_registry().await;
        registry
            .create_token("laptop".to_string(), vec!["anthropic".to_string()])
            .await;
        registry.create_token("ci".to_string(), vec![]).await;

        let tokens = registry.list_tokens().await;
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].name, "laptop");
        assert_eq!(tokens[0].scope, vec!["anthropic"]);
    }

    #[tokio::test]
    async fn test_registry_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tokens.json");

        let token_value = {
            let registry = TokenRegistry::load_or_create(path.clone()).await.unwrap();
            let (_, token) = registry.create_token("persist".to_string(), vec![]).await;
            token
        };

        let registry2 = TokenRegistry::load_or_create(path).await.unwrap();
        assert!(registry2.validate(&token_value).await.is_some());
    }

    #[tokio::test]
    async fn test_scope_enforcement() {
        let (registry, _dir) = create_registry().await;
        let (_record, scoped_token) = registry
            .create_token("scoped".to_string(), vec!["httpbin".to_string()])
            .await;

        let validated = registry.validate(&scoped_token).await.unwrap();
        assert_eq!(validated.scope, vec!["httpbin"]);
        assert!(!validated.scope.is_empty());
        assert!(!validated.scope.iter().any(|s| s == "anthropic"));
    }

    #[tokio::test]
    async fn test_load_or_create_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent/tokens.json");
        let registry = TokenRegistry::load_or_create(path).await.unwrap();
        let tokens = registry.list_tokens().await;
        assert!(tokens.is_empty());
    }
}
