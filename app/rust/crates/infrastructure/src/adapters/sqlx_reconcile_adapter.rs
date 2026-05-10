use anyhow::{Context, Result};
use sqlx::{PgPool, Row};

use super::sqlx_adapter::connect_pg;

#[derive(Debug, Clone)]
pub struct ReconcileReportRecord {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone)]
pub struct ReconcileOptionsRecord {
    pub max_retry_count: i32,
    pub batch_limit: i64,
    pub dry_run: bool,
    pub requeue_base_delay_sec: i64,
    pub requeue_jitter_sec: i64,
}

pub fn load_default_reconcile_options() -> ReconcileOptionsRecord {
    ReconcileOptionsRecord {
        max_retry_count: std::env::var("RECONCILE_MAX_RETRY")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(10),
        batch_limit: std::env::var("RECONCILE_BATCH_LIMIT")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(500),
        dry_run: std::env::var("RECONCILE_DRY_RUN")
            .ok()
            .map(|s| matches!(s.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false),
        requeue_base_delay_sec: std::env::var("RECONCILE_REQUEUE_BASE_DELAY_SEC")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0),
        requeue_jitter_sec: std::env::var("RECONCILE_REQUEUE_JITTER_SEC")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(15),
    }
}

fn default_database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres_password@localhost:5433/alegria".to_string()
    })
}

pub async fn reconcile_target_system(
    pg: &PgPool,
    target_system: &str,
    opts: &ReconcileOptionsRecord,
) -> Result<ReconcileReportRecord> {
    let stale_candidates = sqlx::query(
        r#"
        WITH candidates AS (
          SELECT event_id
          FROM system.sync_outbox
          WHERE target_system = $1
            AND status = 'processing'
            AND (locked_until IS NULL OR locked_until <= now())
          ORDER BY created_at
          LIMIT $2
        )
        SELECT COUNT(*) AS cnt FROM candidates
        "#,
    )
    .bind(target_system)
    .bind(opts.batch_limit)
    .fetch_one(pg)
    .await
    .context("count stale reconcile candidates")?
    .get::<i64, _>("cnt");

    let failed_candidates = sqlx::query(
        r#"
        WITH candidates AS (
          SELECT event_id
          FROM system.sync_outbox
          WHERE target_system = $1
            AND status = 'failed'
            AND retry_count < $2
          ORDER BY updated_at DESC
          LIMIT $3
        )
        SELECT COUNT(*) AS cnt FROM candidates
        "#,
    )
    .bind(target_system)
    .bind(opts.max_retry_count)
    .bind(opts.batch_limit)
    .fetch_one(pg)
    .await
    .context("count failed reconcile candidates")?
    .get::<i64, _>("cnt");

    if opts.dry_run {
        return Ok(ReconcileReportRecord {
            target_system: target_system.to_string(),
            dry_run: true,
            stale_candidates,
            failed_candidates,
            reset_stale_processing: 0,
            requeued_failed: 0,
        });
    }

    let stale = sqlx::query(
        r#"
        WITH candidates AS (
          SELECT event_id
          FROM system.sync_outbox
          WHERE target_system = $1
            AND status = 'processing'
            AND (locked_until IS NULL OR locked_until <= now())
          ORDER BY created_at
          LIMIT $2
          FOR UPDATE SKIP LOCKED
        )
        UPDATE system.sync_outbox o
        SET status = 'pending',
            next_retry_at = now(),
            last_error = COALESCE(last_error, 'reconcile: reset stale processing lease'),
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        FROM candidates c
        WHERE o.event_id = c.event_id
        "#,
    )
    .bind(target_system)
    .bind(opts.batch_limit)
    .execute(pg)
    .await
    .context("reset stale reconcile candidates")?
    .rows_affected() as i64;

    let failed = sqlx::query(
        r#"
        WITH candidates AS (
          SELECT event_id
          FROM system.sync_outbox
          WHERE target_system = $1
            AND status = 'failed'
            AND retry_count < $2
          ORDER BY updated_at DESC
          LIMIT $3
          FOR UPDATE SKIP LOCKED
        )
        UPDATE system.sync_outbox o
        SET status = 'pending',
            next_retry_at = now()
              + make_interval(secs => $4::int)
              + make_interval(secs => FLOOR(random() * $5::int)::int),
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        FROM candidates c
        WHERE o.event_id = c.event_id
        "#,
    )
    .bind(target_system)
    .bind(opts.max_retry_count)
    .bind(opts.batch_limit)
    .bind(opts.requeue_base_delay_sec)
    .bind(opts.requeue_jitter_sec.max(1))
    .execute(pg)
    .await
    .context("requeue failed reconcile candidates")?
    .rows_affected() as i64;

    Ok(ReconcileReportRecord {
        target_system: target_system.to_string(),
        dry_run: false,
        stale_candidates,
        failed_candidates,
        reset_stale_processing: stale,
        requeued_failed: failed,
    })
}

pub async fn read_target_backlog(pg: &PgPool, target_system: &str) -> Result<(i64, i64)> {
    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE status = 'pending') AS pending_events,
          COUNT(*) FILTER (WHERE status = 'failed') AS failed_events
        FROM system.sync_outbox
        WHERE target_system = $1
        "#,
    )
    .bind(target_system)
    .fetch_one(pg)
    .await
    .context("read reconcile backlog")?;

    Ok((
        row.get::<i64, _>("pending_events"),
        row.get::<i64, _>("failed_events"),
    ))
}

pub async fn reconcile_target_system_default(
    target_system: &str,
    opts: &ReconcileOptionsRecord,
) -> Result<ReconcileReportRecord> {
    let pg = connect_pg(&default_database_url()).await?;
    reconcile_target_system(&pg, target_system, opts).await
}

pub async fn read_target_backlog_default(target_system: &str) -> Result<(i64, i64)> {
    let pg = connect_pg(&default_database_url()).await?;
    read_target_backlog(&pg, target_system).await
}
