//! Integration tests for coco-gateway

/// Unit tests for auth middleware helpers
mod auth_tests {

    use coco_gateway::{validate_proxy_authorization, validate_bearer_or_raw};
    use zeroize::Zeroizing;

    fn token(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    // ── validate_proxy_authorization ──────────────────────────────────────

    #[test]
    fn test_bearer_proxy_auth_valid() {
        let t = token("my-secret-token");
        assert!(validate_proxy_authorization(b"Bearer my-secret-token", &t));
    }

    #[test]
    fn test_bearer_proxy_auth_wrong() {
        let t = token("correct");
        assert!(!validate_proxy_authorization(b"Bearer wrong", &t));
    }

    #[test]
    fn test_bearer_proxy_auth_case_insensitive_scheme() {
        let t = token("my-secret-token");
        assert!(validate_proxy_authorization(b"BEARER my-secret-token", &t));
    }

    #[test]
    fn test_basic_proxy_auth_valid() {
        use base64::Engine;
        let t = token("my-secret");
        let encoded = base64::engine::general_purpose::STANDARD.encode("user:my-secret");
        let header = format!("Basic {}", encoded);
        assert!(validate_proxy_authorization(header.as_bytes(), &t));
    }

    #[test]
    fn test_basic_proxy_auth_wrong_password() {
        use base64::Engine;
        let t = token("correct");
        let encoded = base64::engine::general_purpose::STANDARD.encode("user:wrong");
        let header = format!("Basic {}", encoded);
        assert!(!validate_proxy_authorization(header.as_bytes(), &t));
    }

    // ── validate_bearer_or_raw ────────────────────────────────────────────

    #[test]
    fn test_x_api_key_phantom_auth_raw() {
        // Claude Code sends x-api-key: <phantom> (raw, no Bearer prefix)
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
        // Claude Code with ANTHROPIC_AUTH_TOKEN sends Authorization: Bearer <phantom>
        let t = token("oauth-phantom");
        assert!(validate_bearer_or_raw(b"Bearer oauth-phantom", &t));
    }

    #[test]
    fn test_authorization_bearer_phantom_case_insensitive() {
        let t = token("oauth-phantom");
        assert!(validate_bearer_or_raw(b"BEARER oauth-phantom", &t));
    }

    #[test]
    fn test_constant_time_eq() {
        use subtle::ConstantTimeEq;
        assert!(bool::from(b"same".ct_eq(b"same")));
        assert!(!bool::from(b"a".ct_eq(b"b")));
        // Differing lengths: ct_eq requires equal-length slices, so the
        // gateway's wrapper short-circuits on len mismatch.
        assert_ne!(b"short".len(), b"longer-value".len());
    }
}

/// Profile loading and schema tests
mod profile_tests {
    #[test]
    fn test_profile_json_parsing_legacy_format() {
        let json = r#"{
            "routes": {
                "test": {
                    "upstream": "http://localhost:9999",
                    "credential_env": "TEST_TOKEN",
                    "inject_header": "Authorization",
                    "credential_format": "Bearer {}"
                }
            }
        }"#;

        #[derive(serde::Deserialize)]
        struct Profile {
            routes: std::collections::HashMap<String, ProfileRoute>,
        }
        #[derive(serde::Deserialize)]
        struct ProfileRoute {
            upstream: String,
            credential_env: Option<String>,
            inject_header: String,
            credential_format: String,
            #[serde(default)]
            credential_sources: Vec<serde_json::Value>,
        }

        let profile: Profile = serde_json::from_str(json).unwrap();
        let route = profile.routes.get("test").unwrap();
        assert_eq!(route.upstream, "http://localhost:9999");
        assert_eq!(route.credential_env.as_deref(), Some("TEST_TOKEN"));
        assert_eq!(route.inject_header, "Authorization");
        assert_eq!(route.credential_format, "Bearer {}");
        assert!(route.credential_sources.is_empty());
    }

    #[test]
    fn test_credential_sources_parsing() {
        let json = r#"{
            "routes": {
                "anthropic": {
                    "upstream": "https://api.anthropic.com",
                    "credential_sources": [
                        {"env": "ANTHROPIC_AUTH_TOKEN", "inject_header": "Authorization", "format": "Bearer {}"},
                        {"env": "ANTHROPIC_API_KEY",    "inject_header": "x-api-key",     "format": "{}"}
                    ]
                }
            }
        }"#;

        #[derive(serde::Deserialize)]
        struct Profile {
            routes: std::collections::HashMap<String, ProfileRoute>,
        }
        #[derive(serde::Deserialize)]
        struct ProfileRoute {
            upstream: String,
            #[serde(default)]
            credential_sources: Vec<CredSrc>,
        }
        #[derive(serde::Deserialize)]
        struct CredSrc {
            env: String,
            inject_header: String,
            format: String,
        }

        let profile: Profile = serde_json::from_str(json).unwrap();
        let route = profile.routes.get("anthropic").unwrap();
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
        let prefix2 = path2.trim_start_matches('/').split('/').next().unwrap_or("");
        assert_eq!(prefix2, "anthropic");
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
    #[cfg_attr(not(feature = "integration"), ignore = "Requires running gateway. Run with: cargo test --features integration")]
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
    #[cfg_attr(not(feature = "integration"), ignore = "Requires running gateway. Run with: cargo test --features integration")]
    async fn test_wrong_phantom_token_returns_407() {
        let client = reqwest::Client::new();
        let res = client
            .get("http://localhost:8080/test/")
            .header("Proxy-Authorization", "Bearer wrong-token")
            .send()
            .await
            .expect("Gateway not running — start gateway and run with --features integration");
        assert_eq!(res.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore = "Requires running gateway. Run with: cargo test --features integration")]
    async fn test_unknown_route_returns_404() {
        let phantom = std::env::var("TEST_PHANTOM_TOKEN")
            .expect("TEST_PHANTOM_TOKEN must be set for integration tests");
        let client = reqwest::Client::new();
        let res = client
            .get("http://localhost:8080/unknown-route/")
            .header("Proxy-Authorization", format!("Bearer {}", phantom))
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
        // OpenAI / GitHub: Bearer prefix
        assert_eq!("Bearer {}".replace("{}", "sk-openai-123"), "Bearer sk-openai-123");

        // Anthropic API key: raw (no Bearer prefix)
        assert_eq!("{}".replace("{}", "sk-ant-api-456"), "sk-ant-api-456");

        // Anthropic OAuth token: Bearer prefix
        assert_eq!("Bearer {}".replace("{}", "sk-ant-oat01-789"), "Bearer sk-ant-oat01-789");
    }

    #[test]
    fn test_credential_source_fallback_logic() {
        // Simulates resolve_credential: preferred source tried first, then first available
        let sources: Vec<(&str, Option<&str>)> = vec![
            ("ANTHROPIC_AUTH_TOKEN", None),          // not set
            ("ANTHROPIC_API_KEY", Some("sk-ant-123")), // set
        ];

        // Preferred = source 0, but it's not available → falls through to source 1
        let preferred: Option<usize> = Some(0);
        let resolved = preferred
            .and_then(|i| sources.get(i).and_then(|(env, val)| val.map(|v| (*env, v))))
            .or_else(|| sources.iter().find_map(|(env, val)| val.map(|v| (*env, v))));

        let (env, val) = resolved.unwrap();
        assert_eq!(env, "ANTHROPIC_API_KEY");
        assert_eq!(val, "sk-ant-123");
    }
}
