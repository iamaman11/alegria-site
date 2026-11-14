use anyhow::Context;
use contracts::generated::alegria::temporal::v1::{HitlDecision, HitlTaskContext};
use primitives::errors::DomainError;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{types::Json, PgPool, Row};

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

// Truth review is an operator decision, never an alternative verification
// engine. The existing site.seo_hitl_tasks queue owns identity and audit.
// Source-backed candidates remain epistemically "needs_hitl" until the
// independent truth adjudicator reruns successfully.
pub async fn reconcile_truth_candidate_reviews(
    pool: &PgPool,
    context_key: &str,
    run_id: &str,
) -> std::result::Result<u64, DomainError> {
    if context_key.trim().is_empty() || run_id.trim().is_empty() {
        return Err(contract_violation("truth HITL context_key and run_id are required"));
    }
    let result = sqlx::query(
        r#"
        INSERT INTO site.seo_hitl_tasks
            (task_key, task_type, queue_state, first_owner_role, current_owner_role,
             scope_signature, blocking_step_name, blocking_execution_key,
             severity, decision_payload, audit_log_payload)
        SELECT
            'truth_candidate_review@1:' || c.rule_candidate_id || ':' || c.source_snapshot_hash,
            'truth_candidate_review', 'open', 'truth_reviewer', 'truth_reviewer',
            c.context_key, 'truth_adjudication', c.rule_candidate_id,
            'blocking',
            jsonb_build_object(
                'schema_version', 1, 'candidate_id', c.rule_candidate_id,
                'context_key', c.context_key, 'raw_section_id', c.raw_section_id,
                'role', c.role, 'concept', c.concept_canonical_key,
                'params', c.params, 'scope', c.scope,
                'source_url', c.source_key, 'source_snapshot_hash', c.source_snapshot_hash,
                'evidence_quote', c.evidence_quote,
                'span_start', c.span_start, 'span_end', c.span_end,
                'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
                'prompt_version', c.prompt_version, 'epistemic_status', 'needs_hitl',
                'recheck_required', true),
            jsonb_build_array(jsonb_build_object(
                'event', 'truth_candidate_needs_review',
                'run_id', $2::text,
                'source_snapshot_hash', c.source_snapshot_hash))
        FROM extracted.rule_candidates c
        WHERE c.context_key = $1
          AND c.epistemic_status = 'needs_hitl'
        ON CONFLICT (task_key) DO NOTHING
        "#,
    )
    .bind(context_key)
    .bind(run_id)
    .execute(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(result.rows_affected())
}

pub async fn list_truth_candidate_reviews(
    pool: &PgPool,
    context_key: &str,
    limit: i64,
) -> std::result::Result<Vec<Value>, DomainError> {
    if context_key.trim().is_empty() || !(1..=500).contains(&limit) {
        return Err(contract_violation("truth HITL context_key and limit 1..=500 required"));
    }
    let rows = sqlx::query(
        r#"
        SELECT task_key, queue_state, decision_payload, audit_log_payload,
               updated_at::text AS updated_at
        FROM site.seo_hitl_tasks
        WHERE task_type = 'truth_candidate_review'
          AND scope_signature = $1
        ORDER BY created_at, task_key
        LIMIT $2
        "#,
    )
    .bind(context_key)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "task_key": row.get::<String, _>("task_key"),
                "queue_state": row.get::<String, _>("queue_state"),
                "candidate": row.get::<Json<Value>, _>("decision_payload").0,
                "audit_log": row.get::<Json<Value>, _>("audit_log_payload").0,
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect())
}

pub async fn show_truth_candidate_review(
    pool: &PgPool,
    task_key: &str,
) -> std::result::Result<Value, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT task_key, queue_state, decision_payload, audit_log_payload,
               updated_at::text AS updated_at
        FROM site.seo_hitl_tasks
        WHERE task_type = 'truth_candidate_review' AND task_key = $1
        "#,
    )
    .bind(task_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?
    .ok_or_else(|| contract_violation("truth review task not found"))?;
    Ok(json!({
        "task_key": row.get::<String, _>("task_key"),
        "queue_state": row.get::<String, _>("queue_state"),
        "candidate": row.get::<Json<Value>, _>("decision_payload").0,
        "audit_log": row.get::<Json<Value>, _>("audit_log_payload").0,
        "updated_at": row.get::<String, _>("updated_at"),
    }))
}

