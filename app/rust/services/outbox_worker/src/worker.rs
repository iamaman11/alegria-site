use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::time::timeout;
use tracing::{error, info, warn};
use uuid::Uuid;

use infrastructure::adapters::sqlx_outbox_adapter::{
    claim_outbox_batch, mark_done, mark_failed_with_dead_letter, mark_retry, read_sync_status,
    LeaseUpdate,
};
use telemetry::fbs_writer::write_runtime_window;

use crate::materialize::dispatch_event;
use crate::retry::next_retry_delay_sec;

const DEFAULT_BATCH_SIZE: i64 = 64;
const DEFAULT_LEASE_SECONDS: i64 = 120;
const DEFAULT_IDLE_POLL_MS: u64 = 1500;
const DEFAULT_MAX_RETRIES: i32 = 10;
const OUTBOX_NOTIFY_CHANNEL: &str = "sync_outbox_channel";

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
    let only_event_id = std::env::var("OUTBOX_ONLY_EVENT_ID")
        .ok()
        .and_then(|value| Uuid::parse_str(&value).ok());
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

        let batch = claim_outbox_batch(
            pool,
            worker_id,
            DEFAULT_BATCH_SIZE,
            DEFAULT_LEASE_SECONDS,
            only_event_id,
        )
        .await?;
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

            match dispatch_event(
                &event.target_system,
                &event.event_type,
                &event.aggregate_key,
                &event.payload_type,
                &event.payload_bytes,
            )
            .await
            {
                Ok(_) => {
                    if mark_done(pool, &event).await? == LeaseUpdate::Lost {
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

    Ok(())
}
