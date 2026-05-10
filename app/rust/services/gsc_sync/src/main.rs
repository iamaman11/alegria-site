use anyhow::{anyhow, Result};
use infrastructure::adapters::reqwest_adapter::new_default_client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let token =
        std::env::var("GSC_ACCESS_TOKEN").map_err(|_| anyhow!("GSC_ACCESS_TOKEN is not set"))?;
    let site_url = std::env::var("GSC_SITE_URL").map_err(|_| anyhow!("GSC_SITE_URL is not set"))?;

    // Minimal GSC connectivity check. Full ingestion to serp.crawl_queue is a separate step.
    let endpoint = "https://www.googleapis.com/webmasters/v3/sites";
    let http = new_default_client(30)?;
    let resp = http.get(endpoint).bearer_auth(token).send().await?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    tracing::info!(
        target = "gsc_sync",
        status = %status,
        site_url = %site_url,
        body = %body,
        "gsc connectivity check complete"
    );

    println!(
        "{}",
        json!({
            "status": if status.is_success() { "ok" } else { "error" },
            "http_status": status.as_u16(),
            "site_url": site_url,
        })
    );
    Ok(())
}
