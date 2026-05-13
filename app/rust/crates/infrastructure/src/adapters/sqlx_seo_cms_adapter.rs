use std::collections::HashMap;

use contracts::generated::alegria::sync::v1::SeoCmsEventPayload;
use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, PublishArtifact,
    PublishMaterializeInputPayload, PublishMaterializeOutputPayload, RenderPreviewPageState,
};
use primitives::errors::DomainError;
use prost::Message;
use seo_ports::{CmsReviewDecisionOutcome, CmsReviewDecisionRequest};
use serde_json::json;
use sqlx::{types::Json, PgPool, Row};
use std::path::Path;

use super::proto_runtime_payload_store::{classify_sqlx, validation_failure};
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;
use super::static_site_builder_adapter;

#[derive(Debug, Clone)]
struct PublishDraftRow {
    page_draft_key: String,
    page_node_key: String,
    page_brief_key: String,
    draft_revision: i32,
    body_markdown: String,
    qa_verdict: String,
    truth_snapshot_ref: String,
    title: String,
    meta_description: String,
    blueprint_key: String,
    scope_signature: String,
    canonical_url_path: String,
    canonical_slug: String,
    page_type_key: String,
    dominant_intent: String,
    traceability_manifest: serde_json::Value,
}

#[derive(Debug, Clone)]
struct SeoPublishRuntimeConfig {
    output_dir: String,
    base_url: String,
    allow_full_rebuild_fallback: bool,
}

fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

fn make_idempotency_key(aggregate_key: &str, event_type: &str, payload_bytes: &[u8]) -> String {
    let payload_hash = primitives::hash::blake3_hex(payload_bytes);
    primitives::hash::content_hash_v1(&format!("{aggregate_key}|{event_type}|{payload_hash}"))
}

fn cms_event_outbox(event: &SeoCmsEventPayload) -> OutboxEnvelope {
    let payload_bytes = encode_payload(event);
    OutboxEnvelope {
        aggregate_type: "cms_page".to_string(),
        aggregate_key: event.page_node_key.clone(),
        target_system: "cms".to_string(),
        event_type: event.event_type.clone(),
        payload_type: "alegria.outbox.seo_cms_event.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(&event.event_key, &event.event_type, &payload_bytes),
        payload_bytes,
    }
}

fn hitl_task_type(blocker: &str) -> &'static str {
    match blocker {
        "active_cannibalization_conflict" => "cannibalization_conflict",
        "canonical_url_conflict" => "canonical_url_conflict",
        "unsupported_factual_fragment" => "unsupported_factual_fragment",
        _ => "publish_gate_blocker",
    }
}

fn seo_publish_runtime_config(input: &PublishMaterializeInputPayload) -> SeoPublishRuntimeConfig {
    let output_dir = if input.output_dir.trim().is_empty() {
        std::env::var("SEO_STATIC_OUTPUT_DIR")
            .unwrap_or_else(|_| "app/rust/dist/static-site".to_string())
    } else {
        input.output_dir.clone()
    };
    let base_url = if input.base_url.trim().is_empty() {
        std::env::var("SEO_STATIC_BASE_URL").unwrap_or_else(|_| "https://example.com".to_string())
    } else {
        input.base_url.clone()
    };
    let allow_full_rebuild_fallback = std::env::var("SEO_PUBLISH_ALLOW_FULL_REBUILD_FALLBACK")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    SeoPublishRuntimeConfig {
        output_dir,
        base_url,
        allow_full_rebuild_fallback,
    }
}

