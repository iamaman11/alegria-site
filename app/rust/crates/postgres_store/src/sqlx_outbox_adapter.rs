use anyhow::Result;
use sqlx::types::time::OffsetDateTime;
use sqlx::{Connection, PgConnection, PgPool, Row};
use std::future::Future;
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

/// An unknown remote outcome is *not* a normal retryable Outbox failure.
/// A committed marker is written before any verified-rule Qdrant RPC. The
/// reconciler must never requeue it, and no subsequent point effect can pass
/// the same PostgreSQL advisory lock while this marker exists.
///
/// This is deliberately a fail-closed quarantine. Clearing it requires an
/// operator to establish that the old RPC can no longer arrive and to verify
/// the remote projection before releasing newer intents. The API exposes no
/// automatic release or blind retry of an uncertain effect.
pub const VERIFIED_RULE_UNCERTAIN_MARKER: &str = "projection_outcome_uncertain:v1";

#[derive(Debug, thiserror::Error)]
pub enum VerifiedRuleTruthFenceError {
    #[error("invalid verified-rule projection identity")]
    InvalidIdentity,
    #[error("legacy Truth mutation of already projected verified rule {aggregate_key} requires atomic PostgreSQL registry and Qdrant Outbox intent")]
    NonAtomicLegacyProjection { aggregate_key: String },
    #[error("verified-rule Truth mutation blocked: Qdrant outcome of event {event_id} for {aggregate_key} is uncertain; manually reconcile the remote effect first")]
    Uncertain {
        event_id: Uuid,
        aggregate_key: String,
    },
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// PostgreSQL-side prerequisite for *every* verified-rule canonical mutation.
/// Call this from the same Truth SQL transaction before touching the rule,
/// registry or its Outbox intent, and after any enclosing context-level lock.
///
/// It acquires the identical sorted per-point xact advisory locks as the
/// dispatcher, then rejects mutations if an earlier provider RPC outcome is
/// uncertain. This second half is essential: blocking only the next Outbox
/// delivery still permits a stale, delayed old RPC to land after Truth commits
/// a demotion. Temporary Truth write refusal is intentional and fail-closed.
pub async fn lock_and_ensure_verified_rules_writable_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule_ids: &[String],
) -> std::result::Result<(), VerifiedRuleTruthFenceError> {
    let sorted = rule_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for rule_id in &sorted {
        if rule_id.is_empty() || rule_id.trim() != rule_id {
            return Err(VerifiedRuleTruthFenceError::InvalidIdentity);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("qdrant|verified_rules_4|rule_instance|{rule_id}"))
            .execute(&mut **tx)
            .await?;
    }
    if sorted.is_empty() {
        return Ok(());
    }
    let ordered = sorted.into_iter().collect::<Vec<_>>();
    let uncertain: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT event_id, aggregate_key FROM system.sync_outbox
         WHERE target_system='qdrant' AND aggregate_type='rule_instance'
           AND aggregate_key=ANY($1::text[])
           AND status='failed' AND last_error=$2
         ORDER BY created_at, event_id LIMIT 1",
    )
    .bind(&ordered)
    .bind(VERIFIED_RULE_UNCERTAIN_MARKER)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((event_id, aggregate_key)) = uncertain {
        return Err(VerifiedRuleTruthFenceError::Uncertain {
            event_id,
            aggregate_key,
        });
    }
    Ok(())
}

