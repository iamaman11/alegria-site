use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::time::timeout;
use tracing::{error, info, warn};
use uuid::Uuid;

use postgres_store::sqlx_outbox_adapter::{
    claim_outbox_batch, claim_outbox_batch_for_target, mark_done, mark_failed_with_dead_letter,
    mark_retry, read_sync_status, LeaseUpdate,
};
use runtime_composition::adapters::projection_materialize_adapter::{
    DispatchDisposition, DispatchStage,
};
use telemetry::fbs_writer::write_runtime_window;

use crate::materialize::dispatch_event;
use crate::retry::next_retry_delay_sec;
use crate::health;

// A serial dispatcher must not reserve a batch whose tail can lose its lease
// before the first slow provider call returns. Claim one just-in-time event;
// the next non-empty cycle begins immediately without the idle polling delay.
const DEFAULT_BATCH_SIZE: i64 = 1;
const DEFAULT_LEASE_SECONDS: i64 = 120;
const DEFAULT_IDLE_POLL_MS: u64 = 1500;
const DEFAULT_MAX_RETRIES: i32 = 10;
const OUTBOX_NOTIFY_CHANNEL: &str = "sync_outbox_channel";

/// One target per future provider-owned worker. An omitted setting retains
/// the current mixed-worker deployment until all targets have their own
/// executors and have passed concurrent lease/fencing E2E.
fn checked_target_system(value: &str) -> Result<&str> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'_'
                    || byte == b'-'
                    || byte == b'.'
            }),
        "OUTBOX_TARGET_SYSTEM must be a nonempty lowercase provider identifier"
    );
    Ok(value)
}

fn configured_target_system() -> Result<Option<String>> {
    match std::env::var("OUTBOX_TARGET_SYSTEM") {
        Ok(value) => {
            checked_target_system(&value)?;
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).context("invalid OUTBOX_TARGET_SYSTEM setting"),
    }
}

fn parse_only_event_id(value: Option<&str>) -> Result<Option<Uuid>> {
    value
        .map(|raw| Uuid::parse_str(raw).context("invalid OUTBOX_ONLY_EVENT_ID UUID"))
        .transpose()
}