async fn load_publish_draft(
    pool: &PgPool,
    page_draft_key: &str,
) -> Result<Option<PublishDraftRow>, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT
            d.page_draft_key,
            d.page_node_key,
            d.page_brief_key,
            d.draft_revision,
            d.body_markdown,
            d.qa_verdict,
            d.truth_snapshot_ref,
            b.title,
            b.meta_description,
            b.blueprint_key,
            p.scope_signature,
            p.canonical_url_path,
            p.canonical_slug,
            p.page_type_key,
            p.dominant_intent,
            d.traceability_manifest
        FROM site.page_drafts d
        JOIN site.page_briefs b ON b.page_brief_key = d.page_brief_key
        JOIN site.page_nodes p ON p.page_node_key = d.page_node_key
        WHERE d.page_draft_key = $1
        "#,
    )
    .bind(page_draft_key)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(row.map(|r| {
        let traceability_manifest: Json<serde_json::Value> = r.get("traceability_manifest");
        PublishDraftRow {
            page_draft_key: r.get("page_draft_key"),
            page_node_key: r.get("page_node_key"),
            page_brief_key: r.get("page_brief_key"),
            draft_revision: r.get("draft_revision"),
            body_markdown: r.get("body_markdown"),
            qa_verdict: r.get("qa_verdict"),
            truth_snapshot_ref: r.get("truth_snapshot_ref"),
            title: r.get("title"),
            meta_description: r.get("meta_description"),
            blueprint_key: r.get("blueprint_key"),
            scope_signature: r.get("scope_signature"),
            canonical_url_path: r.get("canonical_url_path"),
            canonical_slug: r.get("canonical_slug"),
            page_type_key: r.get("page_type_key"),
            dominant_intent: r.get("dominant_intent"),
            traceability_manifest: traceability_manifest.0,
        }
    }))
}

async fn count_active_cannibalization(
    pool: &PgPool,
    page_node_key: &str,
) -> Result<i64, DomainError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)::bigint
        FROM site.cannibalization_conflicts
        WHERE status = 'open'
          AND severity IN ('high','blocking')
          AND (page_key_a = $1 OR page_key_b = $1)
        "#,
    )
    .bind(page_node_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)
}

async fn count_required_links(pool: &PgPool, page_node_key: &str) -> Result<i64, DomainError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)::bigint
        FROM site.link_recommendations
        WHERE source_page_key = $1
          AND required_flag = true
          AND status IN ('candidate','accepted','applied')
        "#,
    )
    .bind(page_node_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)
}

async fn count_canonical_conflicts(
    pool: &PgPool,
    row: &PublishDraftRow,
) -> Result<i64, DomainError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)::bigint
        FROM site.cms_pages
        WHERE scope_signature = $1
          AND canonical_url_path = $2
          AND page_node_key <> $3
          AND current_status <> 'deprecated'
        "#,
    )
    .bind(&row.scope_signature)
    .bind(&row.canonical_url_path)
    .bind(&row.page_node_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)
}

fn approval_decision_valid(
    decision: &CmsApprovalDecision,
    row: &PublishDraftRow,
    revision_id: &str,
) -> bool {
    decision.decision == "approved"
        && decision.page_node_key == row.page_node_key
        && decision.revision_id == revision_id
        && !decision.actor_role.trim().is_empty()
        && decision.actor_role != "seo_system"
}

async fn persist_approval_decision(
    pool: &PgPool,
    decision: &CmsApprovalDecision,
) -> Result<(), DomainError> {
    sqlx::query(
        r#"
        INSERT INTO site.cms_approval_decisions
            (decision_key, page_node_key, revision_id, actor_role, decision, reason,
             decided_at, decision_payload)
        VALUES ($1, $2, $3, $4, $5, $6,
                COALESCE(NULLIF($7, '')::timestamptz, now()), $8)
        ON CONFLICT (decision_key) DO UPDATE
        SET decision = EXCLUDED.decision,
            reason = EXCLUDED.reason,
            decision_payload = EXCLUDED.decision_payload
        "#,
    )
    .bind(&decision.decision_key)
    .bind(&decision.page_node_key)
    .bind(&decision.revision_id)
    .bind(&decision.actor_role)
    .bind(&decision.decision)
    .bind(&decision.reason)
    .bind(&decision.decided_at)
    .bind(Json(json!({
        "actor_role": decision.actor_role.clone(),
        "reason": decision.reason.clone(),
    })))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn load_latest_approval_decision(
    pool: &PgPool,
    page_node_key: &str,
    revision_id: &str,
) -> Result<Option<CmsApprovalDecision>, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT decision_key, page_node_key, revision_id, actor_role, decision, reason,
               decided_at::text AS decided_at
        FROM site.cms_approval_decisions
        WHERE page_node_key = $1
          AND revision_id = $2
        ORDER BY decided_at DESC NULLS LAST, created_at DESC
        LIMIT 1
        "#,
    )
    .bind(page_node_key)
    .bind(revision_id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(row.map(|row| CmsApprovalDecision {
        decision_key: row.get("decision_key"),
        page_node_key: row.get("page_node_key"),
        revision_id: row.get("revision_id"),
        actor_role: row.get("actor_role"),
        decision: row.get("decision"),
        reason: row.get("reason"),
        decided_at: row.get("decided_at"),
    }))
}

