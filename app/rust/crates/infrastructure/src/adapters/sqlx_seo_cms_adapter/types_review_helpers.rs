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

fn cms_event_outbox(event: &SeoCmsEventPayload, run_id: &str) -> OutboxEnvelope {
    let payload_bytes = encode_payload(event);
    OutboxEnvelope {
        run_id: run_id.to_string(),
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

