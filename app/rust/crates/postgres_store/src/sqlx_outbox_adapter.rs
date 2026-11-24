use anyhow::Result;
use sqlx::types::time::OffsetDateTime;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use primitives::errors::ErrorClass;

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
    pub run_id: String,
    pub aggregate_type: String,
    pub aggregate_key: String,
    pub target_system: String,
    pub event_type: String,
    pub payload_type: String,
    pub schema_version: i32,
    pub idempotency_key: String,
    pub payload_bytes: Vec<u8>,
    pub retry_count: i32,
    pub lease_worker_id: String,
    pub lease_locked_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseUpdate {
    Applied,
    Lost,
}

#[derive(Debug, Clone)]
pub struct SyncStatusSnapshot {
    pub pending_events: i64,
    pub failed_events: i64,
    pub max_lag_ms: i64,
}

/// Preserve the legacy unscoped claim for callers that intentionally deliver
/// all provider targets. A provider-specific executor must use the scoped API.
pub async fn claim_outbox_batch(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
    lease_seconds: i64,
    only_event_id: Option<Uuid>,
) -> Result<Vec<OutboxEvent>> {
    claim_outbox_batch_scoped(pool, worker_id, limit, lease_seconds, only_event_id, None).await
}

/// A provider-specific worker may claim only its configured target. The filter
/// is inside the row-locking candidate query, before SKIP LOCKED, so other
/// targets' leases are never transiently acquired. This is routing, not
/// authorization or fencing of external effects.
pub async fn claim_outbox_batch_for_target(
    pool: &PgPool,
    worker_id: &str,
    target_system: &str,
    limit: i64,
    lease_seconds: i64,
    only_event_id: Option<Uuid>,
) -> Result<Vec<OutboxEvent>> {
    validate_target_claim(worker_id, target_system, limit, lease_seconds)?;
    claim_outbox_batch_scoped(
        pool,
        worker_id,
        limit,
        lease_seconds,
        only_event_id,
        Some(target_system),
    )
    .await
}

/// This routing API is not a list of today's provider implementations.
/// The target remains an opaque name, but malformed names and leases fail before SQL.
fn validate_target_claim(
    worker_id: &str,
    target_system: &str,
    limit: i64,
    lease_seconds: i64,
) -> Result<()> {
    anyhow::ensure!(
        !worker_id.is_empty()
            && worker_id == worker_id.trim()
            && !worker_id.chars().any(char::is_control),
        "invalid Outbox worker_id"
    );
    anyhow::ensure!(
        !target_system.is_empty()
            && target_system.len() <= 128
            && target_system.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')
            }),
        "invalid Outbox target_system"
    );
    anyhow::ensure!(limit > 0, "invalid Outbox claim limit");
    anyhow::ensure!(
        (1..=i32::MAX as i64).contains(&lease_seconds),
        "invalid Outbox lease duration"
    );
    Ok(())
}

async fn claim_outbox_batch_scoped(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
    lease_seconds: i64,
    only_event_id: Option<Uuid>,
    target_system: Option<&str>,
) -> Result<Vec<OutboxEvent>> {
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT event_id
            FROM system.sync_outbox
            WHERE ($4::uuid IS NULL OR event_id = $4)
              AND ($5::text IS NULL OR target_system = $5)
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
            o.run_id,
            o.aggregate_type,
            o.aggregate_key,
            o.target_system,
            o.event_type,
            o.payload_type,
            o.schema_version,
            o.idempotency_key,
            o.payload_bytes,
            o.retry_count,
            o.worker_id,
            o.locked_at
        "#,
    )
    .bind(limit)
    .bind(worker_id)
    .bind(lease_seconds)
    .bind(only_event_id)
    .bind(target_system)
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|r| OutboxEvent {
            event_id: r.get("event_id"),
            run_id: r.get("run_id"),
            aggregate_type: r.get("aggregate_type"),
            aggregate_key: r.get("aggregate_key"),
            target_system: r.get("target_system"),
            event_type: r.get("event_type"),
            payload_type: r.get("payload_type"),
            schema_version: r.get("schema_version"),
            idempotency_key: r.get("idempotency_key"),
            payload_bytes: r.get("payload_bytes"),
            retry_count: r.get("retry_count"),
            lease_worker_id: r.get("worker_id"),
            lease_locked_at: r.get("locked_at"),
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
            o.run_id,
            o.aggregate_type,
            o.aggregate_key,
            o.target_system,
            o.event_type,
            o.payload_type,
            o.schema_version,
            o.idempotency_key,
            o.payload_bytes,
            o.retry_count,
            o.worker_id,
            o.locked_at
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
            run_id: r.get("run_id"),
            aggregate_type: r.get("aggregate_type"),
            aggregate_key: r.get("aggregate_key"),
            target_system: r.get("target_system"),
            event_type: r.get("event_type"),
            payload_type: r.get("payload_type"),
            schema_version: r.get("schema_version"),
            idempotency_key: r.get("idempotency_key"),
            payload_bytes: r.get("payload_bytes"),
            retry_count: r.get("retry_count"),
            lease_worker_id: r.get("worker_id"),
            lease_locked_at: r.get("locked_at"),
        })
        .collect();

    Ok(events)
}