async fn resolve_seo_workflow_id(
    pool: &PgPool,
    page_node_key: &str,
    revision_id: &str,
) -> Result<Option<String>, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT event_payload ->> 'run_id' AS run_id
        FROM site.cms_publish_events
        WHERE page_node_key = $1
          AND revision_id = $2
          AND event_type = 'seo_page_review_requested'
          AND COALESCE(event_payload ->> 'run_id', '') <> ''
        ORDER BY occurred_at DESC
        LIMIT 1
        "#,
    )
    .bind(page_node_key)
    .bind(revision_id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(row.and_then(|row| row.get::<Option<String>, _>("run_id")))
}

pub async fn apply_human_review_decision(
    pool: &PgPool,
    request: &CmsReviewDecisionRequest,
) -> Result<CmsReviewDecisionOutcome, DomainError> {
    let actor_role = request.actor_role.trim();
    if actor_role.is_empty() || actor_role == "seo_system" {
        return Err(validation_failure("human actor_role is required"));
    }

    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.current_revision_id AS revision_id
        FROM site.cms_pages p
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(&request.page_node_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let revision_id: String = row.get("revision_id");
    let decision_key = primitives::seo::seo_artifact_key(
        "cms_approval_decision",
        &[
            &request.page_node_key,
            &revision_id,
            actor_role,
            &request.decision,
        ],
    );
    sqlx::query(
        r#"
        INSERT INTO site.cms_approval_decisions
            (decision_key, page_node_key, revision_id, actor_role, decision, reason,
             decision_payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (decision_key) DO UPDATE
        SET decision = EXCLUDED.decision,
            reason = EXCLUDED.reason,
            decision_payload = EXCLUDED.decision_payload
        "#,
    )
    .bind(&decision_key)
    .bind(&request.page_node_key)
    .bind(&revision_id)
    .bind(actor_role)
    .bind(&request.decision)
    .bind(&request.reason)
    .bind(Json(json!({
        "reason": request.reason.clone(),
        "source": request.source.clone(),
    })))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let queue_state = match request.decision.as_str() {
        "approved" => "resolved",
        "blocked" => "reopened",
        "reopened" => "reopened",
        other => return Err(validation_failure(format!("unsupported decision: {other}"))),
    };
    sqlx::query(
        r#"
        UPDATE site.seo_hitl_tasks
        SET queue_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
          AND blocking_execution_key = $3
          AND queue_state IN ('open','in_review','reopened')
        "#,
    )
    .bind(queue_state)
    .bind(&request.page_node_key)
    .bind(&revision_id)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let workflow_id = resolve_seo_workflow_id(pool, &request.page_node_key, &revision_id)
        .await?
        .ok_or_else(|| {
            validation_failure("seo workflow run_id not found for current review revision")
        })?;

    Ok(CmsReviewDecisionOutcome {
        decision_key,
        revision_id,
        workflow_id,
    })
}

pub async fn persist_cms_publish_output(
    pool: &PgPool,
    input: &CmsPublishInputPayload,
    planned: &CmsPublishOutputPayload,
) -> Result<CmsPublishOutputPayload, DomainError> {
    if planned.page_draft_key.trim().is_empty() {
        return Err(validation_failure(
            "cms_publish persistence requires page_draft_key",
        ));
    }
    let Some(row) = load_publish_draft(pool, &planned.page_draft_key).await? else {
        return Err(validation_failure(format!(
            "page_draft not found for cms_publish: {}",
            planned.page_draft_key
        )));
    };

    let mut blockers = planned.blocking_reasons.clone();
    if row.qa_verdict != "publish_ready" {
        blockers.push("failing_draft_qa_verdict".to_string());
    }
    if row.truth_snapshot_ref.trim().is_empty() {
        blockers.push("missing_traceability_manifest".to_string());
    }
    if count_active_cannibalization(pool, &row.page_node_key).await? > 0 {
        blockers.push("active_cannibalization_conflict".to_string());
    }
    if count_required_links(pool, &row.page_node_key).await? == 0 {
        blockers.push("required_link_obligations_unsatisfied".to_string());
    }
    if count_canonical_conflicts(pool, &row).await? > 0 {
        blockers.push("canonical_url_conflict".to_string());
    }

    let revision_id = if planned.revision_id.trim().is_empty() {
        primitives::seo::seo_artifact_key(
            "cms_revision",
            &[
                &row.page_node_key,
                &row.page_draft_key,
                &row.draft_revision.to_string(),
                "seo_cms_publish@1",
            ],
        )
    } else {
        planned.revision_id.clone()
    };
    let cms_document_id = if planned.cms_document_id.trim().is_empty() {
        primitives::seo::seo_artifact_key("cms_document", &[&row.page_node_key])
    } else {
        planned.cms_document_id.clone()
    };
    let valid_human_approval = input
        .approval_decision
        .as_ref()
        .map(|decision| approval_decision_valid(decision, &row, &revision_id))
        .unwrap_or(false);
    if input.publish_mode == "approved_publish" && !valid_human_approval {
        blockers.push("missing_persisted_human_approval".to_string());
    }
    blockers.sort();
    blockers.dedup();
    let (current_status, page_lifecycle_state, revision_status, event_type, verdict) =
        if blockers.is_empty() {
            match input.publish_mode.as_str() {
                "approved_publish" => (
                    "approved",
                    "approved",
                    "approved",
                    "seo_page_approved",
                    "approved",
                ),
                _ => (
                    "review_required",
                    "review_required",
                    "review_required",
                    "seo_page_review_requested",
                    "review_requested",
                ),
            }
        } else {
            (
                "blocked",
                "blocked",
                "blocked",
                "seo_page_publish_blocked",
                "blocked",
            )
        };

    sqlx::query(
        r#"
        INSERT INTO site.cms_page_revisions
            (revision_id, page_node_key, page_draft_key, page_brief_key, title,
             meta_description, h1, body_payload, faq_payload, schema_markup_payload,
             required_link_payload, traceability_manifest, blueprint_key, qa_report_key,
             freshness_class, review_owner_role, revision_reason, revision_status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '[]'::jsonb, $9, $10, $11,
                $12, $13, 'fresh', 'seo_editor', 'seo_publish_gate', $14)
        ON CONFLICT (revision_id) DO UPDATE
        SET body_payload = EXCLUDED.body_payload,
            revision_status = EXCLUDED.revision_status,
            updated_at = now()
        "#,
    )
    .bind(&revision_id)
    .bind(&row.page_node_key)
    .bind(&row.page_draft_key)
    .bind(&row.page_brief_key)
    .bind(&row.title)
    .bind(&row.meta_description)
    .bind(&row.title)
    .bind(Json(json!({
        "markdown": row.body_markdown.clone(),
        "content_blocks": row
            .traceability_manifest
            .get("content_blocks")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "claim_ledger": row
            .traceability_manifest
            .get("claim_ledger")
            .cloned()
            .unwrap_or_else(|| json!([])),
    })))
    .bind(Json(json!({
        "@type": "Article",
        "headline": row.title.clone(),
        "url": row.canonical_url_path.clone(),
    })))
    .bind(Json(json!([])))
    .bind(Json(json!({
        "truth_snapshot_ref": row.truth_snapshot_ref.clone(),
        "page_draft_key": row.page_draft_key.clone(),
        "traceability_manifest": row.traceability_manifest.clone(),
    })))
    .bind(&row.blueprint_key)
    .bind(format!("draft_qa:{}", row.page_draft_key))
    .bind(revision_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if input.publish_mode == "approved_publish" && valid_human_approval {
        if let Some(decision) = input.approval_decision.as_ref() {
            persist_approval_decision(pool, decision).await?;
        }
    }

    sqlx::query(
        r#"
        INSERT INTO site.cms_pages
            (page_node_key, scope_signature, canonical_url_path, locale_code,
             page_type_key, dominant_intent_key, cms_document_id,
             current_revision_id, current_status, published_at)
        VALUES ($1, $2, $3, 'und', $4, $5, $6, $7, $8,
                CASE WHEN $8 = 'published' THEN now() ELSE NULL END)
        ON CONFLICT (page_node_key) DO UPDATE
        SET canonical_url_path = EXCLUDED.canonical_url_path,
            current_revision_id = EXCLUDED.current_revision_id,
            current_status = EXCLUDED.current_status,
            published_at = CASE
                WHEN EXCLUDED.current_status = 'published'
                THEN COALESCE(site.cms_pages.published_at, now())
                ELSE site.cms_pages.published_at
            END,
            updated_at = now()
        "#,
    )
    .bind(&row.page_node_key)
    .bind(&row.scope_signature)
    .bind(&row.canonical_url_path)
    .bind(&row.page_type_key)
    .bind(&row.dominant_intent)
    .bind(&cms_document_id)
    .bind(&revision_id)
    .bind(current_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(page_lifecycle_state)
    .bind(&row.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let event_key = primitives::seo::seo_artifact_key(
        "cms_event",
        &[
            &row.page_node_key,
            &revision_id,
            event_type,
            "seo_cms_event@1",
        ],
    );
    let event_payload = json!({
        "verdict": verdict,
        "blocking_reasons": blockers.clone(),
        "canonical_url_path": row.canonical_url_path.clone(),
        "canonical_slug": row.canonical_slug.clone(),
        "run_id": input.run_id.clone(),
    });
    sqlx::query(
        r#"
        INSERT INTO site.cms_publish_events
            (event_key, event_type, page_node_key, scope_signature, revision_id,
             actor_role, causal_step, payload_version, event_payload)
        VALUES ($1, $2, $3, $4, $5, $6, 'cms_publish', 'seo_cms_event@1', $7)
        ON CONFLICT (event_key) DO UPDATE
        SET event_payload = EXCLUDED.event_payload
        "#,
    )
    .bind(&event_key)
    .bind(event_type)
    .bind(&row.page_node_key)
    .bind(&row.scope_signature)
    .bind(&revision_id)
    .bind(if input.actor_role.trim().is_empty() {
        "seo_system"
    } else {
        input.actor_role.as_str()
    })
    .bind(Json(event_payload.clone()))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if !blockers.is_empty() {
        for blocker in &blockers {
            let task_key = primitives::seo::seo_artifact_key(
                "seo_hitl_task",
                &[&row.page_node_key, &revision_id, blocker, "seo_hitl@1"],
            );
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     page_node_key, scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                VALUES ($1, $2, 'open', 'seo_editor', 'seo_editor', $3, $4,
                        'cms_publish', $5, 'blocking', $6, $7)
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    decision_payload = EXCLUDED.decision_payload,
                    updated_at = now()
                "#,
            )
            .bind(&task_key)
            .bind(hitl_task_type(blocker))
            .bind(&row.page_node_key)
            .bind(&row.scope_signature)
            .bind(&revision_id)
            .bind(Json(
                json!({ "blocker": blocker, "revision_id": revision_id }),
            ))
            .bind(Json(json!([{
                "event": "publish_blocked",
                "blocker": blocker,
            }])))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
    }

    let cms_event = SeoCmsEventPayload {
        event_key,
        event_type: event_type.to_string(),
        page_node_key: row.page_node_key.clone(),
        scope_signature: row.scope_signature.clone(),
        revision_id: revision_id.clone(),
        actor_role: if input.actor_role.trim().is_empty() {
            "seo_system".to_string()
        } else {
            input.actor_role.clone()
        },
        causal_step: "cms_publish".to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        payload_version: "seo_cms_event@1".to_string(),
        metadata: HashMap::from([
            ("verdict".to_string(), verdict.to_string()),
            (
                "canonical_url_path".to_string(),
                row.canonical_url_path.clone(),
            ),
            ("cms_document_id".to_string(), cms_document_id.clone()),
        ]),
    };
    let emitted = outbox_emit_many(pool, &[cms_event_outbox(&cms_event)]).await?;
    let publish_artifact = if verdict == "approved" {
        let manifest = json!({
            "page_node_key": row.page_node_key.clone(),
            "revision_id": revision_id.clone(),
            "canonical_url_path": row.canonical_url_path.clone(),
            "cms_document_id": cms_document_id.clone(),
            "content_contract": "headless_cms_blocks@1",
        });
        let artifact = PublishArtifact {
            artifact_key: primitives::seo::seo_artifact_key(
                "publish_artifact",
                &[&row.page_node_key, &revision_id, "headless_snapshot"],
            ),
            page_node_key: row.page_node_key.clone(),
            revision_id: revision_id.clone(),
            artifact_type: "headless_snapshot".to_string(),
            artifact_uri: format!(
                "cms://pages/{}/revisions/{}",
                row.page_node_key, revision_id
            ),
            manifest_json: manifest.to_string(),
            status: "pending_materialization".to_string(),
        };
        sqlx::query(
            r#"
            INSERT INTO site.publish_artifacts
                (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                 manifest_json, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (artifact_key) DO UPDATE
            SET artifact_uri = EXCLUDED.artifact_uri,
                manifest_json = EXCLUDED.manifest_json,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(&artifact.artifact_key)
        .bind(&artifact.page_node_key)
        .bind(&artifact.revision_id)
        .bind(&artifact.artifact_type)
        .bind(&artifact.artifact_uri)
        .bind(Json(manifest))
        .bind(&artifact.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        Some(artifact)
    } else {
        None
    };

    Ok(CmsPublishOutputPayload {
        page_node_key: row.page_node_key,
        page_draft_key: row.page_draft_key,
        revision_id,
        cms_document_id,
        verdict: verdict.to_string(),
        blocking_reasons: blockers,
        outbox_emitted: emitted as u32,
        publish_artifact,
    })
}

pub async fn persist_publish_materialize_output(
    pool: &PgPool,
    input: &PublishMaterializeInputPayload,
    planned: &PublishMaterializeOutputPayload,
) -> Result<PublishMaterializeOutputPayload, DomainError> {
    let runtime = seo_publish_runtime_config(input);
    let mut artifact = planned.publish_artifact.clone().unwrap_or_default();
    if artifact.artifact_key.trim().is_empty() {
        artifact.artifact_key = primitives::seo::seo_artifact_key(
            "publish_artifact",
            &[
                &input.page_node_key,
                &input.revision_id,
                "headless_snapshot",
            ],
        );
    }
    let mut blocking_reasons = planned.blocking_reasons.clone();
    let (build_scope, build, incremental_error) =
        match static_site_builder_adapter::build_static_site_incremental(
            pool,
            Path::new(&runtime.output_dir),
            &runtime.base_url,
            &[input.page_node_key.clone()],
        )
        .await
        {
            Ok(result) => ("page".to_string(), result, None::<String>),
            Err(incremental_err) if runtime.allow_full_rebuild_fallback => {
                match static_site_builder_adapter::build_static_site(
                    pool,
                    Path::new(&runtime.output_dir),
                    &runtime.base_url,
                )
                .await
                {
                    Ok(result) => (
                        "site".to_string(),
                        result,
                        Some(incremental_err.to_string()),
                    ),
                    Err(full_err) => {
                        blocking_reasons.push("static_build_failed".to_string());
                        artifact.status = "build_failed".to_string();
                        artifact.artifact_uri = runtime.output_dir.clone();
                        artifact.manifest_json = json!({
                            "build_scope": "site",
                            "output_dir": runtime.output_dir,
                            "base_url": runtime.base_url,
                            "incremental_error": incremental_err.to_string(),
                            "full_rebuild_error": full_err.to_string(),
                        })
                        .to_string();
                        sqlx::query(
                            r#"
                            INSERT INTO site.publish_artifacts
                                (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                                 manifest_json, status)
                            VALUES ($1, $2, $3, $4, $5, $6, $7)
                            ON CONFLICT (artifact_key) DO UPDATE
                            SET artifact_uri = EXCLUDED.artifact_uri,
                                manifest_json = EXCLUDED.manifest_json,
                                status = EXCLUDED.status,
                                updated_at = now()
                            "#,
                        )
                        .bind(&artifact.artifact_key)
                        .bind(&input.page_node_key)
                        .bind(&input.revision_id)
                        .bind(if artifact.artifact_type.trim().is_empty() {
                            "headless_snapshot"
                        } else {
                            artifact.artifact_type.as_str()
                        })
                        .bind(&artifact.artifact_uri)
                        .bind(Json(json!({
                            "error": artifact.manifest_json.clone(),
                        })))
                        .bind(&artifact.status)
                        .execute(pool)
                        .await
                        .map_err(classify_sqlx)?;
                        return Ok(PublishMaterializeOutputPayload {
                            publish_artifact: Some(artifact),
                            preview_pages: Vec::new(),
                            materialization_status: "build_failed".to_string(),
                            blocking_reasons,
                        });
                    }
                }
            }
            Err(err) => {
                blocking_reasons.push("static_build_failed".to_string());
                artifact.status = "build_failed".to_string();
                artifact.artifact_uri = runtime.output_dir.clone();
                artifact.manifest_json = json!({
                    "build_scope": "page",
                    "output_dir": runtime.output_dir,
                    "base_url": runtime.base_url,
                    "incremental_error": err.to_string(),
                })
                .to_string();
                sqlx::query(
                    r#"
                    INSERT INTO site.publish_artifacts
                        (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                         manifest_json, status)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (artifact_key) DO UPDATE
                    SET artifact_uri = EXCLUDED.artifact_uri,
                        manifest_json = EXCLUDED.manifest_json,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(&artifact.artifact_key)
                .bind(&input.page_node_key)
                .bind(&input.revision_id)
                .bind(if artifact.artifact_type.trim().is_empty() {
                    "headless_snapshot"
                } else {
                    artifact.artifact_type.as_str()
                })
                .bind(&artifact.artifact_uri)
                .bind(Json(json!({
                    "error": artifact.manifest_json.clone(),
                })))
                .bind(&artifact.status)
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
                return Ok(PublishMaterializeOutputPayload {
                    publish_artifact: Some(artifact),
                    preview_pages: Vec::new(),
                    materialization_status: "build_failed".to_string(),
                    blocking_reasons,
                });
            }
        };

    let preview_pages = build
        .previews
        .into_iter()
        .map(|preview| RenderPreviewPageState {
            page_node_key: preview.page_node_key,
            revision_id: preview.revision_id,
            canonical_url_path: preview.canonical_url_path,
            rendered_html: preview.rendered_html,
            has_breadcrumbs: preview.has_breadcrumbs,
            has_schema_markup: preview.has_schema_markup,
            required_link_count: preview.required_link_count as u32,
            rendered_link_count: preview.rendered_link_count as u32,
        })
        .collect::<Vec<_>>();
    artifact.artifact_uri = build.output_dir.display().to_string();
    artifact.status = "built_pending_validation".to_string();
    artifact.manifest_json = json!({
        "build_scope": build_scope,
        "output_dir": build.output_dir,
        "base_url": runtime.base_url,
        "artifact_count": build.artifacts.len(),
        "page_count": preview_pages.len(),
        "content_contract": "headless_cms_blocks@1",
        "renderer_version": "alegria_static_site_builder@2",
        "incremental_error": incremental_error,
    })
    .to_string();
    sqlx::query(
        r#"
        INSERT INTO site.publish_artifacts
            (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
             manifest_json, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (artifact_key) DO UPDATE
        SET artifact_uri = EXCLUDED.artifact_uri,
            manifest_json = EXCLUDED.manifest_json,
            status = EXCLUDED.status,
            updated_at = now()
        "#,
    )
    .bind(&artifact.artifact_key)
    .bind(&input.page_node_key)
    .bind(&input.revision_id)
    .bind(if artifact.artifact_type.trim().is_empty() {
        "headless_snapshot"
    } else {
        artifact.artifact_type.as_str()
    })
    .bind(&artifact.artifact_uri)
    .bind(Json(json!({
        "build_scope": build_scope,
        "output_dir": artifact.artifact_uri.clone(),
        "page_count": preview_pages.len(),
        "blocking_reasons": blocking_reasons,
        "incremental_error": incremental_error,
    })))
    .bind(&artifact.status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(r#"DELETE FROM site.publish_artifact_entries WHERE artifact_key = $1"#)
        .bind(&artifact.artifact_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    for built_artifact in &build.artifacts {
        let entry_type = match built_artifact.relative_path.as_str() {
            "sitemap.xml" => "sitemap",
            "robots.txt" => "robots",
            "alegria-static-manifest.json" => "manifest",
            _ => "page",
        };
        sqlx::query(
            r#"
            INSERT INTO site.publish_artifact_entries
                (artifact_entry_key, artifact_key, relative_path, entry_type, status)
            VALUES ($1, $2, $3, $4, 'built')
            ON CONFLICT (artifact_key, relative_path) DO UPDATE
            SET entry_type = EXCLUDED.entry_type,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(primitives::seo::seo_artifact_key(
            "publish_artifact_entry",
            &[&artifact.artifact_key, &built_artifact.relative_path],
        ))
        .bind(&artifact.artifact_key)
        .bind(&built_artifact.relative_path)
        .bind(entry_type)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    Ok(PublishMaterializeOutputPayload {
        publish_artifact: Some(artifact),
        preview_pages,
        materialization_status: format!("built_pending_validation:{build_scope}"),
        blocking_reasons: planned.blocking_reasons.clone(),
    })
}

pub async fn persist_finalize_publish_output(
    pool: &PgPool,
    input: &FinalizePublishInputPayload,
    planned: &FinalizePublishOutputPayload,
) -> Result<FinalizePublishOutputPayload, DomainError> {
    let artifact = input.publish_artifact.as_ref().cloned().unwrap_or_default();
    let render_validation = input
        .render_validation
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let mut blocking_reasons = planned.blocking_reasons.clone();
    blocking_reasons.extend(render_validation.blocking_reasons.clone());
    blocking_reasons.sort();
    blocking_reasons.dedup();
    let is_published = planned.verdict == "published";

    sqlx::query(
        r#"
        UPDATE site.cms_page_revisions
        SET revision_status = $1,
            updated_at = now()
        WHERE revision_id = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.revision_id)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.cms_pages
        SET current_status = $1,
            published_at = CASE WHEN $1 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if !artifact.artifact_key.trim().is_empty() {
        sqlx::query(
            r#"
            UPDATE site.publish_artifacts
            SET status = $1,
                updated_at = now()
            WHERE artifact_key = $2
            "#,
        )
        .bind(if is_published { "ready" } else { "blocked" })
        .bind(&artifact.artifact_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    let event_type = if is_published {
        "seo_page_published"
    } else {
        "seo_page_publish_blocked"
    };
    let event_key = primitives::seo::seo_artifact_key(
        "cms_event",
        &[
            &input.page_node_key,
            &input.revision_id,
            event_type,
            "publish_finalize@1",
        ],
    );
    sqlx::query(
        r#"
        INSERT INTO site.cms_publish_events
            (event_key, event_type, page_node_key, scope_signature, revision_id,
             actor_role, causal_step, payload_version, event_payload)
        SELECT $1, $2, page_node_key, scope_signature, $3, 'seo_system',
               'publish_materialize', 'seo_cms_event@1', $4
        FROM site.cms_pages
        WHERE page_node_key = $5
        ON CONFLICT (event_key) DO UPDATE
        SET event_payload = EXCLUDED.event_payload
        "#,
    )
    .bind(&event_key)
    .bind(event_type)
    .bind(&input.revision_id)
    .bind(Json(json!({
        "verdict": planned.verdict,
        "blocking_reasons": blocking_reasons,
    })))
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let cms_event = SeoCmsEventPayload {
        event_key,
        event_type: event_type.to_string(),
        page_node_key: input.page_node_key.clone(),
        scope_signature: String::new(),
        revision_id: input.revision_id.clone(),
        actor_role: "seo_system".to_string(),
        causal_step: "publish_materialize".to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        payload_version: "seo_cms_event@1".to_string(),
        metadata: HashMap::from([("verdict".to_string(), planned.verdict.clone())]),
    };
    let _ = outbox_emit_many(pool, &[cms_event_outbox(&cms_event)]).await?;

    if !is_published {
        for blocker_reason in &blocking_reasons {
            let task_key = primitives::seo::seo_artifact_key(
                "seo_hitl_task",
                &[
                    &input.page_node_key,
                    &input.revision_id,
                    blocker_reason,
                    "publish_finalize@1",
                ],
            );
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     page_node_key, scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                SELECT $1, $2, 'open', 'seo_editor', 'seo_editor', page_node_key, scope_signature,
                       'publish_materialize', $3, 'blocking', $4, $5
                FROM site.cms_pages
                WHERE page_node_key = $6
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    updated_at = now()
                "#,
            )
            .bind(&task_key)
            .bind(hitl_task_type(blocker_reason))
            .bind(&input.revision_id)
            .bind(Json(json!({ "blocker": blocker_reason })))
            .bind(Json(
                json!([{ "event": "publish_blocked", "blocker": blocker_reason }]),
            ))
            .bind(&input.page_node_key)
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
    }

    Ok(FinalizePublishOutputPayload {
        verdict: planned.verdict.clone(),
        current_status: if is_published {
            "published".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons,
        publish_artifact: Some(PublishArtifact {
            status: if is_published {
                "ready".to_string()
            } else {
                "blocked".to_string()
            },
            ..artifact
        }),
    })
}
