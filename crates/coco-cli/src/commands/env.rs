use crate::config::Config;
use anyhow::{Context, Result};

pub fn run(name: &str, codex: bool) -> Result<()> {
    let config = Config::load()?;
    for line in build_exports(&config, name)? {
        println!("{line}");
    }

    let entry = config
        .tokens
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Token '{}' not found in config", name))?;
    if codex && has(entry, "openai") {
        write_codex_config(config.gateway_url.trim_end_matches('/'), &entry.token)?;
    }

    Ok(())
}

fn build_exports(config: &Config, name: &str) -> Result<Vec<String>> {
    let entry = config
        .tokens
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Token '{}' not found in config", name))?;
    let base = config.gateway_url.trim_end_matches('/');
    let token = &entry.token;
    let mut exports = Vec::new();

    if has(entry, "anthropic") {
        exports.push(format!("export ANTHROPIC_BASE_URL={}/anthropic", base));
        exports.push(format!("export ANTHROPIC_API_KEY={}", token));
    }
    if has(entry, "openai") {
        exports.push(format!("export OPENAI_BASE_URL={}/openai", base));
        exports.push(format!("export OPENAI_API_KEY={}", token));
    }
    if has(entry, "github") {
        let host = host_only(base);
        exports.push(format!("export GH_HOST={}", host));
        exports.push(format!("export GH_ENTERPRISE_TOKEN={}", token));
        exports.push(format!("export GH_TOKEN={}", token));
    }
    if has(entry, "ollama") {
        exports.push(format!("export OLLAMA_HOST={}/ollama", base));
    }
    if has(entry, "httpbin") {
        exports.push(format!("export HTTPBIN_TOKEN={}", token));
    }

    Ok(exports)
}

fn has(entry: &crate::config::TokenEntry, route: &str) -> bool {
    entry.scope.is_empty() || entry.scope.iter().any(|s| s == route)
}

fn host_only(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped.split('/').next().unwrap_or(stripped).to_string()
}

fn write_codex_config(base: &str, token: &str) -> Result<()> {
    let codex_dir = dirs::home_dir()
        .map(|p| p.join(".codex"))
        .context("Cannot find home directory")?;
    std::fs::create_dir_all(&codex_dir)?;

    let config_path = codex_dir.join("config.toml");
    let content = format!(
        "[model]\nprovider = \"openai\"\nname = \"o4-mini\"\nopenai_base_url = \"{}/openai\"\n\n[auth]\napi_key = \"{}\"\n",
        base, token
    );
    std::fs::write(&config_path, content)?;
    eprintln!("Wrote {}", config_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_exports;
    use crate::config::{Config, TokenEntry};
    use std::collections::HashMap;

    #[test]
    fn github_exports_include_enterprise_token_for_custom_host() {
        let mut tokens = HashMap::new();
        tokens.insert(
            "laptop".to_string(),
            TokenEntry {
                token: "ccgw_test".to_string(),
                scope: vec!["github".to_string()],
            },
        );

        let config = Config {
            gateway_url: "https://localhost".to_string(),
            admin_token: None,
            tokens,
        };

        let exports = build_exports(&config, "laptop").unwrap();

        assert_eq!(
            exports,
            vec![
                "export GH_HOST=localhost",
                "export GH_ENTERPRISE_TOKEN=ccgw_test",
                "export GH_TOKEN=ccgw_test",
            ]
        );
    }
}
