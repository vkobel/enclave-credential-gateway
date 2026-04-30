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

/// Unit tests for `extract_candidate_tokens` covering Bearer/token/Basic
/// schemes, mixed-case token preservation, and malformed inputs.
mod extract_candidate_tokens_tests {
    use axum::{body::Body, http::Request};
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use coco_gateway::auth::extract_candidate_tokens;

    fn req_with_auth(value: &str) -> Request<Body> {
        Request::builder()
            .uri("/")
            .header("authorization", value)
            .body(Body::empty())
            .unwrap()
    }

    fn b64(s: &str) -> String {
        STANDARD.encode(s)
    }

    #[test]
    fn bearer_extracts_token() {
        let r = req_with_auth("Bearer ccgw_abc");
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"ccgw_abc".to_string()));
    }

    #[test]
    fn token_scheme_extracts_token() {
        let r = req_with_auth("token gh-pat");
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"gh-pat".to_string()));
    }

    #[test]
    fn bearer_preserves_mixed_case() {
        let r = req_with_auth("Bearer ccgw_AbCdEf");
        let cands = extract_candidate_tokens(&r);
        assert_eq!(cands, vec!["ccgw_AbCdEf".to_string()]);
    }

    #[test]
    fn token_scheme_preserves_mixed_case() {
        let r = req_with_auth("Token Gh_AbCdEf");
        let cands = extract_candidate_tokens(&r);
        assert_eq!(cands, vec!["Gh_AbCdEf".to_string()]);
    }

    #[test]
    fn basic_decodes_token_in_password_slot() {
        let r = req_with_auth(&format!("Basic {}", b64("x-access-token:ccgw_AbC")));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"ccgw_AbC".to_string()));
    }

    #[test]
    fn basic_decodes_token_in_username_slot() {
        let r = req_with_auth(&format!("Basic {}", b64("ccgw_AbC:x-oauth-basic")));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"ccgw_AbC".to_string()));
    }

    #[test]
    fn basic_preserves_mixed_case() {
        let r = req_with_auth(&format!("Basic {}", b64("x:ccgw_AbCdEf")));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"ccgw_AbCdEf".to_string()));
    }

    #[test]
    fn basic_pushes_both_halves() {
        let r = req_with_auth(&format!("Basic {}", b64("oauth2:ccgw_xyz")));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"oauth2".to_string()));
        assert!(cands.contains(&"ccgw_xyz".to_string()));
    }

    #[test]
    fn basic_no_colon_pushes_whole_string() {
        let r = req_with_auth(&format!("Basic {}", b64("just-a-token")));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.contains(&"just-a-token".to_string()));
    }

    #[test]
    fn basic_malformed_b64_does_not_panic() {
        let r = req_with_auth("Basic !!!not-base64!!!");
        let cands = extract_candidate_tokens(&r);
        assert!(cands.is_empty());
    }

    #[test]
    fn basic_non_utf8_payload_is_skipped() {
        // Valid base64 of bytes that are not valid UTF-8.
        let payload = STANDARD.encode([0xff, 0xfe, 0xfd]);
        let r = req_with_auth(&format!("Basic {}", payload));
        let cands = extract_candidate_tokens(&r);
        assert!(cands.is_empty());
    }

    #[test]
    fn unknown_scheme_yields_no_candidate() {
        let r = req_with_auth("Digest something");
        let cands = extract_candidate_tokens(&r);
        assert!(cands.is_empty());
    }

    #[test]
    fn duplicate_candidates_are_deduplicated() {
        let r = Request::builder()
            .uri("/")
            .header("authorization", "Bearer ccgw_dup")
            .header("x-other", "Bearer ccgw_dup")
            .body(Body::empty())
            .unwrap();
        let cands = extract_candidate_tokens(&r);
        assert_eq!(cands.iter().filter(|c| c.as_str() == "ccgw_dup").count(), 1);
    }
}

