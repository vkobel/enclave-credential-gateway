use crate::config::Config;
use crate::transport::AdminTransport;
use anyhow::{bail, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct RegisterRequest {
    name: String,
    service: String,
    value: String,
}

#[derive(Deserialize)]
struct CredListEntry {
    name: String,
    service: String,
}

#[derive(Deserialize)]
struct CredListResponse {
    creds: Vec<CredListEntry>,
}

/// Validate a cred name: [A-Za-z0-9_-], non-empty, ≤128.
pub fn validate_cred_name(name: &str, label: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        bail!("{label} must be 1–128 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("{label} must contain only [A-Za-z0-9_-]");
    }
    Ok(())
}

pub async fn register(name: &str, service: &str, value: &str) -> Result<()> {
    validate_cred_name(name, "cred name")?;
    validate_cred_name(service, "service name")?;

    let config = Config::load()?;
    let transport = AdminTransport::from_config(&config)?;

    let body = serde_json::to_value(RegisterRequest {
        name: name.to_string(),
        service: service.to_string(),
        value: value.to_string(),
    })?;

    let (status, text) = transport
        .request(Method::POST, "/admin/creds", Some(body))
        .await?;

    if !status.is_success() {
        bail!("Gateway returned {}: {}", status, text);
    }

    println!("registered {} ({})", name, service);
    Ok(())
}

pub async fn list() -> Result<()> {
    let config = Config::load()?;
    let transport = AdminTransport::from_config(&config)?;

    let (status, text) = transport.request(Method::GET, "/admin/creds", None).await?;

    if !status.is_success() {
        bail!("Gateway returned {}: {}", status, text);
    }

    let resp: CredListResponse = serde_json::from_str(&text)?;
    if resp.creds.is_empty() {
        println!("No creds registered.");
        return Ok(());
    }

    print_creds_table(&resp.creds);
    Ok(())
}

fn print_creds_table(creds: &[CredListEntry]) {
    let name_width = creds
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0)
        .max("NAME".len());
    let service_width = creds
        .iter()
        .map(|c| c.service.len())
        .max()
        .unwrap_or(0)
        .max("SERVICE".len());

    println!(
        "{:<name_width$}  {:<service_width$}",
        "NAME",
        "SERVICE",
        name_width = name_width,
        service_width = service_width
    );
    println!("{}  {}", "-".repeat(name_width), "-".repeat(service_width));
    for cred in creds {
        println!(
            "{:<name_width$}  {:<service_width$}",
            cred.name,
            cred.service,
            name_width = name_width,
            service_width = service_width
        );
    }
}

pub async fn rm(name: &str) -> Result<()> {
    validate_cred_name(name, "cred name")?;

    let config = Config::load()?;
    let transport = AdminTransport::from_config(&config)?;

    let path = format!("/admin/creds/{}", name);
    let (status, text) = transport.request(Method::DELETE, &path, None).await?;

    if !status.is_success() {
        bail!("Gateway returned {}: {}", status, text);
    }

    println!("removed {}", name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_cred_name;

    #[test]
    fn cred_name_rejects_empty() {
        assert!(validate_cred_name("", "cred name").is_err());
    }

    #[test]
    fn cred_name_rejects_too_long() {
        let long = "a".repeat(129);
        let err = validate_cred_name(&long, "cred name").unwrap_err();
        assert!(err.to_string().contains("1–128"));
    }

    #[test]
    fn cred_name_rejects_invalid_chars() {
        for name in ["gh prod", "gh/prod", "gh.prod", "gh@prod"] {
            let err = validate_cred_name(name, "cred name").unwrap_err();
            assert!(
                err.to_string().contains("[A-Za-z0-9_-]"),
                "expected char error for {:?}, got: {}",
                name,
                err
            );
        }
    }

    #[test]
    fn cred_name_accepts_valid() {
        for name in ["gh-prod", "gh_prod", "GH-PROD-1", "a", "A1-b_C"] {
            validate_cred_name(name, "cred name").unwrap();
        }
    }
}
