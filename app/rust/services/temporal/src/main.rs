use anyhow::{Context, Result};
use infrastructure::adapters::sqlx_adapter::connect_pg;
use infrastructure::adapters::temporalio_sdk_adapter::{build_runtime, connect_client, Worker};
use std::env;
use std::sync::Arc;

mod activities;
mod metrics;
mod workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let temporal_url =
        std::env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".to_string());
    let db_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let host = env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string());
    let build_id = env::var("WORKER_BUILD_ID").unwrap_or_else(|_| "dev-local".to_string());
    let metrics_port = env::var("METRICS_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(9464);
    let worker_identity = format!("alegria-temporal-worker@{}:{}", host, std::process::id());

    metrics::start_metrics_server(metrics_port).await?;
    tracing::info!(metrics_port, build_id = %build_id, "metrics server started");

    let runtime = build_runtime()?;
    let client = connect_client(&temporal_url, worker_identity, "default").await?;

    let pool = connect_pg(&db_url).await?;
    // Reconcile any interrupted public release before consuming more publish
    // activities. A crash cannot silently turn an uncommitted release into
    // an accepted site after the worker restarts.
    #[cfg(unix)]
    infrastructure::adapters::sqlx_seo_cms_adapter::recover_pending_publication(&pool)
        .await
        .context("restore or verify interrupted static publication")?;
    #[cfg(unix)]
    infrastructure::adapters::sqlx_seo_cms_adapter::recover_pending_renderer_refresh(&pool)
        .await
        .context("restore interrupted renderer-only publication")?;
    let pool = Arc::new(pool);
    // Navigation and layout refreshes reuse published CMS revisions; no
    // editorial rerun or synthetic rebuild queue is necessary.
    #[cfg(unix)]
    {
        let refresh_pool = Arc::clone(&pool);
        tokio::spawn(
            infrastructure::adapters::sqlx_seo_cms_adapter::run_published_navigation_refresh_loop(
                refresh_pool,
                db_url.clone(),
                Arc::new(infrastructure::adapters::sqlx_seo_cms_adapter::CloudflareStaticDelivery),
            ),
        );
    }
    let acts = activities::AlegriaActivities { pool };

    // WorkerOptions built inside workflows module to keep workflow types private
    let worker_opts = workflows::build_worker_options("alegria-pipeline", acts)?;

    // Worker::new returns Result<_, Box<dyn Error>> — not anyhow-compatible directly
    Worker::new(&runtime, client, worker_opts)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .run()
        .await?;
    Ok(())
}
