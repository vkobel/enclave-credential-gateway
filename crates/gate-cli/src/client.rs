use crate::config::Config;
use reqwest::Client;

pub fn http_client() -> Client {
    Client::new()
}

pub fn admin_url(config: &Config, path: &str) -> String {
    let base = config.gateway_url.trim_end_matches('/');
    format!("{}{}", base, path)
}
