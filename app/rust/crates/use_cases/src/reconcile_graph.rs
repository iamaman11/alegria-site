use anyhow::Result;
use serde::{Deserialize, Serialize};

use infrastructure::adapters::sqlx_reconcile_adapter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileReport {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    pub max_retry_count: i32,
    pub batch_limit: i64,
    pub dry_run: bool,
    pub requeue_base_delay_sec: i64,
    pub requeue_jitter_sec: i64,
}

impl Default for ReconcileOptions {
    fn default() -> Self {
        let record = sqlx_reconcile_adapter::load_default_reconcile_options();
        Self {
            max_retry_count: record.max_retry_count,
            batch_limit: record.batch_limit,
            dry_run: record.dry_run,
            requeue_base_delay_sec: record.requeue_base_delay_sec,
            requeue_jitter_sec: record.requeue_jitter_sec,
        }
    }
}

pub async fn reconcile_target_system(target_system: &str, opts: &ReconcileOptions) -> Result<ReconcileReport> {
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(
        target_system,
        &sqlx_reconcile_adapter::ReconcileOptionsRecord {
            max_retry_count: opts.max_retry_count,
            batch_limit: opts.batch_limit,
            dry_run: opts.dry_run,
            requeue_base_delay_sec: opts.requeue_base_delay_sec,
            requeue_jitter_sec: opts.requeue_jitter_sec,
        },
    )
    .await?;
    Ok(ReconcileReport {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub async fn reconcile_graph() -> Result<ReconcileReport> {
    let opts = ReconcileOptions::default();
    reconcile_target_system("neo4j", &opts).await
}

pub async fn read_graph_backlog() -> Result<(i64, i64)> {
    sqlx_reconcile_adapter::read_target_backlog_default("neo4j").await
}
