use crate::config::Config;
use anyhow::{Context, Result};

pub fn run(name: &str, codex: bool) -> Result<()> {
    let config = Config::load()?;
    let token = config.tokens.get(name)
        .ok_or_else(|| anyhow::anyhow!("Token '{}' not found in config", name))?;

    let base = config.gateway_url.trim_end_matches('/');

    println!("export ANTHROPIC_BASE_URL={}/anthropic", base);
    println!("export ANTHROPIC_API_KEY={}", token);
    println!("export OPENAI_BASE_URL={}/openai", base);
    println!("export OPENAI_API_KEY={}", token);
    println!("export GH_HOST={}", host_only(base));
    println!("export GH_TOKEN={}", token);
    println!("export OLLAMA_HOST={}/ollama", base);

    if codex {
        write_codex_config(base, token)?;
    }

    Ok(())
}

fn host_only(url: &str) -> String {
    let stripped = url.trim_start_matches("https://").trim_start_matches("http://");
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