use contracts::generated::alegria::temporal::v1::{HitlDecision, HitlTaskContext};
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::sqlx_hitl_adapter;
use primitives::errors::DomainError;
use serde::Serialize;

/// Поставить задачу HITL в очередь.
/// task_type: 'fact_conflict' | 'low_confidence' | 'manual_review'
pub async fn enqueue_hitl_task(
    pool: &AlegriaPgPool,
    task_type: &str,
    diagnostics: &HitlTaskContext,
    priority: i32,
) -> std::result::Result<i64, DomainError> {
    sqlx_hitl_adapter::enqueue_hitl_task(pool, task_type, diagnostics, priority).await
}

/// Завершить задачу HITL с резолюцией.
pub async fn resolve_hitl_task(
    pool: &AlegriaPgPool,
    task_id: i64,
    resolution: &HitlDecision,
) -> std::result::Result<(), DomainError> {
    sqlx_hitl_adapter::resolve_hitl_task(pool, task_id, resolution).await
}

pub async fn write_context_fragment<T: Serialize>(
    pool: &AlegriaPgPool,
    task_id: i64,
    key: &str,
    payload: &T,
) -> std::result::Result<(), DomainError> {
    sqlx_hitl_adapter::write_context_fragment(pool, task_id, key, payload).await
}