fn configured_only_event_id() -> Result<Option<Uuid>> {
    match std::env::var("OUTBOX_ONLY_EVENT_ID") {
        Ok(value) => parse_only_event_id(Some(&value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).context("invalid OUTBOX_ONLY_EVENT_ID setting"),
    }
}

fn current_build_id() -> String {
    std::env::var("OUTBOX_WORKER_BUILD_ID")
        .or_else(|_| std::env::var("WORKER_BUILD_ID"))
        .unwrap_or_else(|_| "outbox-worker-local".to_string())
}

fn emit_heartbeat(worker_id: &str, cycle: u32, claimed: usize) {
    let ts_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let payload = format!(
        "{{\"worker_id\":\"{}\",\"cycle\":{},\"claimed\":{},\"ts_epoch_ms\":{}}}",
        worker_id, cycle, claimed, ts_epoch_ms
    );
    write_runtime_window("worker_heartbeat", payload.as_bytes());
}

async fn emit_sync_status(pool: &PgPool) {
    match read_sync_status(pool).await {
        Ok(s) => {
            let payload = format!(
                "{{\"pending_events\":{},\"failed_events\":{},\"max_lag_ms\":{}}}",
                s.pending_events, s.failed_events, s.max_lag_ms
            );
            write_runtime_window("sync_status", payload.as_bytes());
        }
        Err(err) => {
            warn!(error = %err, "failed to read sync status snapshot");
        }
    }
}

pub async fn run_worker_loop(
    pool: &PgPool,
    database_url: &str,
    worker_id: &str,
    max_cycles: Option<u32>,
) -> Result<()> {
    let readiness = health::configured_file();
    if let Some(file) = readiness.as_deref() {
        // A restart may re-use the same ephemeral container filesystem.
        // Never expose the previous process's health before a real PG poll.
        health::clear(file)?;
    }
    let target_system = configured_target_system()?;
    info!(
        worker_id,
        target_system = target_system.as_deref().unwrap_or("all"),
        "outbox claim scope selected"
    );
    // Never silently widen a requested one-event lease to every event.
    let only_event_id = configured_only_event_id()?;
    let mut cycle: u32 = 0;
    let mut listener = match PgListener::connect(database_url).await {
        Ok(mut l) => {
            if let Err(err) = l.listen(OUTBOX_NOTIFY_CHANNEL).await {
                warn!(worker_id, error = %err, "failed to subscribe LISTEN channel; fallback polling only");
                None
            } else {
                info!(
                    worker_id,
                    channel = OUTBOX_NOTIFY_CHANNEL,
                    "LISTEN channel enabled"
                );
                Some(l)
            }
        }
        Err(err) => {
            warn!(worker_id, error = %err, "failed to init PgListener; fallback polling only");
            None
        }
    };

    loop {
        if let Some(max) = max_cycles {
            if cycle >= max {
                info!(
                    worker_id,
                    cycle, "outbox worker max cycles reached, exiting"
                );
                break;
            }
        }
        cycle += 1;
        emit_sync_status(pool).await;

        let batch = match target_system.as_deref() {
            Some(target) => {
                claim_outbox_batch_for_target(
                    pool,
                    worker_id,
                    target,
                    DEFAULT_BATCH_SIZE,
                    DEFAULT_LEASE_SECONDS,
                    only_event_id,
                )
                .await?
            }
            None => {
                claim_outbox_batch(
                    pool,
                    worker_id,
                    DEFAULT_BATCH_SIZE,
                    DEFAULT_LEASE_SECONDS,
                    only_event_id,
                )
                .await?
            }
        };
        if let Some(file) = readiness.as_deref() {
            health::publish(file)?;
        }
        if batch.is_empty() {
            emit_heartbeat(worker_id, cycle, 0);
            if let Some(l) = listener.as_mut() {
                // Wait for NOTIFY, but keep periodic fallback wake-ups.
                match timeout(Duration::from_millis(DEFAULT_IDLE_POLL_MS), l.recv()).await {
                    Ok(Ok(note)) => {
                        info!(
                            worker_id,
                            channel = note.channel(),
                            payload = note.payload(),
                            "received outbox notification"
                        );
                    }
                    Ok(Err(err)) => {
                        warn!(worker_id, error = %err, "LISTEN recv failed; switching to polling fallback");
                        listener = None;
                        tokio::time::sleep(Duration::from_millis(DEFAULT_IDLE_POLL_MS)).await;
                    }
                    Err(_) => {
                        // timeout: fallback scan cadence
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(DEFAULT_IDLE_POLL_MS)).await;
            }
            emit_sync_status(pool).await;
            continue;
        }

        emit_heartbeat(worker_id, cycle, batch.len());
        info!(
            worker_id,
            cycle,
            claimed = batch.len(),
            "outbox batch claimed"
        );

        for event in batch {
            let event_id = event.event_id;
            let event_type = event.event_type.clone();
            let aggregate_type = event.aggregate_type.clone();
            let aggregate_key = event.aggregate_key.clone();
            let target_system = event.target_system.clone();
            let payload_size = event.payload_bytes.len();

            info!(
                worker_id,
                %event_id,
                run_id = %event.run_id,
                event_type,
                aggregate_type,
                aggregate_key,
                target_system,
                payload_size,
                retry_count = event.retry_count,
                "processing outbox event"
            );

            match dispatch_event(pool, &event).await {
                Ok(disposition) => {
                    let completion = match disposition {
                        DispatchDisposition::NeedsAck => mark_done(pool, &event).await?,
                        DispatchDisposition::AcknowledgedByFence => LeaseUpdate::Applied,
                    };
                    if completion == LeaseUpdate::Lost {
                        warn!(worker_id, %event_id, run_id = %event.run_id,
                            "outbox completion rejected: another worker owns the lease");
                    } else {
                        info!(worker_id, %event_id, run_id = %event.run_id,
                            target_system = %event.target_system,
                            operation = %event.event_type,
                            attempt = event.retry_count + 1, result = "done",
                            "outbox event delivered");
                    }
                }
                Err(err) if err.stage == DispatchStage::Quarantine => {
                    // The PostgreSQL owner committed an uncertainty marker
                    // before the Qdrant RPC. It may still complete late.
                    // Retrying or dead-lettering this event can resurrect
                    // stale Truth, so only explicit reconciliation may clear
                    // the marker and unblock this point's future writes.
                    error!(worker_id, %event_id, run_id = %event.run_id,
                        target_system = %event.target_system,
                        operation = %event.event_type,
                        result = "quarantined_no_retry",
                        error = %err,
                        "verified-rule remote outcome uncertain; manual reconciliation required");
                }
                Err(err) => {
                    let msg = err.to_string();
                    if event.retry_count + 1 >= DEFAULT_MAX_RETRIES {
                        let result = mark_failed_with_dead_letter(
                            pool,
                            &event,
                            "sync_outbox_dispatch",
                            err.class,
                            &current_build_id(),
                            &msg,
                        )
                        .await?;
                        if result == LeaseUpdate::Applied {
                            error!(worker_id, %event_id, run_id = %event.run_id,
                                target_system = %event.target_system,
                                operation = %event.event_type,
                                retries = event.retry_count + 1,
                                result = "dead_letter",
                                error_class = err.class.as_str(), stage = ?err.stage,
                                error = %msg,
                                "outbox terminal failure and dead letter committed together");
                        } else {
                            warn!(worker_id, %event_id, run_id = %event.run_id,
                                "outbox failure rejected: another worker owns the lease");
                        }
                    } else {
                        let delay = next_retry_delay_sec(event.retry_count);
                        if mark_retry(pool, &event, &msg, delay).await? == LeaseUpdate::Lost {
                            warn!(worker_id, %event_id, run_id = %event.run_id,
                                "outbox retry rejected: another worker owns the lease");
                        } else {
                            warn!(worker_id, %event_id, run_id = %event.run_id,
                                operation = %event.event_type,
                                target_system = %event.target_system,
                                error_class = err.class.as_str(), stage = ?err.stage,
                                attempt = event.retry_count + 1, result = "retry",
                                retry_in_sec = delay, error = %msg,
                                "outbox event scheduled for retry");
                        }
                    }
                }
            }
        }
        emit_sync_status(pool).await;
    }

    if let Some(file) = readiness.as_deref() {
        health::clear(file)?;
    }

    Ok(())
}

#[cfg(test)]
mod target_selection_tests {
    use super::{checked_target_system, parse_only_event_id};

    #[test]
    fn target_scoped_worker_is_not_tied_to_the_current_three_providers() {
        for target in ["neo4j", "qdrant", "cms", "future_provider.v2"] {
            assert_eq!(checked_target_system(target).unwrap(), target);
        }
        for invalid in ["", "Neo4j", " qdrant", "cms ", "../qdrant", "qdrant\n"] {
            assert!(checked_target_system(invalid).is_err());
        }
    }


    #[test]
    fn malformed_one_event_scope_never_becomes_unscoped_delivery() {
        let expected = uuid::Uuid::new_v4();
        assert_eq!(parse_only_event_id(None).unwrap(), None);
        assert_eq!(parse_only_event_id(Some(&expected.to_string())).unwrap(), Some(expected));
        for invalid in ["", "bogus", " ", "not-a-uuid"] {
            assert!(parse_only_event_id(Some(invalid)).is_err());
        }
    }
}