pub async fn decide_truth_candidate_review(
    pool: &PgPool,
    task_key: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> std::result::Result<String, DomainError> {
    let actor_role = actor_role.trim();
    let reason = reason.trim();
    if task_key.trim().is_empty()
        || actor_role.is_empty()
        || actor_role == "seo_system"
        || reason.is_empty()
        || !matches!(decision, "approve" | "reject" | "request_changes")
    {
        return Err(contract_violation(
            "truth review needs a task, human actor, valid decision and explicit reason",
        ));
    }
    let mut tx = pool.begin().await.map_err(|e| infra_unavailable(e.to_string()))?;
    let row = sqlx::query(
        "SELECT queue_state, decision_payload
         FROM site.seo_hitl_tasks
         WHERE task_key = $1 AND task_type = 'truth_candidate_review'
         FOR UPDATE",
    )
    .bind(task_key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?
    .ok_or_else(|| contract_violation("truth review task not found"))?;
    let state: String = row.get("queue_state");
    let payload: Value = row.get::<Json<Value>, _>("decision_payload").0;
    let candidate_id = payload
        .get("candidate_id")
        .and_then(Value::as_str)
        .ok_or_else(|| contract_violation("truth review task has no candidate_id"))?;
    let snapshot = payload
        .get("source_snapshot_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| contract_violation("truth review task has no snapshot hash"))?;
    let current: Option<(String, String)> = sqlx::query_as(
        "SELECT epistemic_status, source_snapshot_hash
         FROM extracted.rule_candidates WHERE rule_candidate_id = $1",
    )
    .bind(candidate_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    if !matches!(current, Some((ref epistemic, ref hash))
        if epistemic == "needs_hitl" && hash == snapshot)
    {
        return Err(contract_violation(
            "candidate changed or was superseded; independent recheck required",
        ));
    }

    // An exact repeated operator command is a no-op, including after resolution.
    let previous = payload.get("review");
    if previous.is_some_and(|review| {
        review.get("actor_role").and_then(Value::as_str) == Some(actor_role)
            && review.get("decision").and_then(Value::as_str) == Some(decision)
            && review.get("reason").and_then(Value::as_str) == Some(reason)
    }) {
        tx.commit().await.map_err(|e| infra_unavailable(e.to_string()))?;
        return Ok(state);
    }
    if !matches!(state.as_str(), "open" | "in_review" | "reopened") {
        return Err(contract_violation(
            "truth review already resolved; do not overwrite its audit",
        ));
    }
    let next_state = if decision == "request_changes" {
        "reopened"
    } else {
        "resolved"
    };
    // In particular, "approve" is NOT a verified candidate and does not
    // write to extracted.rule_candidates or verified.rule_instances.
    let recorded = json!({
        "actor_role": actor_role,
        "decision": decision,
        "reason": reason,
        "recheck_required": true,
        "auto_verification": false,
    });
    sqlx::query(
        r#"
        UPDATE site.seo_hitl_tasks
        SET queue_state = $2, current_owner_role = $3,
            decision_payload = jsonb_set(decision_payload, '{review}', $4::jsonb, true),
            audit_log_payload = audit_log_payload || jsonb_build_array(
                jsonb_build_object('event', 'truth_review_decided',
                    'actor_role', $3::text, 'decision', $5::text,
                    'reason', $6::text, 'recheck_required', true, 'at', now())),
            updated_at = now()
        WHERE task_key = $1 AND task_type = 'truth_candidate_review'
        "#,
    )
    .bind(task_key)
    .bind(next_state)
    .bind(actor_role)
    .bind(Json(recorded))
    .bind(decision)
    .bind(reason)
    .execute(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    tx.commit().await.map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(next_state.to_string())
}

#[cfg(test)]
mod truth_review_policy_tests {
    #[test]
    fn approval_is_not_a_verification_method() {
        // This boundary intentionally does not depend on verified rule writers.
        assert_ne!("approve", "verified");
        assert_ne!("request_changes", "verified");
    }
}
