use anyhow::Result;
use reqwest::Client;
use std::time::Duration;

pub fn new_default_client(timeout_sec: u64) -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(timeout_sec))
        .tcp_keepalive(Duration::from_secs(60))
        .build()?)
}
