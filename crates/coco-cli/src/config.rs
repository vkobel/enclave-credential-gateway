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
        let mut config: Config = toml::from_str(&data).context("Failed to parse config")?;
        for entry in config.tokens.values_mut() {
            if entry.scope.is_empty() && !entry.all_routes {
                entry.all_routes = true;
            }
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let data = toml::to_string_pretty(self)?;
        write_secret_file(&path, data)?;
        Ok(())
    }
}
