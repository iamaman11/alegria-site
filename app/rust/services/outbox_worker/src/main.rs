use anyhow::Result;
use std::env;

use infrastructure::adapters::sqlx_adapter::connect_pg;
use tracing::info;

mod materialize;
mod retry;
mod worker;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres_password@localhost:5433/alegria".to_string());
    let worker_id = env::var("OUTBOX_WORKER_ID").unwrap_or_else(|_| {
        format!("outbox-worker-{}", std::process::id())
    });
    let max_cycles = env::var("OUTBOX_MAX_CYCLES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok());

    info!(worker_id, "starting outbox worker");

    let pool = connect_pg(&database_url).await?;
    worker::run_worker_loop(&pool, &database_url, &worker_id, max_cycles).await?;
    Ok(())
}
