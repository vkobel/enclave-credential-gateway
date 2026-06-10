use crate::config::Config;
use anyhow::{Context, Result};
use reqwest::Method;
use std::collections::HashMap;

/// Abstraction over plain-HTTP and steve E2E-encrypted admin transport.
pub enum AdminTransport {
    Plain {
        client: reqwest::Client,
        admin_token: String,
        gateway_url: String,
    },
    Encrypted {
        client: steve_sdk::Client,
        admin_token: String,
    },
}

impl std::fmt::Debug for AdminTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminTransport::Plain { gateway_url, .. } => f
                .debug_struct("AdminTransport::Plain")
                .field("gateway_url", gateway_url)
                .finish_non_exhaustive(),
            AdminTransport::Encrypted { client, .. } => f
                .debug_struct("AdminTransport::Encrypted")
                .field("client", client)
                .finish_non_exhaustive(),
        }
    }
}

impl AdminTransport {
    pub fn from_config(config: &Config) -> Result<Self> {
        let admin_token = config
            .admin_token
            .clone()
            .context("admin_token not set in config")?;

        if !config.e2e {
            return Ok(AdminTransport::Plain {
                client: crate::client::http_client(),
                admin_token,
                gateway_url: config.gateway_url.clone(),
            });
        }

        // e2e = true: require attestation config
        let att = config.attestation.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "e2e is enabled but no [attestation] section found in config.\n\
                Set pcr0/pcr1/pcr2 hex values under [attestation] in ~/.config/gate/config.toml,\n\
                or disable encryption with e2e = false."
            )
        })?;

        let pcrs = build_pcrs(att).context("invalid attestation PCR hex values")?;

        let builder = steve_sdk::Client::builder(&config.gateway_url, pcrs)
            .context("invalid gateway_url for steve client")?;

        let builder = if let Some(ref base) = att.attestation_base_url {
            builder
                .attestation_base_url(base)
                .context("invalid attestation_base_url")?
        } else {
            builder
        };

        Ok(AdminTransport::Encrypted {
            client: builder.build(),
            admin_token,
        })
    }

    /// Issue an admin request. Returns `(status_code, body_text)`.
    /// Always sends `Authorization: Bearer <admin_token>`.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        json_body: Option<serde_json::Value>,
    ) -> Result<(reqwest::StatusCode, String)> {
        match self {
            AdminTransport::Plain {
                client,
                admin_token,
                gateway_url,
            } => {
                let url = plain_url(gateway_url, path);
                let mut req = client
                    .request(method, &url)
                    .header("Authorization", format!("Bearer {admin_token}"));
                if let Some(body) = json_body {
                    req = req.json(&body);
                }
                let resp = req.send().await.context("Failed to connect to gateway")?;
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                Ok((status, text))
            }
            AdminTransport::Encrypted {
                client,
                admin_token,
            } => {
                let mut builder = client
                    .request(method, path)
                    .header("authorization", format!("Bearer {admin_token}"))
                    .map_err(|e| anyhow::anyhow!("invalid header: {e}"))?;
                if let Some(body) = json_body {
                    builder = builder
                        .json(&body)
                        .map_err(|e| anyhow::anyhow!("failed to serialize body: {e}"))?;
                }
                let resp = builder
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("gateway request failed: {e}"))?;
                let status = resp.status();
                let text = resp
                    .text()
                    .map_err(|e| anyhow::anyhow!("response body is not valid UTF-8: {e}"))?
                    .to_owned();
                Ok((status, text))
            }
        }
    }
}

fn plain_url(gateway_url: &str, path: &str) -> String {
    let base = gateway_url.trim_end_matches('/');
    format!("{base}{path}")
}

fn build_pcrs(att: &crate::config::AttestationConfig) -> Result<steve_sdk::ExpectedPcrs> {
    let pcr0 = hex::decode(&att.pcr0).context("pcr0 is not valid hex")?;
    let pcr1 = hex::decode(&att.pcr1).context("pcr1 is not valid hex")?;
    let pcr2 = hex::decode(&att.pcr2).context("pcr2 is not valid hex")?;
    Ok(HashMap::from_iter([(0u8, pcr0), (1u8, pcr1), (2u8, pcr2)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AttestationConfig, Config};

    fn config_e2e_no_attestation() -> Config {
        Config {
            gateway_url: "https://example.com".to_string(),
            admin_token: Some("tok_test".to_string()),
            e2e: true,
            attestation: None,
            ..Config::default()
        }
    }

    #[test]
    fn e2e_without_attestation_config_errors_with_actionable_message() {
        let config = config_e2e_no_attestation();
        let err = AdminTransport::from_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("[attestation]"),
            "error should mention [attestation] section, got: {msg}"
        );
        assert!(
            msg.contains("e2e = false"),
            "error should mention e2e = false opt-out, got: {msg}"
        );
    }

    #[test]
    fn plain_mode_constructs_without_attestation() {
        let config = Config {
            gateway_url: "https://example.com".to_string(),
            admin_token: Some("tok_test".to_string()),
            e2e: false,
            attestation: None,
            ..Config::default()
        };
        assert!(AdminTransport::from_config(&config).is_ok());
    }

    #[test]
    fn e2e_with_valid_attestation_config_constructs() {
        let config = Config {
            gateway_url: "https://example.com".to_string(),
            admin_token: Some("tok_test".to_string()),
            e2e: true,
            attestation: Some(AttestationConfig {
                pcr0: "ef093e4c1fd13878956589833c0e396b935cdf5ae45c1cc595e1a19a6da5812850f0ef3e77df918cb2a86d88ddf9cc03".to_string(),
                pcr1: "ef093e4c1fd13878956589833c0e396b935cdf5ae45c1cc595e1a19a6da5812850f0ef3e77df918cb2a86d88ddf9cc03".to_string(),
                pcr2: "21b9efbc184807662e966d34f390821309eeac6802309798826296bf3e8bec7c10edb30948c90ba67310f7b964fc500a".to_string(),
                attestation_base_url: None,
            }),
            ..Config::default()
        };
        assert!(AdminTransport::from_config(&config).is_ok());
    }

    #[test]
    fn e2e_with_invalid_pcr_hex_errors() {
        let config = Config {
            gateway_url: "https://example.com".to_string(),
            admin_token: Some("tok_test".to_string()),
            e2e: true,
            attestation: Some(AttestationConfig {
                pcr0: "not-valid-hex".to_string(),
                pcr1: "aabbcc".to_string(),
                pcr2: "aabbcc".to_string(),
                attestation_base_url: None,
            }),
            ..Config::default()
        };
        let err = AdminTransport::from_config(&config).unwrap_err();
        let full = format!("{err:#}");
        assert!(
            full.contains("pcr0"),
            "error chain should mention pcr0: {full}"
        );
    }
}
