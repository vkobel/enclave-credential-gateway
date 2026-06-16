use crate::secure_file::write_secret_file;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenEntry {
    pub token: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub all_routes: bool,
}

impl TokenEntry {
    pub fn allows_route(&self, route: &str) -> bool {
        self.all_routes || self.scope.iter().any(|scoped_route| scoped_route == route)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AttestationConfig {
    pub pcr0: String,
    pub pcr1: String,
    pub pcr2: String,
    pub base_url: Option<String>,
}

fn default_e2e() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub gateway_url: String,
    pub admin_token: Option<String>,
    #[serde(default)]
    pub tokens: HashMap<String, TokenEntry>,
    #[serde(default = "default_e2e")]
    pub e2e: bool,
    #[serde(default)]
    pub attestation: Option<AttestationConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gateway_url: String::new(),
            admin_token: None,
            tokens: HashMap::new(),
            e2e: true,
            attestation: None,
        }
    }
}

impl Config {
    const DEFAULT_GATEWAY_URL: &'static str = "https://localhost";

    pub fn config_dir() -> PathBuf {
        #[cfg(test)]
        if let Some(root) = crate::test_support::config_root_override() {
            return root;
        }

        Self::home_dir().join(".config").join("gate")
    }

    pub fn path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn generated_dir() -> PathBuf {
        Self::config_dir().join("generated")
    }

    pub(crate) fn expand_home(path: &str) -> PathBuf {
        if path == "~" {
            return Self::home_dir();
        }
        if let Some(rest) = path.strip_prefix("~/") {
            return Self::home_dir().join(rest);
        }
        PathBuf::from(path)
    }

    fn home_dir() -> PathBuf {
        #[cfg(test)]
        if let Some(home) = crate::test_support::home_dir_override() {
            return home;
        }

        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn load() -> Result<Config> {
        let path = Self::path();
        if !path.exists() {
            let config = Config {
                gateway_url: Self::DEFAULT_GATEWAY_URL.to_string(),
                ..Config::default()
            };
            config.save().with_context(|| {
                format!("Failed to create default config at {}", path.display())
            })?;
            return Ok(config);
        }
        let data = std::fs::read_to_string(&path).context("Failed to read config")?;
        let mut config: Config = toml::from_str(&data).context("Failed to parse config")?;
        config.apply_env_overrides();
        for entry in config.tokens.values_mut() {
            if entry.scope.is_empty() && !entry.all_routes {
                entry.all_routes = true;
            }
        }
        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(url) = std::env::var("GATEWAY_URL") {
            if !url.trim().is_empty() {
                self.gateway_url = url;
            }
        }
        if let Ok(token) = std::env::var("GATE_ADMIN_TOKEN") {
            if !token.trim().is_empty() {
                self.admin_token = Some(token);
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let data = toml::to_string_pretty(self)?;
        write_secret_file(&path, data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use crate::test_support::with_temp_config_root;

    #[test]
    fn load_creates_default_config_when_missing() {
        with_temp_config_root(|temp| {
            let config = Config::load().unwrap();

            assert_eq!(config.gateway_url, "https://localhost");
            assert!(config.admin_token.is_none());
            assert!(config.tokens.is_empty());

            let path = temp.path().join(".config/gate/config.toml");
            let contents = std::fs::read_to_string(path).unwrap();
            assert!(contents.contains("gateway_url = \"https://localhost\""));
        });
    }

    #[test]
    fn load_applies_env_overrides() {
        with_temp_config_root(|_temp| {
            let config = Config {
                gateway_url: "https://from-file.example".to_string(),
                ..Config::default()
            };
            config.save().unwrap();

            std::env::set_var("GATEWAY_URL", "https://from-env.example");
            std::env::set_var("GATE_ADMIN_TOKEN", "env-admin");
            let loaded = Config::load().unwrap();
            std::env::remove_var("GATEWAY_URL");
            std::env::remove_var("GATE_ADMIN_TOKEN");

            assert_eq!(loaded.gateway_url, "https://from-env.example");
            assert_eq!(loaded.admin_token.as_deref(), Some("env-admin"));
        });
    }

    #[test]
    fn e2e_defaults_to_true_when_field_missing() {
        let toml = r#"gateway_url = "https://example.com""#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.e2e);
    }

    #[test]
    fn e2e_can_be_disabled() {
        let toml = r#"
gateway_url = "https://example.com"
e2e = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.e2e);
    }

    #[test]
    fn legacy_config_without_new_fields_parses() {
        let toml = r#"
gateway_url = "https://example.com"
admin_token = "tok_abc"

[tokens.my-token]
token = "gate_xyz"
scope = ["openai"]
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.gateway_url, "https://example.com");
        assert_eq!(config.admin_token.as_deref(), Some("tok_abc"));
        assert!(config.e2e);
        assert!(config.attestation.is_none());
        assert!(config.tokens.contains_key("my-token"));
    }

    #[test]
    fn attestation_section_parses() {
        let toml = r#"
gateway_url = "https://example.com"

[attestation]
pcr0 = "aabbcc"
pcr1 = "ddeeff"
pcr2 = "112233"
base_url = "https://attest.example.com"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let att = config.attestation.unwrap();
        assert_eq!(att.pcr0, "aabbcc");
        assert_eq!(att.pcr1, "ddeeff");
        assert_eq!(att.pcr2, "112233");
        assert_eq!(att.base_url.as_deref(), Some("https://attest.example.com"));
    }

    #[test]
    fn default_config_has_e2e_true() {
        let config = Config::default();
        assert!(config.e2e);
    }
}
