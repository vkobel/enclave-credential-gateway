use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenEntry {
    pub token: String,
    #[serde(default)]
    pub scope: Vec<String>,
}

impl TokenEntry {
    pub fn allows_route(&self, route: &str) -> bool {
        self.scope.is_empty() || self.scope.iter().any(|scoped_route| scoped_route == route)
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub gateway_url: String,
    pub admin_token: Option<String>,
    #[serde(default)]
    pub tokens: HashMap<String, TokenEntry>,
}

impl Config {
    pub fn config_dir() -> PathBuf {
        #[cfg(test)]
        if let Some(root) = crate::test_support::config_root_override() {
            return root;
        }

        Self::home_dir().join(".config").join("coco")
    }

    pub fn path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn tools_path() -> PathBuf {
        Self::config_dir().join("tools.toml")
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
            anyhow::bail!(
                "Config not found at {}. Run: mkdir -p ~/.config/coco && create config.toml",
                path.display()
            );
        }
        let data = std::fs::read_to_string(&path).context("Failed to read config")?;
        let config: Config = toml::from_str(&data).context("Failed to parse config")?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = toml::to_string_pretty(self)?;
        std::fs::write(&path, data)?;
        Ok(())
    }
}