/// Fail-closed compatibility boundary for legacy Truth writers that cannot
/// atomically stage a verified-rule registry change AND its Qdrant Outbox
/// intent in the same PostgreSQL transaction.
///
/// A legacy writer may still modify a rule that has never had an active
/// `verified_rules_4` point. It MUST NOT mutate an already projected rule:
/// that would change canonical Truth while leaving Qdrant's old point live,
/// with no corresponding durable repair intent. This is intentionally a
/// temporary loss of legacy functionality pending its atomic writer cutover.
///
/// This helper must be called BEFORE any canonical rule mutation and within
/// that mutation's own transaction. It obtains the usual per-point advisory
/// locks and rejects unresolved remote outcomes first; the registry check is
/// then protected against concurrent canonical projection writers by the
/// SAME locks, so an active point cannot appear between check and mutation.
pub async fn lock_and_reject_projected_legacy_truth_write_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule_ids: &[String],
) -> std::result::Result<(), VerifiedRuleTruthFenceError> {
    lock_and_ensure_verified_rules_writable_in_tx(tx, rule_ids).await?;
    if rule_ids.is_empty() {
        return Ok(());
    }
    let projected_rule: Option<String> = sqlx::query_scalar(
        "SELECT entity_key FROM kb.qdrant_points
         WHERE collection_name='verified_rules_4'
           AND entity_type='rule_instance' AND entity_key=ANY($1::text[])
         ORDER BY entity_key LIMIT 1",
    )
    .bind(rule_ids)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(aggregate_key) = projected_rule {
        return Err(VerifiedRuleTruthFenceError::NonAtomicLegacyProjection { aggregate_key });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedRuleDispatchOutcome {
    /// This event was superseded or its SQL registry intent was withdrawn.
    /// The event has already been marked `done`; no provider call was made.
    SkippedStale,
    /// The provider acknowledged the effect. The durable uncertainty marker
    /// was then conditionally replaced with `done` under the advisory lock.
    Delivered,
}

#[derive(Debug)]
pub enum VerifiedRuleDispatchError<E> {
    InvalidEvent(&'static str),
    LeaseLost,
    BlockedUncertain {
        event_id: Uuid,
    },
    /// UUIDv4 is not an ordering tie-breaker. A monotonic per-point revision
    /// is required to resolve the older-vs-newer intent correctly.
    AmbiguousOrdering,
    Database(sqlx::Error),
    /// An RPC may have reached the provider before failing or being cancelled.
    /// The committed marker stays in place, including on caller cancellation.
    UnknownOutcome(E),
}

impl<E: std::fmt::Display> std::fmt::Display for VerifiedRuleDispatchError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEvent(reason) => write!(f, "invalid verified-rule Outbox event: {reason}"),
            Self::LeaseLost => write!(f, "verified-rule Outbox lease lost before effect"),
            Self::BlockedUncertain { event_id } => write!(
                f,
                "verified-rule projection blocked by uncertain event {event_id}; manual reconciliation required"
            ),
            Self::AmbiguousOrdering => write!(
                f,
                "verified-rule Outbox intent order ambiguous; monotonic per-point revision required"
            ),
            Self::Database(error) => write!(f, "verified-rule PostgreSQL fence: {error}"),
            Self::UnknownOutcome(error) => write!(
                f,
                "verified-rule remote outcome uncertain (durable quarantine retained): {error}"
            ),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for VerifiedRuleDispatchError<E> {}

fn validate_verified_rule_event(event: &OutboxEvent) -> std::result::Result<(), &'static str> {
    if event.target_system != "qdrant" || event.aggregate_type != "rule_instance" {
        return Err("expected qdrant/rule_instance identity");
    }
    if event.aggregate_key.is_empty() || event.aggregate_key.trim() != event.aggregate_key {
        return Err("empty or non-canonical rule identity");
    }
    let expected_payload_type = match event.event_type.as_str() {
        "QdrantUpsertCommand" => "alegria.outbox.qdrant_upsert_command.v1",
        "QdrantDeleteCommand" => "alegria.outbox.qdrant_delete_command.v1",
        _ => return Err("unsupported verified-rule event_type"),
    };
    if event.payload_type != expected_payload_type || event.schema_version != 1 {
        return Err("unsupported verified-rule payload contract");
    }
    if event.payload_bytes.is_empty() || event.lease_worker_id.is_empty() {
        return Err("missing payload or worker lease identity");
    }
    Ok(())
}

/// Timestamp ties cannot be broken by random UUIDv4 ordering. This narrow
/// guard refuses known ambiguity; it is *not* a substitute for a monotonic
/// writer-owned revision (timestamps can regress or precede commit order).
fn latest_intent_timestamp_unambiguous(events: &[(Uuid, String, OffsetDateTime)]) -> bool {
    events.len() < 2 || events[0].2 != events[1].2
}

/// Execute the *single* verified-rule projection effect while preserving the
/// exact PostgreSQL per-point fence used by canonical Truth writers.
///
/// The caller MUST validate the protobuf point identity against aggregate_key
/// before providing `effect`. Only the PG Outbox owner checks freshness/lease.
/// `effect` is invoked at most once, while the transaction-scoped advisory lock
/// is held. It MUST perform exactly the external effect and return `Ok(())`
/// only after a definitive provider acknowledgement. It must not acquire the
/// same PG advisory lock, and must not perform a second Outbox state update.
///
/// Atomic order:
/// 1. Acquire the same advisory lock used by every verified-rule Truth writer.
/// 2. Validate the current processing lease, latest event ID+payload hash,
///    current registry intent and absence of an older unknown RPC.
/// 3. Commit `status='failed', last_error=UNCERTAIN_MARKER` via a SECOND PG
///    connection, with lease CAS, before invoking the provider.
/// 4. Keep the advisory lock across provider IO. Only its successful ACK allows
///    an exact-marker CAS to `done`; errors, cancellation or crash leave the
///    marker durable and block all later effects for this point.
///
/// Both success outcomes are already `done`; callers MUST NOT call mark_done.
/// The marker writer uses a separate direct PgConnection initialized from the
/// supplied pool's connection options. Acquiring two slots from one bounded
/// pool would deadlock under N concurrent dispatches with N reserved slots.
pub async fn dispatch_verified_rule_with_fence<F, Fut, E>(
    pool: &PgPool,
    event: &OutboxEvent,
    effect: F,
) -> std::result::Result<VerifiedRuleDispatchOutcome, VerifiedRuleDispatchError<E>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<(), E>>,
{
    validate_verified_rule_event(event).map_err(VerifiedRuleDispatchError::InvalidEvent)?;
    // Never reserve two slots from the *same* bounded pool. N concurrent
    // calls reserving N first slots could wait forever for second slots.
    // A dedicated direct connection shares PG credentials/configuration with
    // the pool, but not its capacity semaphore; a failed connection prevents
    // the external effect from starting and is safe to retry normally.
    let mut marker_conn = PgConnection::connect_with(pool.connect_options().as_ref())
        .await
        .map_err(VerifiedRuleDispatchError::Database)?;
    let mut lock_tx = pool
        .begin()
        .await
        .map_err(VerifiedRuleDispatchError::Database)?;
    let advisory_key = format!(
        "qdrant|verified_rules_4|rule_instance|{}",
        event.aggregate_key
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(&advisory_key)
        .execute(&mut *lock_tx)
        .await
        .map_err(VerifiedRuleDispatchError::Database)?;

    let lease_live: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM system.sync_outbox
            WHERE event_id=$1 AND status='processing'
              AND worker_id=$2 AND locked_at=$3 AND locked_until > now()
              AND target_system='qdrant' AND aggregate_type='rule_instance'
              AND aggregate_key=$4 AND event_type=$5 AND payload_type=$6
              AND schema_version=1 AND payload_hash=$7
         )",
    )
    .bind(event.event_id)
    .bind(&event.lease_worker_id)
    .bind(event.lease_locked_at)
    .bind(&event.aggregate_key)
    .bind(&event.event_type)
    .bind(&event.payload_type)
    .bind(primitives::hash::blake3_hex(&event.payload_bytes))
    .fetch_one(&mut *lock_tx)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    if !lease_live {
        return Err(VerifiedRuleDispatchError::LeaseLost);
    }

    // The marker is per physical point identity, NOT per event or run. An
    // earlier request may still land after its sender crashed or timed out.
    let uncertain_event: Option<Uuid> = sqlx::query_scalar(
        "SELECT event_id FROM system.sync_outbox
         WHERE target_system='qdrant' AND aggregate_type='rule_instance'
           AND aggregate_key=$1 AND status='failed' AND last_error=$2
         ORDER BY created_at, event_id LIMIT 1",
    )
    .bind(&event.aggregate_key)
    .bind(VERIFIED_RULE_UNCERTAIN_MARKER)
    .fetch_optional(&mut *lock_tx)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    if let Some(event_id) = uncertain_event {
        return Err(VerifiedRuleDispatchError::BlockedUncertain { event_id });
    }

    let latest: Vec<(Uuid, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT event_id, payload_hash, created_at FROM system.sync_outbox
         WHERE aggregate_type='rule_instance' AND aggregate_key=$1
           AND target_system='qdrant'
           AND event_type IN ('QdrantUpsertCommand', 'QdrantDeleteCommand')
         ORDER BY created_at DESC, event_id DESC LIMIT 2",
    )
    .bind(&event.aggregate_key)
    .fetch_all(&mut *lock_tx)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    if !latest_intent_timestamp_unambiguous(&latest) {
        return Err(VerifiedRuleDispatchError::AmbiguousOrdering);
    }

    let registry_active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM kb.qdrant_points
            WHERE collection_name='verified_rules_4'
              AND entity_type='rule_instance' AND entity_key=$1
         )",
    )
    .bind(&event.aggregate_key)
    .fetch_one(&mut *lock_tx)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    let expected_active = event.event_type == "QdrantUpsertCommand";
    let current = latest.first().is_some_and(|(event_id, digest, _)| {
        *event_id == event.event_id
            && digest == &primitives::hash::blake3_hex(&event.payload_bytes)
            && registry_active == expected_active
    });
    if !current {
        // A superseded delivery is a terminal no-op. No remote call and no
        // quarantine marker are necessary; the advisory lock makes this
        // decision atomic with all participating Truth writers.
        let updated = sqlx::query(
            "UPDATE system.sync_outbox SET status='done', last_error=NULL,
                 locked_at=NULL, locked_until=NULL, worker_id=NULL, updated_at=now()
             WHERE event_id=$1 AND status='processing'
               AND worker_id=$2 AND locked_at=$3",
        )
        .bind(event.event_id)
        .bind(&event.lease_worker_id)
        .bind(event.lease_locked_at)
        .execute(&mut *lock_tx)
        .await
        .map_err(VerifiedRuleDispatchError::Database)?;
        if updated.rows_affected() != 1 {
            return Err(VerifiedRuleDispatchError::LeaseLost);
        }
        lock_tx
            .commit()
            .await
            .map_err(VerifiedRuleDispatchError::Database)?;
        return Ok(VerifiedRuleDispatchOutcome::SkippedStale);
    }

    // The status is committed BEFORE the RPC. A killed or cancelled process
    // cannot clear it merely by dropping the PG transaction/advisory lock.
    let marked = sqlx::query(
        "UPDATE system.sync_outbox
         SET status='failed', last_error=$4, worker_id=NULL,
             locked_at=NULL, locked_until=NULL, updated_at=now()
         WHERE event_id=$1 AND status='processing'
           AND worker_id=$2 AND locked_at=$3 AND locked_until > now()",
    )
    .bind(event.event_id)
    .bind(&event.lease_worker_id)
    .bind(event.lease_locked_at)
    .bind(VERIFIED_RULE_UNCERTAIN_MARKER)
    .execute(&mut marker_conn)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    if marked.rows_affected() != 1 {
        return Err(VerifiedRuleDispatchError::LeaseLost);
    }

    // Any Err is uncertain even if it looks retryable: the provider may have
    // applied it. Never attempt automatic release or resend from this path.
    effect()
        .await
        .map_err(VerifiedRuleDispatchError::UnknownOutcome)?;

    let acknowledged = sqlx::query(
        "UPDATE system.sync_outbox SET status='done', last_error=NULL,
             locked_at=NULL, locked_until=NULL, worker_id=NULL, updated_at=now()
         WHERE event_id=$1 AND status='failed' AND last_error=$2",
    )
    .bind(event.event_id)
    .bind(VERIFIED_RULE_UNCERTAIN_MARKER)
    .execute(&mut marker_conn)
    .await
    .map_err(VerifiedRuleDispatchError::Database)?;
    if acknowledged.rows_affected() != 1 {
        // Someone changed the marker outside this protocol. Do not report an
        // acknowledged effect as a safely finalized Outbox event.
        return Err(VerifiedRuleDispatchError::LeaseLost);
    }
    lock_tx
        .commit()
        .await
        .map_err(VerifiedRuleDispatchError::Database)?;
    Ok(VerifiedRuleDispatchOutcome::Delivered)
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
    use super::{
        latest_intent_timestamp_unambiguous, validate_target_claim, validate_verified_rule_event,
        OutboxEvent, VerifiedRuleTruthFenceError, VERIFIED_RULE_UNCERTAIN_MARKER,
    };
    use sqlx::types::time::OffsetDateTime;
    use uuid::Uuid;

    fn verified_rule_event(event_type: &str, payload_type: &str) -> OutboxEvent {
        OutboxEvent {
            event_id: Uuid::new_v4(),
            run_id: "run-1".into(),
            aggregate_type: "rule_instance".into(),
            aggregate_key: "rule-1".into(),
            target_system: "qdrant".into(),
            event_type: event_type.into(),
            payload_type: payload_type.into(),
            schema_version: 1,
            idempotency_key: "run-1|rule-1".into(),
            payload_bytes: vec![1],
            retry_count: 0,
            lease_worker_id: "worker-1".into(),
            lease_locked_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn verified_rule_fence_rejects_non_rule_payload_contracts_before_sql() {
        let upsert = verified_rule_event(
            "QdrantUpsertCommand",
            "alegria.outbox.qdrant_upsert_command.v1",
        );
        let delete = verified_rule_event(
            "QdrantDeleteCommand",
            "alegria.outbox.qdrant_delete_command.v1",
        );
        assert!(validate_verified_rule_event(&upsert).is_ok());
        assert!(validate_verified_rule_event(&delete).is_ok());

        let mut invalid = upsert;
        invalid.aggregate_type = "raw_section".into();
        assert!(validate_verified_rule_event(&invalid).is_err());
        invalid.aggregate_type = "rule_instance".into();
        invalid.aggregate_key = " rule-1".into();
        assert!(validate_verified_rule_event(&invalid).is_err());
        invalid.aggregate_key = "rule-1".into();
        invalid.event_type = "SeoGraphProjectionUpserted".into();
        assert!(validate_verified_rule_event(&invalid).is_err());
        invalid.event_type = "QdrantUpsertCommand".into();
        invalid.schema_version = 2;
        assert!(validate_verified_rule_event(&invalid).is_err());
        invalid.schema_version = 1;
        invalid.payload_bytes.clear();
        assert!(validate_verified_rule_event(&invalid).is_err());
        assert_eq!(
            VERIFIED_RULE_UNCERTAIN_MARKER,
            "projection_outcome_uncertain:v1"
        );
    }

    #[test]
    fn random_uuid_tie_must_not_choose_latest_projection_intent() {
        let one = (
            Uuid::new_v4(),
            "hash-a".to_owned(),
            OffsetDateTime::UNIX_EPOCH,
        );
        let tied = (
            Uuid::new_v4(),
            "hash-b".to_owned(),
            OffsetDateTime::UNIX_EPOCH,
        );
        let later = (
            Uuid::new_v4(),
            "hash-c".to_owned(),
            OffsetDateTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
        );
        assert!(latest_intent_timestamp_unambiguous(&[]));
        assert!(latest_intent_timestamp_unambiguous(&[one.clone()]));
        assert!(!latest_intent_timestamp_unambiguous(&[one.clone(), tied]));
        assert!(latest_intent_timestamp_unambiguous(&[later, one]));
    }

    #[test]
    fn legacy_projected_truth_write_error_identifies_the_unsafe_rule() {
        let error = VerifiedRuleTruthFenceError::NonAtomicLegacyProjection {
            aggregate_key: "rule-42".to_owned(),
        };
        let message = error.to_string();
        assert!(message.contains("rule-42"));
        assert!(message.contains("atomic PostgreSQL registry and Qdrant Outbox"));
    }

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
