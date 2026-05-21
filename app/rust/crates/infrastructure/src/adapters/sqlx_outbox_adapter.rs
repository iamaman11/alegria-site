use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct OutboxEnvelope {
    pub run_id: String,
    pub aggregate_type: String,
    pub aggregate_key: String,
    pub target_system: String,
    pub event_type: String,
    pub payload_type: String,
    pub schema_version: i32,
    pub idempotency_key: String,
    pub payload_bytes: Vec<u8>,
}

impl OutboxEnvelope {
    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload_bytes
    }
}

#[derive(Debug, Clone)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_key: String,
    pub target_system: String,
    pub event_type: String,
    pub payload_type: String,
    pub schema_version: i32,
    pub idempotency_key: String,
    pub payload_bytes: Vec<u8>,
    pub retry_count: i32,
}

#[derive(Debug, Clone)]
pub struct SyncStatusSnapshot {
    pub pending_events: i64,
    pub failed_events: i64,
    pub max_lag_ms: i64,
}

pub async fn claim_outbox_batch(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
    lease_seconds: i64,
    only_event_id: Option<Uuid>,
) -> Result<Vec<OutboxEvent>> {
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT event_id
            FROM system.sync_outbox
            WHERE ($4::uuid IS NULL OR event_id = $4) AND (
                status = 'pending'
                AND next_retry_at <= now()
            ) OR (
                ($4::uuid IS NULL OR event_id = $4) AND
                status = 'processing'
                AND (locked_until IS NULL OR locked_until <= now())
            )
            ORDER BY created_at
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE system.sync_outbox o
        SET status = 'processing',
            worker_id = $2,
            locked_at = now(),
            locked_until = now() + make_interval(secs => $3::int),
            updated_at = now()
        FROM candidates c
        WHERE o.event_id = c.event_id
        RETURNING
            o.event_id,
            o.aggregate_type,
            o.aggregate_key,
            o.target_system,
            o.event_type,
            o.payload_type,
            o.schema_version,
            o.idempotency_key,
            o.payload_bytes,
            o.retry_count
        "#,
    )
    .bind(limit)
    .bind(worker_id)
    .bind(lease_seconds)
    .bind(only_event_id)
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|r| OutboxEvent {
            event_id: r.get("event_id"),
            aggregate_type: r.get("aggregate_type"),
            aggregate_key: r.get("aggregate_key"),
            target_system: r.get("target_system"),
            event_type: r.get("event_type"),
            payload_type: r.get("payload_type"),
            schema_version: r.get("schema_version"),
            idempotency_key: r.get("idempotency_key"),
            payload_bytes: r.get("payload_bytes"),
            retry_count: r.get("retry_count"),
        })
        .collect();

    Ok(events)
}

pub async fn claim_outbox_batch_for_run_target(
    pool: &PgPool,
    worker_id: &str,
    run_id: &str,
    target_system: &str,
    limit: i64,
    lease_seconds: i64,
) -> Result<Vec<OutboxEvent>> {
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT event_id
            FROM system.sync_outbox
            WHERE run_id = $4
              AND target_system = $5
              AND (
                (status = 'pending' AND next_retry_at <= now())
                OR
                (status = 'processing' AND (locked_until IS NULL OR locked_until <= now()))
              )
            ORDER BY created_at
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE system.sync_outbox o
        SET status = 'processing',
            worker_id = $2,
            locked_at = now(),
            locked_until = now() + make_interval(secs => $3::int),
            updated_at = now()
        FROM candidates c
        WHERE o.event_id = c.event_id
        RETURNING
            o.event_id,
            o.aggregate_type,
            o.aggregate_key,
            o.target_system,
            o.event_type,
            o.payload_type,
            o.schema_version,
            o.idempotency_key,
            o.payload_bytes,
            o.retry_count
        "#,
    )
    .bind(limit)
    .bind(worker_id)
    .bind(lease_seconds)
    .bind(run_id)
    .bind(target_system)
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|r| OutboxEvent {
            event_id: r.get("event_id"),
            aggregate_type: r.get("aggregate_type"),
            aggregate_key: r.get("aggregate_key"),
            target_system: r.get("target_system"),
            event_type: r.get("event_type"),
            payload_type: r.get("payload_type"),
            schema_version: r.get("schema_version"),
            idempotency_key: r.get("idempotency_key"),
            payload_bytes: r.get("payload_bytes"),
            retry_count: r.get("retry_count"),
        })
        .collect();

    Ok(events)
}

pub async fn mark_done(pool: &PgPool, event_id: Uuid) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'done',
            last_error = NULL,
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_retry(
    pool: &PgPool,
    event_id: Uuid,
    error: &str,
    next_retry_delay_sec: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'pending',
            retry_count = retry_count + 1,
            next_retry_at = now() + make_interval(secs => $2::int),
            last_error = left($3, 4000),
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .bind(next_retry_delay_sec)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_deferred(
    pool: &PgPool,
    event_id: Uuid,
    reason: &str,
    next_retry_delay_sec: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'pending',
            next_retry_at = now() + make_interval(secs => $2::int),
            last_error = left($3, 4000),
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .bind(next_retry_delay_sec)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(pool: &PgPool, event_id: Uuid, error: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'failed',
            retry_count = retry_count + 1,
            last_error = left($2, 4000),
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn read_sync_status(pool: &PgPool) -> Result<SyncStatusSnapshot> {
    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE status = 'pending') AS pending_events,
          COUNT(*) FILTER (WHERE status = 'failed') AS failed_events,
          COALESCE(
            MAX(
              CASE
                WHEN status IN ('pending','processing') THEN
                  GREATEST(0, FLOOR(EXTRACT(EPOCH FROM (now() - created_at)) * 1000))::BIGINT
                ELSE 0
              END
            ),
            0
          ) AS max_lag_ms
        FROM system.sync_outbox
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(SyncStatusSnapshot {
        pending_events: row.get::<i64, _>("pending_events"),
        failed_events: row.get::<i64, _>("failed_events"),
        max_lag_ms: row.get::<i64, _>("max_lag_ms"),
    })
}