/// Profile loading and schema tests
mod profile_tests {
    use coco_gateway::profile::{load_embedded_routes, try_load_routes_from_str};
    use coco_gateway::{InjectMode, ProfileRoute, RouteEntry, RouteMatcher};

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
        let json = r##"{
            "upstream": "http://test",
            "credential_sources": [{"env": "TEST_TOKEN", "inject_header": "Authorization"}]
        }"##;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.inject_mode, InjectMode::Header);
        assert_eq!(route.credential_sources[0].format, "Bearer {}");
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
    fn test_alias_deserialization() {
        let json = r#"{
            "upstream": "https://api.github.com",
            "aliases": [{"prefix": "api", "strip_prefix": "/v3"}],
            "credential_sources": [{"env": "GITHUB_TOKEN", "inject_header": "Authorization", "format": "Bearer {}"}]
        }"#;
        let route: ProfileRoute = serde_json::from_str(json).unwrap();
        assert_eq!(route.aliases.len(), 1);
        assert_eq!(route.aliases[0].prefix, "api");
        assert_eq!(route.aliases[0].strip_prefix.as_deref(), Some("/v3"));
    }

    #[test]
    fn test_embedded_routes_expand_github_compat_alias() {
        let routes = load_embedded_routes();
        let github = routes.iter().find(|(key, _)| key == "github").unwrap();
        let api = routes.iter().find(|(key, _)| key == "api").unwrap();

        assert_eq!(github.1.canonical_route, "github");
        assert_eq!(github.1.strip_prefix.as_deref(), None);
        assert_eq!(api.1.canonical_route, "github");
        assert_eq!(api.1.upstream, github.1.upstream);
        assert_eq!(
            api.1.credential_sources.len(),
            github.1.credential_sources.len()
        );
        assert_eq!(api.1.strip_prefix.as_deref(), Some("/v3"));
    }

    #[test]
    fn test_embedded_manifest_has_no_top_level_api_route() {
        let manifest: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../../../profiles/coco.yaml")).unwrap();
        let routes = manifest["routes"].as_mapping().unwrap();
        assert!(routes.contains_key(serde_yaml::Value::String("github".to_string())));
        assert!(!routes.contains_key(serde_yaml::Value::String("api".to_string())));

        let aliases = routes["github"]["aliases"].as_sequence().unwrap();
        assert_eq!(aliases[0]["prefix"].as_str(), Some("api"));
        assert_eq!(aliases[0]["strip_prefix"].as_str(), Some("/v3"));
    }

    #[test]
    fn test_embedded_github_compat_route_strips_v3() {
        let routes = load_embedded_routes();
        let api = routes.iter().find(|(key, _)| key == "api").unwrap();

        assert_eq!(api.1.canonical_route, "github");
        assert_eq!(api.1.strip_prefix.as_deref(), Some("/v3"));

        let path = "/api/v3/";
        let prefix = "api";
        let upstream_path = &path[prefix.len() + 1..];
        let stripped = upstream_path
            .strip_prefix(api.1.strip_prefix.as_deref().unwrap())
            .unwrap_or(upstream_path);
        assert_eq!(stripped, "/");
    }

    #[test]
    fn test_embedded_github_expands_into_api_and_git_routes() {
        let routes = load_embedded_routes();
        let github = routes.iter().find(|(key, _)| key == "github").unwrap();
        let git = routes
            .iter()
            .find(|(_, e)| e.matcher == RouteMatcher::GitSmartHttp && e.canonical_route == "github")
            .expect("expected an expanded git_protocol entry for github");

        assert_eq!(github.1.matcher, RouteMatcher::Prefix);
        assert_eq!(github.1.upstream, "https://api.github.com");
        assert_eq!(git.1.upstream, "https://github.com");
        assert_eq!(git.1.canonical_route, "github");
        assert_eq!(
            git.1.credential_sources.len(),
            github.1.credential_sources.len()
        );
        assert!(git.1.strip_prefix.is_none());
    }

    #[test]
    fn test_git_protocol_field_optional_for_other_routes() {
        let routes = load_embedded_routes();
        for (_, entry) in &routes {
            if entry.canonical_route != "github" {
                assert_eq!(
                    entry.matcher,
                    RouteMatcher::Prefix,
                    "non-github routes should not have git_protocol matcher"
                );
            }
        }
        // Sanity: at least one non-github route exists and loads cleanly.
        assert!(routes.iter().any(|(k, _)| k == "openai"));
        assert!(routes.iter().any(|(k, _)| k == "anthropic"));
        let ollama = routes.iter().find(|(k, _)| k == "ollama").unwrap();
        assert_eq!(ollama.1.upstream, "https://ollama.com");
    }

    #[test]
    fn test_git_protocol_deserializes_from_profile() {
        let routes = try_load_routes_from_str(
            "test",
            r#"{
                "routes": {
                    "gh": {
                        "upstream": "https://api.example.com",
                        "git_protocol": { "upstream": "https://example.com" },
                        "credential_sources": [
                            {"env": "GH_TOKEN", "inject_header": "Authorization"}
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let prefix_entry = routes
            .iter()
            .find(|(_, e)| e.canonical_route == "gh" && e.matcher == RouteMatcher::Prefix)
            .unwrap();
        let git_entry = routes
            .iter()
            .find(|(_, e)| e.canonical_route == "gh" && e.matcher == RouteMatcher::GitSmartHttp)
            .unwrap();
        assert_eq!(prefix_entry.1.upstream, "https://api.example.com");
        assert_eq!(git_entry.1.upstream, "https://example.com");
    }

    #[test]
    fn test_profile_routes_are_returned_in_deterministic_order() {
        let routes = try_load_routes_from_str(
            "test",
            r#"{
                "routes": {
                    "zeta": {
                        "upstream": "https://z.example",
                        "credential_sources": [{"env": "Z_TOKEN", "inject_header": "Authorization"}]
                    },
                    "alpha": {
                        "upstream": "https://a.example",
                        "credential_sources": [{"env": "A_TOKEN", "inject_header": "Authorization"}]
                    }
                }
            }"#,
        )
        .unwrap();

        let keys: Vec<_> = routes.into_iter().map(|(key, _)| key).collect();
        assert_eq!(keys, vec!["alpha", "zeta"]);
    }
}

/// `is_git_smart_http` matcher tests.
mod git_matcher_tests {
    use coco_gateway::is_git_smart_http;

    #[test]
    fn matches_info_refs() {
        assert!(is_git_smart_http("/octocat/hello.git/info/refs"));
    }

    #[test]
    fn matches_git_upload_pack() {
        assert!(is_git_smart_http("/octocat/hello.git/git-upload-pack"));
    }

    #[test]
    fn matches_git_receive_pack() {
        assert!(is_git_smart_http("/octocat/hello.git/git-receive-pack"));
    }

    #[test]
    fn rejects_dumb_http_objects_path() {
        assert!(!is_git_smart_http(
            "/octocat/hello.git/objects/pack/pack-abc.idx"
        ));
    }

    #[test]
    fn rejects_api_path() {
        assert!(!is_git_smart_http("/api/v3/repos/foo/bar"));
    }

    #[test]
    fn rejects_single_segment() {
        assert!(!is_git_smart_http("/octocat.git/info/refs"));
    }

    #[test]
    fn rejects_too_many_segments() {
        assert!(!is_git_smart_http("/a/b/c.git/info/refs"));
    }

    #[test]
    fn rejects_no_dot_git() {
        assert!(!is_git_smart_http("/octocat/hello/info/refs"));
    }

    #[test]
    fn rejects_empty_owner() {
        assert!(!is_git_smart_http("//hello.git/info/refs"));
    }

    #[test]
    fn rejects_empty_repo() {
        assert!(!is_git_smart_http("/octocat/.git/info/refs"));
    }
}

/// Tests for `resolve_route` covering both the prefix matcher and the new
/// git-smart-http matcher.
mod resolver_tests {
    use coco_gateway::profile::{load_embedded_routes, try_load_routes_from_str};
    use coco_gateway::{resolve_route, RouteMatcher};

    #[test]
    fn resolve_prefix_match_returns_existing_entry() {
        let routes = load_embedded_routes();
        let r = resolve_route(&routes, "/openai/v1/chat").unwrap();
        assert_eq!(r.entry.canonical_route, "openai");
        assert_eq!(r.upstream_path, "/v1/chat");
        assert_eq!(r.entry.matcher, RouteMatcher::Prefix);
    }

    #[test]
    fn resolve_prefix_match_strips_alias_v3() {
        let routes = load_embedded_routes();
        let r = resolve_route(&routes, "/api/v3/user").unwrap();
        assert_eq!(r.entry.canonical_route, "github");
        assert_eq!(r.upstream_path, "/user");
    }

    #[test]
    fn resolve_prefix_match_with_root_path() {
        let routes = load_embedded_routes();
        let r = resolve_route(&routes, "/openai").unwrap();
        assert_eq!(r.upstream_path, "/");
    }

    #[test]
    fn resolve_git_smart_http_uses_full_path_and_github_upstream() {
        let routes = load_embedded_routes();
        let r = resolve_route(&routes, "/vkobel/hello-attested.git/info/refs").unwrap();
        assert_eq!(r.entry.canonical_route, "github");
        assert_eq!(r.entry.matcher, RouteMatcher::GitSmartHttp);
        assert_eq!(r.entry.upstream, "https://github.com");
        assert_eq!(r.upstream_path, "/vkobel/hello-attested.git/info/refs");
    }

    #[test]
    fn resolve_unknown_path_returns_none() {
        let routes = load_embedded_routes();
        assert!(resolve_route(&routes, "/no-such-route/x").is_none());
        assert!(resolve_route(&routes, "/vkobel/hello-attested").is_none());
    }

    #[test]
    fn resolve_does_not_match_synthetic_git_key_via_prefix() {
        // The synthetic `__git__github` key must not be reachable via the
        // prefix path; only the GitSmartHttp matcher should match.
        let routes = load_embedded_routes();
        assert!(resolve_route(&routes, "/__git__github/foo").is_none());
    }

    #[test]
    fn resolve_skips_unknown_matcher_for_prefix_path() {
        // Synthetic profile where a key collides with a non-prefix matcher.
        // Only GitSmartHttp can satisfy the git matcher; an unrelated path
        // must not accidentally match it.
        let routes = try_load_routes_from_str(
            "test",
            r#"{
                "routes": {
                    "gh": {
                        "upstream": "https://api.example.com",
                        "git_protocol": { "upstream": "https://example.com" },
                        "credential_sources": [
                            {"env": "GH_TOKEN", "inject_header": "Authorization"}
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let r = resolve_route(&routes, "/gh/anything").unwrap();
        assert_eq!(r.entry.upstream, "https://api.example.com");

        let r = resolve_route(&routes, "/foo/bar.git/info/refs").unwrap();
        assert_eq!(r.entry.upstream, "https://example.com");
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
        let (record, token_value) = registry
            .create_token("test".to_string(), vec![], true)
            .await;

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
        registry
            .create_token("test".to_string(), vec![], true)
            .await;

        let validated = registry.validate("ccgw_wrong").await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn test_revoked_token_fails_validation() {
        let (registry, _dir) = create_registry().await;
        let (record, token_value) = registry
            .create_token("test".to_string(), vec![], true)
            .await;

        assert!(registry.validate(&token_value).await.is_some());
        assert!(registry.revoke_token(record.id).await);

        let validated = registry.validate(&token_value).await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn test_list_tokens_omits_token_value() {
        let (registry, _dir) = create_registry().await;
        registry
            .create_token("laptop".to_string(), vec!["anthropic".to_string()], false)
            .await;
        registry.create_token("ci".to_string(), vec![], true).await;

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
            let (_, token) = registry
                .create_token("persist".to_string(), vec![], true)
                .await;
            token
        };

        let registry2 = TokenRegistry::load_or_create(path).await.unwrap();
        assert!(registry2.validate(&token_value).await.is_some());
    }

    #[tokio::test]
    async fn test_scope_enforcement() {
        let (registry, _dir) = create_registry().await;
        let (_record, scoped_token) = registry
            .create_token("scoped".to_string(), vec!["httpbin".to_string()], false)
            .await;

        let validated = registry.validate(&scoped_token).await.unwrap();
        assert_eq!(validated.scope, vec!["httpbin"]);
        assert!(!validated.is_all_routes());
        assert!(validated.allows_route("httpbin"));
        assert!(!validated.allows_route("anthropic"));
    }

    #[tokio::test]
    async fn test_empty_scope_allows_all_routes() {
        let (registry, _dir) = create_registry().await;
        let (_record, token) = registry.create_token("all".to_string(), vec![], true).await;

        let validated = registry.validate(&token).await.unwrap();
        assert!(validated.is_all_routes());
        assert!(validated.allows_route("httpbin"));
        assert!(validated.allows_route("future-route"));
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
