use crate::client::{admin_url, http_client};
use crate::config::Config;
use crate::tooling;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct TokenResponse {
    id: String,
    name: String,
    scope: Vec<String>,
    #[serde(default)]
    all_routes: bool,
    created_at: String,
    token: String,
    warning: Option<String>,
}

#[derive(Deserialize)]
struct TokenListEntry {
    id: String,
    name: String,
    scope: Vec<String>,
    #[serde(default)]
    all_routes: bool,
    status: String,
}

#[derive(Serialize)]
struct CreateRequest {
    name: String,
    scope: Vec<String>,
    all_routes: bool,
}

pub async fn create(name: &str, scope: &[String], all_routes: bool) -> Result<()> {
    validate_scope(scope, all_routes)?;

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
            all_routes,
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
            all_routes: token_resp.all_routes,
        },
    );
    config.save()?;

    println!("id:         {}", token_resp.id);
    println!("name:       {}", token_resp.name);
    println!("scope:      {:?}", token_resp.scope);
    println!("all_routes: {}", token_resp.all_routes);
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
        let scope = if t.all_routes {
            "*".to_string()
        } else {
            t.scope.join(",")
        };
        println!("{:<36} {:<15} {:<10} {}", t.id, t.name, t.status, scope);
    }
    Ok(())
}

fn validate_scope(scope: &[String], all_routes: bool) -> Result<()> {
    if all_routes && !scope.is_empty() {
        anyhow::bail!("use either --scope or --all-routes, not both");
    }
    if scope.is_empty() && !all_routes {
        anyhow::bail!("scope must be non-empty (or pass --all-routes for unrestricted)");
    }

    let known_routes = tooling::known_routes()?;
    let unknown: Vec<_> = scope
        .iter()
        .filter(|route| !known_routes.contains(route))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        anyhow::bail!(
            "unknown route(s): {} (known: {})",
            unknown.join(", "),
            known_routes.join(", ")
        );
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
