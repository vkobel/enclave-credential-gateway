use reqwest::Client;

pub fn http_client() -> Client {
    Client::new()
}
