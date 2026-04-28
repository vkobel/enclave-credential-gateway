use crate::config::Config;
use anyhow::{anyhow, Result};
use std::io::{self, Read};

pub fn run(token_name: &str, operation: &str) -> Result<()> {
    let config = Config::load()?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if let Some(response) = credential_response(&config, token_name, operation, &input)? {
        print!("{response}");
    }

    Ok(())
}

fn credential_response(
    config: &Config,
    token_name: &str,
    operation: &str,
    input: &str,
) -> Result<Option<String>> {
    if operation != "get" {
        return Ok(None);
    }

    let entry = config
        .tokens
        .get(token_name)
        .ok_or_else(|| anyhow!("Token '{}' not found in config", token_name))?;
    if !entry.allows_route("github") {
        return Ok(None);
    }

    let request = CredentialRequest::parse(input);
    let Some((gateway_protocol, gateway_host)) = gateway_endpoint(config) else {
        return Ok(None);
    };

    if request.protocol.as_deref() != Some(gateway_protocol.as_str()) {
        return Ok(None);
    }
    if request.host.as_deref() != Some(gateway_host.as_str()) {
        return Ok(None);
    }

    Ok(Some(format!(
        "username=x-access-token\npassword={}\n\n",
        entry.token
    )))
}

fn gateway_endpoint(config: &Config) -> Option<(String, String)> {
    let gateway_url = config.gateway_url.trim_end_matches('/');
    let (protocol, rest) = gateway_url.split_once("://")?;
    let host = rest.split('/').next()?.to_ascii_lowercase();
    if protocol.is_empty() || host.is_empty() {
        return None;
    }
    Some((protocol.to_ascii_lowercase(), host))
}

#[derive(Default)]
struct CredentialRequest {
    protocol: Option<String>,
    host: Option<String>,
}

impl CredentialRequest {
    fn parse(input: &str) -> Self {
        let mut request = Self::default();
        for line in input.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "protocol" => request.protocol = Some(value.to_ascii_lowercase()),
                "host" => request.host = Some(value.to_ascii_lowercase()),
                _ => {}
            }
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use super::credential_response;
    use crate::config::{Config, TokenEntry};
    use std::collections::HashMap;

    fn config_with_scope(scope: &[&str]) -> Config {
        let mut tokens = HashMap::new();
        tokens.insert(
            "laptop".to_string(),
            TokenEntry {
                token: "ccgw_test".to_string(),
                scope: scope.iter().map(|scope| scope.to_string()).collect(),
            },
        );

        Config {
            gateway_url: "https://gw.example.com".to_string(),
            admin_token: None,
            tokens,
        }
    }

    #[test]
    fn returns_credentials_for_matching_gateway_host() {
        let config = config_with_scope(&["github"]);
        let response = credential_response(
            &config,
            "laptop",
            "get",
            "protocol=https\nhost=gw.example.com\npath=owner/repo.git\n\n",
        )
        .unwrap();

        assert_eq!(
            response.as_deref(),
            Some("username=x-access-token\npassword=ccgw_test\n\n")
        );
    }

    #[test]
    fn returns_nothing_for_other_hosts() {
        let config = config_with_scope(&["github"]);
        let response = credential_response(
            &config,
            "laptop",
            "get",
            "protocol=https\nhost=github.com\n\n",
        )
        .unwrap();

        assert!(response.is_none());
    }

    #[test]
    fn returns_nothing_without_github_scope() {
        let config = config_with_scope(&["openai"]);
        let response = credential_response(
            &config,
            "laptop",
            "get",
            "protocol=https\nhost=gw.example.com\n\n",
        )
        .unwrap();

        assert!(response.is_none());
    }

    #[test]
    fn ignores_store_and_erase_operations() {
        let config = config_with_scope(&["github"]);

        assert!(credential_response(
            &config,
            "laptop",
            "store",
            "protocol=https\nhost=gw.example.com\n\n",
        )
        .unwrap()
        .is_none());
        assert!(credential_response(
            &config,
            "laptop",
            "erase",
            "protocol=https\nhost=gw.example.com\n\n",
        )
        .unwrap()
        .is_none());
    }
}
