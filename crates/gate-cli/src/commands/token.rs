use crate::config::Config;
use crate::secure_file::validate_path_component;
use crate::tooling;
use crate::transport::AdminTransport;
use anyhow::Result;
use reqwest::Method;
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
    validate_path_component(name, "token name")?;
    validate_scope(scope, all_routes)?;

    let config = Config::load()?;
    let transport = AdminTransport::from_config(&config)?;

    let body = serde_json::to_value(CreateRequest {
        name: name.to_string(),
        scope: scope.to_vec(),
        all_routes,
    })?;

    let (status, text) = transport
        .request(Method::POST, "/admin/tokens", Some(body))
        .await?;

    if !status.is_success() {
        anyhow::bail!("Gateway returned {}: {}", status, text);
    }

    let token_resp: TokenResponse = serde_json::from_str(&text)?;

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
    let transport = AdminTransport::from_config(&config)?;

    let (status, text) = transport
        .request(Method::GET, "/admin/tokens", None)
        .await?;

    if !status.is_success() {
        anyhow::bail!("Gateway returned {}: {}", status, text);
    }

    let tokens: Vec<TokenListEntry> = serde_json::from_str(&text)?;
    if tokens.is_empty() {
        println!("No tokens found.");
        return Ok(());
    }

    print_token_table(&tokens);
    Ok(())
}

fn print_token_table(tokens: &[TokenListEntry]) {
    let rows: Vec<_> = tokens
        .iter()
        .map(|token| {
            let scope = if token.all_routes {
                "all routes".to_string()
            } else if token.scope.is_empty() {
                "none".to_string()
            } else {
                token.scope.join(", ")
            };
            [
                token.name.clone(),
                token.status.to_uppercase(),
                scope,
                token.id.clone(),
            ]
        })
        .collect();

    let headers = ["NAME", "STATUS", "SCOPE", "ID"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.len());
        }
    }

    print_row(&headers, &widths);
    print_separator(&widths);
    for row in &rows {
        let cells = [
            row[0].as_str(),
            row[1].as_str(),
            row[2].as_str(),
            row[3].as_str(),
        ];
        print_row(&cells, &widths);
    }
}

fn print_row(cells: &[&str; 4], widths: &[usize; 4]) {
    println!(
        "{:<name_width$}  {:<status_width$}  {:<scope_width$}  {}",
        cells[0],
        cells[1],
        cells[2],
        cells[3],
        name_width = widths[0],
        status_width = widths[1],
        scope_width = widths[2],
    );
}

fn print_separator(widths: &[usize; 4]) {
    println!(
        "{}  {}  {}  {}",
        "-".repeat(widths[0]),
        "-".repeat(widths[1]),
        "-".repeat(widths[2]),
        "-".repeat(widths[3]),
    );
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

#[cfg(test)]
mod tests {
    use crate::secure_file::validate_path_component;

    #[test]
    fn token_create_rejects_names_that_are_paths() {
        for name in ["", ".", "..", "../escape", "nested/name", r"nested\name"] {
            assert!(validate_path_component(name, "token name").is_err());
        }
        validate_path_component("laptop-1", "token name").unwrap();
    }
}

pub async fn revoke(name: &str) -> Result<()> {
    let config = Config::load()?;
    let transport = AdminTransport::from_config(&config)?;

    let (status, text) = transport
        .request(Method::GET, "/admin/tokens", None)
        .await?;

    if !status.is_success() {
        anyhow::bail!("Gateway returned {}: {}", status, text);
    }

    let tokens: Vec<TokenListEntry> = serde_json::from_str(&text)?;
    let target = tokens
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| anyhow::anyhow!("Token '{}' not found", name))?;

    let revoke_path = format!("/admin/tokens/{}", target.id);
    let (status, text) = transport
        .request(Method::DELETE, &revoke_path, None)
        .await?;

    if status.is_success() {
        println!("Token '{}' revoked.", name);
    } else {
        anyhow::bail!("Gateway returned {}: {}", status, text);
    }
    Ok(())
}
