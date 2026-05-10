use anyhow::{Context, Result};
use sqlx::PgPool;

#[derive(Debug, Clone, Copy, Default)]
pub struct FreshnessSnapshot {
    pub stale_count: i64,
    pub max_lag_hours: f64,
}

pub async fn load_freshness_snapshot(
    pool: &PgPool,
    threshold_hours: i64,
) -> Result<FreshnessSnapshot> {
    let stale_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM pipeline.execution_runs
         WHERE status NOT IN ('done','failed')
           AND status <> 'pending_hitl'
           AND created_at < now() - ($1::text || ' hours')::interval",
    )
    .bind(threshold_hours.to_string())
    .fetch_one(pool)
    .await
    .context("load stale execution run count")?;

    let max_lag_hours: Option<f64> = sqlx::query_scalar(
        "SELECT (EXTRACT(EPOCH FROM (now() - min(created_at))) / 3600.0)::float8
         FROM pipeline.execution_runs
         WHERE status NOT IN ('done','failed')
           AND status <> 'pending_hitl'",
    )
    .fetch_one(pool)
    .await
    .context("load execution run max lag hours")?;

    Ok(FreshnessSnapshot {
        stale_count,
        max_lag_hours: max_lag_hours.unwrap_or_default(),
    })
}
