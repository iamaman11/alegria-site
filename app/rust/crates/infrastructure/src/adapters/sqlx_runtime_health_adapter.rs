use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn count_open_dead_letters(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM system.dead_letter_queue
         WHERE resolution_status = 'open'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0))
}

pub async fn count_stale_runs(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM pipeline.execution_runs
         WHERE status NOT IN ('done','failed')
           AND status <> 'pending_hitl'
           AND updated_at < now() - interval '12 hours'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0))
}

pub async fn count_pending_hitl_runs(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM pipeline.execution_runs
         WHERE status = 'pending_hitl'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0))
}

pub async fn count_stuck_steps(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM pipeline.step_executions
         WHERE status = 'running'
           AND updated_at < now() - interval '1 hour'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0))
}

pub async fn is_run_pending_hitl(pool: &PgPool, run_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*)::bigint
             FROM pipeline.execution_runs
             WHERE run_id = $1 AND status = 'pending_hitl'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        > 0)
}
