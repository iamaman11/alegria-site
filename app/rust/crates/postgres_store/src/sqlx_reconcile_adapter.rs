//! Outbox reconciliation only edits PostgreSQL leases and retry queues.

use anyhow::{Context, Result};
use operation_protocol::{
    ArtifactSchema, EffectId, IdempotencyPolicy, Operation, OperationContract, OperationId,
    OperationOwnerId, OperationRef, PortId,
};
use runtime_models::ReconcileOutboxCommand;
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, BTreeSet};

use super::sqlx_adapter::connect_pg;

pub use runtime_models::{
    ReconcileOutboxOptions as ReconcileOptionsRecord,
    ReconcileTargetReportRecord as ReconcileReportRecord,
};

/// The operation's physical owner is PostgreSQL. Its effect changes only
/// Outbox leases/retry rows, never the Qdrant/Neo4j projections themselves.
pub struct ReconcileOutbox;

impl Operation for ReconcileOutbox {
    type Input = ReconcileOutboxCommand;
    type Output = ReconcileReportRecord;
    type Failure = ReconcileOutboxFailure;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId::from("postgres.reconcile_outbox"),
                revision: 1,
            },
            owner: OperationOwnerId::from("postgres_outbox"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                ArtifactSchema {
                    id: "runtime.outbox_reconcile_command".into(),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                ArtifactSchema {
                    id: "runtime.outbox_reconcile_report".into(),
                    revision: 1,
                },
            )]),
            effects: BTreeSet::from([EffectId::from("postgres.outbox.reconcile")]),
            // This is a maintenance mutation; the report and row set can
            // change between calls. Never auto-retry an ambiguous outcome.
            idempotency: IdempotencyPolicy::AtMostOnce,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcileOutboxFailure {
    #[error("invalid PostgreSQL Outbox reconciliation command: {0}")]
    InvalidCommand(&'static str),
    #[error("PostgreSQL Outbox reconciliation failed: {0}")]
    Database(#[from] anyhow::Error),
}

fn validate_outbox_command(
    command: &ReconcileOutboxCommand,
) -> std::result::Result<(), ReconcileOutboxFailure> {
    if command.target_system.trim().is_empty()
        || command.target_system != command.target_system.trim()
        || command.target_system.chars().any(char::is_control)
    {
        return Err(ReconcileOutboxFailure::InvalidCommand("target_system"));
    }
    let opts = &command.options;
    if opts.batch_limit <= 0 {
        return Err(ReconcileOutboxFailure::InvalidCommand("batch_limit"));
    }
    if opts.max_retry_count < 0 {
        return Err(ReconcileOutboxFailure::InvalidCommand("max_retry_count"));
    }
    if opts.requeue_base_delay_sec < 0 || opts.requeue_base_delay_sec > i32::MAX as i64 {
        return Err(ReconcileOutboxFailure::InvalidCommand(
            "requeue_base_delay_sec",
        ));
    }
    if opts.requeue_jitter_sec < 0 || opts.requeue_jitter_sec > i32::MAX as i64 {
        return Err(ReconcileOutboxFailure::InvalidCommand("requeue_jitter_sec"));
    }
    Ok(())
}

/// One typed owner adapter. Keeps the existing SQL changes and ordering in
/// `reconcile_target_system`; it does not initiate a second transaction or
/// call a projection provider. Untrusted commands fail before any SQL call.
/// The contract's AtMostOnce policy is NOT enforced by this direct-call API:
/// durable retries require a separate atomic execution ledger and lease fence.
/// This is internal Outbox maintenance, not a site business workflow step.
pub async fn execute_reconcile_outbox(
    pg: &PgPool,
    command: &ReconcileOutboxCommand,
) -> std::result::Result<ReconcileReportRecord, ReconcileOutboxFailure> {
    validate_outbox_command(command)?;
    reconcile_target_system(pg, &command.target_system, &command.options)
        .await
        .map_err(ReconcileOutboxFailure::Database)
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

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{ContractCatalog, IdempotencyPolicy};

    fn command() -> ReconcileOutboxCommand {
        ReconcileOutboxCommand {
            target_system: "qdrant".to_owned(),
            options: ReconcileOptionsRecord {
                max_retry_count: 10,
                batch_limit: 100,
                dry_run: false,
                requeue_base_delay_sec: 0,
                requeue_jitter_sec: 15,
            },
        }
    }

    #[test]
    fn postgres_reconcile_is_a_versioned_owner_operation_with_no_provider_sdk() {
        let mut catalog = ContractCatalog::default();
        catalog.register::<ReconcileOutbox>().unwrap();
        let contract = ReconcileOutbox::contract();
        assert_eq!(contract.owner.as_str(), "postgres_outbox");
        assert_eq!(contract.reference.id.as_str(), "postgres.reconcile_outbox");
        assert_eq!(contract.idempotency, IdempotencyPolicy::AtMostOnce);
        assert_eq!(contract.effects.len(), 1);
        assert!(catalog.contract(&contract.reference).is_some());
    }

    #[test]
    fn invalid_reconcile_commands_are_rejected_before_sql() {
        let mut input = command();
        assert!(validate_outbox_command(&input).is_ok());
        input.target_system = " ".to_owned();
        assert!(validate_outbox_command(&input).is_err());
        input = command();
        input.options.batch_limit = 0;
        assert!(validate_outbox_command(&input).is_err());
        input = command();
        input.options.requeue_base_delay_sec = i64::MAX;
        assert!(validate_outbox_command(&input).is_err());
        input = command();
        input.options.max_retry_count = -1;
        assert!(validate_outbox_command(&input).is_err());
    }
}
