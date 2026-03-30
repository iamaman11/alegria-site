use anyhow::Result;

use crate::reconcile_graph::{reconcile_target_system, ReconcileOptions, ReconcileReport};
use infrastructure::adapters::sqlx_reconcile_adapter;

pub async fn reconcile_qdrant() -> Result<ReconcileReport> {
    let opts = ReconcileOptions::default();
    reconcile_target_system("qdrant", &opts).await
}

pub async fn read_qdrant_backlog() -> Result<(i64, i64)> {
    sqlx_reconcile_adapter::read_target_backlog_default("qdrant").await
}