pub async fn mark_done(pool: &PgPool, event: &OutboxEvent) -> Result<LeaseUpdate> {
    let result = sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'done',
            last_error = NULL,
            locked_at = NULL,
            locked_until = NULL,
            worker_id = NULL,
            updated_at = now()
        WHERE event_id = $1 AND status = 'processing'
          AND worker_id = $2 AND locked_at = $3
        "#,
    )
    .bind(event.event_id)
    .bind(&event.lease_worker_id)
    .bind(event.lease_locked_at)
    .execute(pool)
    .await?;
    Ok(if result.rows_affected() == 1 {
        LeaseUpdate::Applied
    } else {
        LeaseUpdate::Lost
    })
}

pub async fn mark_retry(
    pool: &PgPool,
    event: &OutboxEvent,
    error: &str,
    next_retry_delay_sec: i64,
) -> Result<LeaseUpdate> {
    let result = sqlx::query(
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
        WHERE event_id = $1 AND status = 'processing'
          AND worker_id = $4 AND locked_at = $5
        "#,
    )
    .bind(event.event_id)
    .bind(next_retry_delay_sec)
    .bind(error)
    .bind(&event.lease_worker_id)
    .bind(event.lease_locked_at)
    .execute(pool)
    .await?;
    Ok(if result.rows_affected() == 1 {
        LeaseUpdate::Applied
    } else {
        LeaseUpdate::Lost
    })
}

/// A terminal failure and its dead letter are one transaction. If persisting
/// the dead letter fails, the lease remains recoverable instead of losing it.
pub async fn mark_failed_with_dead_letter(
    pool: &PgPool,
    event: &OutboxEvent,
    step_name: &str,
    error_class: ErrorClass,
    build_id: &str,
    error: &str,
) -> Result<LeaseUpdate> {
    let mut tx = pool.begin().await?;
    let update = sqlx::query(
        r#"
        UPDATE system.sync_outbox
        SET status = 'failed', retry_count = retry_count + 1,
            last_error = left($2, 4000), locked_at = NULL,
            locked_until = NULL, worker_id = NULL, updated_at = now()
        WHERE event_id = $1 AND status = 'processing'
          AND worker_id = $3 AND locked_at = $4
        "#,
    )
    .bind(event.event_id)
    .bind(error)
    .bind(&event.lease_worker_id)
    .bind(event.lease_locked_at)
    .execute(&mut *tx)
    .await?;
    if update.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(LeaseUpdate::Lost);
    }
    let payload_hash = primitives::hash::blake3_hex(&event.payload_bytes);
    let idempotency_key = if event.idempotency_key.is_empty() {
        event.event_id.to_string()
    } else {
        event.idempotency_key.clone()
    };
    // Legacy run IDs may be non-UUID strings, and not every projection run
    // has a pipeline.execution_runs row. Preserve all of them in sync_outbox;
    // only attach the optional UUID foreign key when its parent exists.
    let run_uuid = Uuid::parse_str(&event.run_id).ok();
    sqlx::query(
        r#"
        INSERT INTO system.dead_letter_queue
        (run_id, step_name, workflow_id, error_class, retryable, payload_type,
         schema_version, payload_bytes, payload_hash, input_hash,
         idempotency_key, build_id, error_message)
        VALUES ((SELECT run_id FROM pipeline.execution_runs WHERE run_id=$1),
                $2, $3, $4, false, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(run_uuid)
    .bind(step_name)
    .bind(event.event_id.to_string())
    .bind(error_class.as_str())
    .bind(&event.payload_type)
    .bind(event.schema_version)
    .bind(&event.payload_bytes)
    .bind(&payload_hash)
    .bind(&payload_hash)
    .bind(&idempotency_key)
    .bind(build_id)
    .bind(error)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(LeaseUpdate::Applied)
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

#[cfg(test)]
mod tests {
    use super::validate_target_claim;

    #[test]
    fn target_claim_admits_future_providers_without_central_enum() {
        for target in ["neo4j", "qdrant", "cms", "future-provider.v3"] {
            assert!(validate_target_claim("worker-1", target, 64, 120).is_ok());
        }
    }

    #[test]
    fn target_claim_rejects_ambiguous_names_and_invalid_leases_before_sql() {
        for target in ["", " ", "Qdrant", " neo4j", "qdrant ", "a/b", "cms\n"] {
            assert!(validate_target_claim("worker-1", target, 64, 120).is_err());
        }
        assert!(validate_target_claim("worker-1", &"a".repeat(129), 64, 120).is_err());
        assert!(validate_target_claim("", "qdrant", 64, 120).is_err());
        assert!(validate_target_claim("worker\n1", "qdrant", 64, 120).is_err());
        assert!(validate_target_claim("worker-1", "qdrant", 0, 120).is_err());
        assert!(validate_target_claim("worker-1", "qdrant", 64, 0).is_err());
        assert!(validate_target_claim("worker-1", "qdrant", 64, i64::MAX).is_err());
    }
}
