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
        return Err(contract_violation(
            "truth HITL context_key and run_id are required",
        ));
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| infra_unavailable(e.to_string()))?;
    // Close obsolete open reviews only AFTER the canonical candidate's
    // independent status or exact source snapshot has changed. The audit is
    // append-only; human approval on its own cannot close a truth gap.
    sqlx::query(
        r#"
        UPDATE site.seo_hitl_tasks AS t
        SET queue_state = 'superseded',
            audit_log_payload = t.audit_log_payload || jsonb_build_array(
                jsonb_build_object('event', 'truth_candidate_rechecked',
                    'candidate_status', c.epistemic_status,
                    'run_id', $2::text, 'at', now())),
            updated_at = now()
        FROM extracted.rule_candidates AS c
        WHERE t.task_type = 'truth_candidate_review'
          AND t.scope_signature = $1
          AND t.blocking_execution_key = c.rule_candidate_id
          AND t.queue_state IN ('open', 'in_review', 'reopened')
          AND (
              (t.decision_payload->>'source_snapshot_hash')
                  IS DISTINCT FROM c.source_snapshot_hash
              OR (c.epistemic_status <> 'needs_hitl'
                  AND NOT (
                      c.epistemic_status = 'rejected'
                      AND t.decision_payload->'review'->>'decision' = 'reject'
                  ))
          )
        "#,
    )
    .bind(context_key)
    .bind(run_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    // A candidate can become unresolved again after an independent recheck,
    // or the extraction projection can change while source bytes stay fixed.
    // Reopen the SAME task; keep the immutable audit, discard the stale review
    // verdict and refresh the candidate payload. No duplicate queue owner.
    sqlx::query(
        r#"
        UPDATE site.seo_hitl_tasks AS t
        SET queue_state = 'open',
            current_owner_role = 'truth_reviewer',
            decision_payload = (
                t.decision_payload || jsonb_build_object(
                    'raw_section_id', c.raw_section_id,
                    'role', c.role,
                    'concept', c.concept_canonical_key,
                    'params', c.params, 'scope', c.scope,
                    'source_url', c.source_key,
                    'source_snapshot_hash', c.source_snapshot_hash,
                    'evidence_quote', c.evidence_quote,
                    'span_start', c.span_start, 'span_end', c.span_end,
                    'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
                    'prompt_version', c.prompt_version,
                    'epistemic_status', 'needs_hitl', 'recheck_required', true)
            ) - 'review',
            audit_log_payload = t.audit_log_payload || jsonb_build_array(
                jsonb_build_object('event', 'truth_candidate_reopened_for_review',
                    'run_id', $2::text, 'at', now())),
            updated_at = now()
        FROM extracted.rule_candidates AS c
        WHERE t.task_key = 'truth_candidate_review@1:' ||
                c.rule_candidate_id || ':' || c.source_snapshot_hash
          AND t.task_type = 'truth_candidate_review'
          AND t.scope_signature = $1
          AND c.context_key = $1
          AND c.epistemic_status = 'needs_hitl'
          AND (
              t.queue_state = 'superseded'
              OR (t.decision_payload->>'raw_section_id')::bigint
                  IS DISTINCT FROM c.raw_section_id
              OR (t.decision_payload->>'role') IS DISTINCT FROM c.role
              OR (t.decision_payload->>'concept') IS DISTINCT FROM c.concept_canonical_key
              OR t.decision_payload->'params' IS DISTINCT FROM c.params
              OR t.decision_payload->'scope' IS DISTINCT FROM c.scope
              OR (t.decision_payload->>'source_url') IS DISTINCT FROM c.source_key
              OR (t.decision_payload->>'evidence_quote') IS DISTINCT FROM c.evidence_quote
              OR (t.decision_payload->>'span_start')::integer IS DISTINCT FROM c.span_start
              OR (t.decision_payload->>'span_end')::integer IS DISTINCT FROM c.span_end
              OR (t.decision_payload->>'llm_provider') IS DISTINCT FROM c.llm_provider
              OR (t.decision_payload->>'llm_model') IS DISTINCT FROM c.llm_model
              OR (t.decision_payload->>'prompt_version') IS DISTINCT FROM c.prompt_version
          )
        "#,
    )
    .bind(context_key)
    .bind(run_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
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
    .execute(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(result.rows_affected())
}

/// A rejection is a conservative input to the *existing* canonical
/// adjudication. Approval is intentionally not returned as verification.
pub async fn load_rejected_truth_review_payloads(
    pool: &PgPool,
    context_key: &str,
) -> std::result::Result<Vec<Value>, DomainError> {
    if context_key.trim().is_empty() {
        return Err(contract_violation("truth review context_key required"));
    }
    let rows = sqlx::query(
        r#"
        SELECT t.decision_payload
        FROM site.seo_hitl_tasks AS t
        JOIN extracted.rule_candidates AS c
          ON c.rule_candidate_id = t.blocking_execution_key
         AND c.context_key = t.scope_signature
        JOIN raw.sections AS s ON s.id = c.raw_section_id
        WHERE t.scope_signature = $1
          AND t.task_type = 'truth_candidate_review'
          AND t.queue_state = 'in_review'
          AND t.decision_payload->'review'->>'decision' = 'reject'
          AND c.epistemic_status = 'needs_hitl'
          AND c.source_snapshot_hash = t.decision_payload->>'source_snapshot_hash'
          AND s.content_hash = c.source_snapshot_hash
        "#,
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| row.get::<Json<Value>, _>("decision_payload").0)
        .collect())
}

/// Show bounded evidence context, never duplicate the original raw section
/// into the persisted review task. Byte offsets stay UTF-8 exact.
fn reviewer_evidence_excerpt(raw: &str, start: usize, end: usize) -> Option<String> {
    let matched = raw.get(start..end)?;
    let prefix = raw
        .get(..start)?
        .chars()
        .rev()
        .take(180)
        .collect::<Vec<_>>();
    let before = prefix.into_iter().rev().collect::<String>();
    let after = raw.get(end..)?.chars().take(220).collect::<String>();
    Some(format!("{before}{matched}{after}"))
}

pub async fn list_truth_candidate_reviews(
    pool: &PgPool,
    context_key: &str,
    limit: i64,
) -> std::result::Result<Vec<Value>, DomainError> {
    if context_key.trim().is_empty() || !(1..=500).contains(&limit) {
        return Err(contract_violation(
            "truth HITL context_key and limit 1..=500 required",
        ));
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
    let payload = row.get::<Json<Value>, _>("decision_payload").0;
    let candidate_id = payload
        .get("candidate_id")
        .and_then(Value::as_str)
        .ok_or_else(|| contract_violation("review candidate_id missing"))?;
    let section = sqlx::query(
        r#"
        SELECT s.content_md, s.content_hash, k.source_type
        FROM extracted.rule_candidates AS c
        JOIN raw.sections AS s ON s.id = c.raw_section_id
        LEFT JOIN kb.sources AS k ON k.source_key = c.source_key
        WHERE c.rule_candidate_id = $1
        "#,
    )
    .bind(candidate_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;
    let evidence_context = section.map(|section| {
        let raw: String = section.get("content_md");
        let hash: String = section.get("content_hash");
        let start = payload.get("span_start").and_then(Value::as_u64);
        let end = payload.get("span_end").and_then(Value::as_u64);
        let excerpt = start
            .zip(end)
            .and_then(|(a, b)| reviewer_evidence_excerpt(&raw, a as usize, b as usize));
        let exact = start
            .zip(end)
            .and_then(|(a, b)| raw.get(a as usize..b as usize))
            .is_some_and(|span| {
                payload.get("evidence_quote").and_then(Value::as_str) == Some(span.trim())
            });
        json!({
            "excerpt": excerpt,
            "source_type": section.get::<Option<String>, _>("source_type"),
            "snapshot_matches": payload.get("source_snapshot_hash").and_then(Value::as_str)
                == Some(hash.as_str()),
            "quote_span_matches": exact,
        })
    });
    Ok(json!({
        "task_key": row.get::<String, _>("task_key"),
        "queue_state": row.get::<String, _>("queue_state"),
        "candidate": payload,
        "evidence_context": evidence_context,
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
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| infra_unavailable(e.to_string()))?;
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
    // A stable source hash does not prove that the extracted proposition is
    // unchanged: parser revisions can replace the quote, scope and params.
    // Lock the current candidate and original section for this decision.
    let current = sqlx::query(
        r#"
        SELECT c.epistemic_status, c.source_snapshot_hash, c.source_key,
               c.role, c.concept_canonical_key, c.params, c.scope,
               c.evidence_quote, c.span_start, c.span_end,
               c.llm_provider, c.llm_model, c.prompt_version,
               s.content_hash, s.content_md
        FROM extracted.rule_candidates AS c
        JOIN raw.sections AS s ON s.id = c.raw_section_id
        WHERE c.rule_candidate_id = $1 AND c.context_key = $2
        FOR SHARE OF c, s
        "#,
    )
    .bind(candidate_id)
    .bind(
        payload
            .get("context_key")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?
    .ok_or_else(|| contract_violation("review source candidate no longer exists"))?;
    let current_hash: String = current.get("source_snapshot_hash");
    let section_hash: String = current.get("content_hash");
    let section_text: String = current.get("content_md");
    let span_start: i32 = current.get("span_start");
    let span_end: i32 = current.get("span_end");
    let evidence_span = if span_start >= 0 && span_end > span_start {
        section_text.get(span_start as usize..span_end as usize)
    } else {
        None
    };
    let candidate_unchanged = current.get::<String, _>("epistemic_status") == "needs_hitl"
        && current_hash == snapshot
        && section_hash == snapshot
        && payload.get("source_url").and_then(Value::as_str)
            == Some(current.get::<String, _>("source_key").as_str())
        && payload.get("role").and_then(Value::as_str)
            == Some(current.get::<String, _>("role").as_str())
        && payload.get("concept").and_then(Value::as_str)
            == Some(current.get::<String, _>("concept_canonical_key").as_str())
        && payload.get("params") == Some(&current.get::<Json<Value>, _>("params").0)
        && payload.get("scope") == Some(&current.get::<Json<Value>, _>("scope").0)
        && payload.get("span_start").and_then(Value::as_i64) == Some(span_start as i64)
        && payload.get("span_end").and_then(Value::as_i64) == Some(span_end as i64)
        && payload.get("evidence_quote").and_then(Value::as_str)
            == Some(current.get::<String, _>("evidence_quote").as_str())
        && evidence_span.is_some_and(|span| {
            Some(span.trim()) == payload.get("evidence_quote").and_then(Value::as_str)
        })
        && payload.get("llm_provider").and_then(Value::as_str)
            == Some(current.get::<String, _>("llm_provider").as_str())
        && payload.get("llm_model").and_then(Value::as_str)
            == Some(current.get::<String, _>("llm_model").as_str())
        && payload.get("prompt_version").and_then(Value::as_str)
            == Some(current.get::<String, _>("prompt_version").as_str());
    if !candidate_unchanged {
        return Err(contract_violation(
            "reviewed candidate, original section or source claim changed; re-review required",
        ));
    }

    // An exact repeated operator command is a no-op, including after resolution.
    let previous = payload.get("review");
    if previous.is_some_and(|review| {
        review.get("actor_role").and_then(Value::as_str) == Some(actor_role)
            && review.get("decision").and_then(Value::as_str) == Some(decision)
            && review.get("reason").and_then(Value::as_str) == Some(reason)
    }) {
        tx.commit()
            .await
            .map_err(|e| infra_unavailable(e.to_string()))?;
        return Ok(state);
    }
    if !matches!(state.as_str(), "open" | "in_review" | "reopened")
        || previous.is_some_and(|review| {
            matches!(
                review.get("decision").and_then(Value::as_str),
                Some("approve" | "reject")
            )
        })
    {
        return Err(contract_violation(
            "truth review already decided; do not overwrite its audit",
        ));
    }
    let next_state = if decision == "request_changes" {
        "reopened"
    } else {
        // Remain actionable until independent truth adjudication changes the
        // candidate or its source snapshot; no human vote grants VERIFIED.
        "in_review"
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
    tx.commit()
        .await
        .map_err(|e| infra_unavailable(e.to_string()))?;
    Ok(next_state.to_string())
}
