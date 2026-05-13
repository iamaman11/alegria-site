use contracts::generated::alegria::temporal::v1::{HitlDecision, HitlTaskContext};
use primitives::errors::DomainError;
use seo_ports::HitlQueuePort;

pub async fn enqueue_hitl_task<P: HitlQueuePort>(
    port: &P,
    task_type: &str,
    diagnostics: &HitlTaskContext,
    priority: i32,
) -> Result<i64, DomainError> {
    port.enqueue_hitl_task(task_type, diagnostics, priority)
        .await
}

pub async fn resolve_hitl_task<P: HitlQueuePort>(
    port: &P,
    task_id: i64,
    resolution: &HitlDecision,
) -> Result<(), DomainError> {
    port.resolve_hitl_task(task_id, resolution).await
}
