use anyhow::Context;
use contracts::generated::alegria::temporal::v1::{HitlDecision, HitlTaskContext};
use primitives::errors::DomainError;
use serde::Serialize;
use sqlx::PgPool;

fn contract_violation(message: impl Into<String>) -> DomainError {
    DomainError::ContractViolation {
        message: message.into(),
    }
}

fn infra_unavailable(message: impl Into<String>) -> DomainError {
    DomainError::InfraUnavailable {
        message: message.into(),
    }
}

pub async fn enqueue_hitl_task(
    pool: &PgPool,
    task_type: &str,
    diagnostics: &HitlTaskContext,
    priority: i32,
) -> std::result::Result<i64, DomainError> {
    let diagnostics_json = serde_json::to_string(diagnostics)
        .map_err(|e| contract_violation(format!("failed to serialize HITL context: {e}")))?;
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO pipeline.hitl_tasks
         (task_type, context, priority, status)
         VALUES ($1, $2::jsonb, $3, 'pending')
         RETURNING id",
    )
    .bind(task_type)
    .bind(diagnostics_json)
    .bind(priority)
    .fetch_one(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(row.0)
}

pub async fn resolve_hitl_task(
    pool: &PgPool,
    task_id: i64,
    resolution: &HitlDecision,
) -> std::result::Result<(), DomainError> {
    let resolution_json = serde_json::to_string(resolution)
        .map_err(|e| contract_violation(format!("failed to serialize HITL resolution: {e}")))?;
    sqlx::query(
        "UPDATE pipeline.hitl_tasks
         SET status          = 'resolved',
             context         = jsonb_set(COALESCE(context, '{}'::jsonb), '{resolution}', $1::jsonb, true),
             updated_at      = now()
         WHERE id = $2",
    )
    .bind(resolution_json)
    .bind(task_id)
    .execute(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(())
}

pub async fn write_context_fragment<T: Serialize>(
    pool: &PgPool,
    task_id: i64,
    key: &str,
    payload: &T,
) -> std::result::Result<(), DomainError> {
    let json = serde_json::to_string(payload).map_err(|e| {
        contract_violation(format!("failed to serialize HITL context fragment: {e}"))
    })?;
    sqlx::query(
        "UPDATE pipeline.hitl_tasks
         SET context = jsonb_set(COALESCE(context, '{}'::jsonb), $1::text[], $2::jsonb, true),
             updated_at = now()
         WHERE id = $3",
    )
    .bind(vec![key])
    .bind(json)
    .bind(task_id)
    .execute(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(())
}

pub async fn count_open_hitl_tasks(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT count(*)::bigint FROM pipeline.hitl_tasks WHERE status = 'pending'")
        .fetch_one(pool)
        .await
        .context("count open hitl tasks")
}
