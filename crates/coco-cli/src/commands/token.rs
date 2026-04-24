use crate::client::{admin_url, http_client};
use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct TokenResponse {
    id: String,
    name: String,
    scope: Vec<String>,
    created_at: String,
    token: String,
    warning: Option<String>,
}

#[derive(Deserialize)]
struct TokenListEntry {
    id: String,
    name: String,
    scope: Vec<String>,
    status: String,
}

#[derive(Serialize)]
struct CreateRequest {
    name: String,
    scope: Vec<String>,
}

pub async fn create(name: &str, scope: &[String]) -> Result<()> {
    let config = Config::load()?;
    let admin_token = config
        .admin_token
        .as_deref()
        .context("admin_token not set in config")?;

    let url = admin_url(&config, "/admin/tokens");
    let client = http_client();

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", admin_token))
        .json(&CreateRequest {
            name: name.to_string(),
            scope: scope.to_vec(),
        })
        .send()
        .await
        .context("Failed to connect to gateway")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let token_resp: TokenResponse = resp.json().await?;

    let mut config = Config::load()?;
    config.tokens.insert(
        token_resp.name.clone(),
        crate::config::TokenEntry {
            token: token_resp.token.clone(),
            scope: token_resp.scope.clone(),
        },
    );
    config.save()?;

    println!("id:         {}", token_resp.id);
    println!("name:       {}", token_resp.name);
    println!("scope:      {:?}", token_resp.scope);
    println!("created_at: {}", token_resp.created_at);
    println!("token:      {}", token_resp.token);
    if let Some(warning) = token_resp.warning {
        eprintln!("Warning: {warning}");
    }
    eprintln!("\nSaved to {}", crate::config::Config::path().display());
    Ok(())
}

pub async fn list() -> Result<()> {
    let config = Config::load()?;
    let admin_token = config
        .admin_token
        .as_deref()
        .context("admin_token not set in config")?;

    let url = admin_url(&config, "/admin/tokens");
    let client = http_client();

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .context("Failed to connect to gateway")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let tokens: Vec<TokenListEntry> = resp.json().await?;
    if tokens.is_empty() {
        println!("No tokens found.");
        return Ok(());
    }

    for t in &tokens {
        let scope = if t.scope.is_empty() {
            "*".to_string()
        } else {
            t.scope.join(",")
        };
        println!("{:<36} {:<15} {:<10} {}", t.id, t.name, t.status, scope);
    }
    Ok(())
}

pub async fn revoke(name: &str) -> Result<()> {
    let config = Config::load()?;
    let admin_token = config
        .admin_token
        .as_deref()
        .context("admin_token not set in config")?;

    let url = admin_url(&config, "/admin/tokens");
    let client = http_client();

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .context("Failed to connect to gateway")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let tokens: Vec<TokenListEntry> = resp.json().await?;
    let target = tokens
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| anyhow::anyhow!("Token '{}' not found", name))?;

    let revoke_url = admin_url(&config, &format!("/admin/tokens/{}", target.id));
    let resp = client
        .delete(&revoke_url)
        .header("Authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .context("Failed to connect to gateway")?;

    if resp.status().is_success() {
        println!("Token '{}' revoked.", name);
    } else {
        anyhow::bail!(
            "Gateway returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    Ok(())
}
