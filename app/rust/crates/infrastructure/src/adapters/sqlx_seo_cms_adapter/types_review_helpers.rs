use std::collections::HashMap;

use contracts::generated::alegria::sync::v1::SeoCmsEventPayload;
use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, PublishArtifact,
    PublishMaterializeInputPayload, PublishMaterializeOutputPayload, RenderPreviewPageState,
};
use primitives::errors::DomainError;
use prost::Message;
use seo_ports::{
    CmsReviewDecisionOutcome, CmsReviewDecisionRequest, PublicReleaseDelivery,
    PublicReleaseFile, ReleaseDeliveryError, ReleaseDeliveryStage,
};
use serde_json::json;
use sqlx::{types::Json, PgConnection, PgPool, Row};
use std::path::Path;

use super::proto_runtime_payload_store::{classify_sqlx, validation_failure};
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx;
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
    locale_code: String,
    canonical_url_path: String,
    canonical_slug: String,
    page_type_key: String,
    dominant_intent: String,
    traceability_manifest: serde_json::Value,
}

fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

fn make_idempotency_key(event_key: &str, event_type: &str) -> String {
    // Event identity is durable. Hashing protobuf bytes here is unsafe because
    // map<string,string> metadata has no guaranteed serialization order.
    primitives::hash::content_hash_v1(&format!("{event_key}|{event_type}"))
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
        idempotency_key: make_idempotency_key(&event.event_key, &event.event_type),
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

pub fn resolve_publish_materialize_input(
    input: &PublishMaterializeInputPayload,
) -> PublishMaterializeInputPayload {
    resolve_publish_settings(
        input,
        std::env::var("SEO_STATIC_OUTPUT_DIR").ok(),
        std::env::var("SEO_STATIC_BASE_URL").ok(),
    )
}

fn resolve_publish_settings(
    input: &PublishMaterializeInputPayload,
    configured_output_dir: Option<String>,
    configured_base_url: Option<String>,
) -> PublishMaterializeInputPayload {
    let mut resolved = input.clone();
    if resolved.output_dir.trim().is_empty() {
        resolved.output_dir = configured_output_dir.unwrap_or_default();
    }
    if resolved.base_url.trim().is_empty() {
        resolved.base_url = configured_base_url.unwrap_or_default();
    }
    resolved
}

#[cfg(test)]
mod publish_runtime_config_tests {
    use super::*;

    #[test]
    fn omitted_settings_resolve_and_explicit_settings_win() {
        let input = PublishMaterializeInputPayload {
            page_node_key: "page".to_string(),
            ..Default::default()
        };
        let resolved = resolve_publish_settings(
            &input,
            Some("/configured/site".to_string()),
            Some("https://configured.example".to_string()),
        );
        assert_eq!(resolved.output_dir, "/configured/site");
        assert_eq!(resolved.base_url, "https://configured.example");
        assert_eq!(resolved.page_node_key, input.page_node_key);
        let explicit = resolve_publish_settings(
            &resolved,
            Some("/other".to_string()),
            Some("https://other.example".to_string()),
        );
        assert_eq!(explicit, resolved);
    }

    #[test]
    fn missing_settings_do_not_fall_back_to_a_placeholder_site() {
        let resolved =
            resolve_publish_settings(&PublishMaterializeInputPayload::default(), None, None);
        assert!(resolved.output_dir.is_empty());
        assert!(resolved.base_url.is_empty());
    }
}
async fn load_publish_draft(
    conn: &mut PgConnection,
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
            COALESCE(d.traceability_manifest->>'seo_title', b.title) AS title,
            COALESCE(d.traceability_manifest->>'seo_meta_description', b.meta_description) AS meta_description,
            b.blueprint_key,
            p.scope_signature,
            COALESCE(NULLIF(scope.locale, ''), 'und') AS locale_code,
            p.canonical_url_path,
            p.canonical_slug,
            p.page_type_key,
            p.dominant_intent,
            d.traceability_manifest
        FROM site.page_drafts d
        JOIN site.page_briefs b ON b.page_brief_key = d.page_brief_key
        JOIN site.page_nodes p ON p.page_node_key = d.page_node_key
        LEFT JOIN site.site_scopes scope ON scope.scope_signature = p.scope_signature
        WHERE d.page_draft_key = $1
        FOR UPDATE OF d, p
        "#,
    )
    .bind(page_draft_key)
    .fetch_optional(&mut *conn)
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
            locale_code: r.get("locale_code"),
            canonical_url_path: r.get("canonical_url_path"),
            canonical_slug: r.get("canonical_slug"),
            page_type_key: r.get("page_type_key"),
            dominant_intent: r.get("dominant_intent"),
            traceability_manifest: traceability_manifest.0,
        }
    }))
}

async fn count_active_cannibalization(
    conn: &mut PgConnection,
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
    .fetch_one(&mut *conn)
    .await
    .map_err(classify_sqlx)
}

// Candidate graph edges describe future page planning, not immediately
// enforceable publication obligations. Only accepted/applied required edges
// may block publication when their CMS targets are not yet live.
// Required structural links must be planned. Candidate targets that are not
// yet published are intentionally deferred until their pages go live; they do
// not become hard obligations until accepted or applied.
async fn count_required_link_recommendations(
    conn: &mut PgConnection,
    page_node_key: &str,
) -> Result<i64, DomainError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(DISTINCT target_page_key)::bigint
        FROM site.link_recommendations
        WHERE source_page_key = $1
          AND required_flag = true
          AND status IN ('candidate','accepted','applied')
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&mut *conn)
    .await
    .map_err(classify_sqlx)
}

async fn count_unpublished_mandatory_links(
    conn: &mut PgConnection,
    page_node_key: &str,
) -> Result<i64, DomainError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(DISTINCT l.target_page_key)::bigint
        FROM site.link_recommendations l
        WHERE l.source_page_key = $1
          AND l.required_flag = true
          AND l.status IN ('accepted','applied')
          AND NOT EXISTS (
              SELECT 1
              FROM site.cms_pages p
              JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
              WHERE p.page_node_key = l.target_page_key
                AND p.current_status = 'published'
                AND r.revision_status = 'published'
          )
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&mut *conn)
    .await
    .map_err(classify_sqlx)
}

#[derive(Debug, Clone)]
struct ResolvedInternalLink {
    target_page_key: String,
    canonical_url_path: String,
    title: String,
    link_role: String,
}

async fn resolve_required_internal_links(
    conn: &mut PgConnection,
    page_node_key: &str,
) -> Result<Vec<ResolvedInternalLink>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (l.target_page_key)
               l.target_page_key, l.link_role, p.canonical_url_path, r.title
        FROM site.link_recommendations l
        JOIN site.cms_pages p ON p.page_node_key = l.target_page_key
                             AND p.current_status = 'published'
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
                                      AND r.revision_status = 'published'
        WHERE l.source_page_key = $1
          AND l.required_flag = true
          AND l.status IN ('candidate','accepted','applied')
        ORDER BY l.target_page_key, l.score DESC, l.link_recommendation_key
        "#,
    )
    .bind(page_node_key)
    .fetch_all(&mut *conn)
    .await
    .map_err(classify_sqlx)?;
    rows.into_iter()
        .map(|row| {
            let path: String = row.get("canonical_url_path");
            if !path.starts_with('/')
                || path.starts_with("//")
                || path.chars().any(|ch| {
                    ch.is_whitespace() || ch.is_control() || matches!(ch, '[' | ']' | '(' | ')' | '<' | '>')
                })
            {
                return Err(validation_failure("required internal target has unsafe canonical URL"));
            }
            let original_title: String = row.get("title");
            let label = original_title
                .chars()
                .filter(|ch| !ch.is_control() && !matches!(ch, '[' | ']' | '(' | ')' | '<' | '>' | '*' | '`'))
                .take(110)
                .collect::<String>();
            Ok(ResolvedInternalLink {
                target_page_key: row.get("target_page_key"),
                canonical_url_path: path,
                title: if label.trim().is_empty() { "Связанная страница".into() } else { label },
                link_role: row.get("link_role"),
            })
        })
        .collect()
}

async fn count_canonical_conflicts(
    conn: &mut PgConnection,
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
    .fetch_one(&mut *conn)
    .await
    .map_err(classify_sqlx)
}

pub async fn load_latest_approval_decision(
    pool: &PgPool,
    page_node_key: &str,
    revision_id: &str,
) -> Result<Option<seo_domain::review_decision::CmsApproval>, DomainError> {
    let mut conn = pool.acquire().await.map_err(classify_sqlx)?;
    load_latest_approval_decision_in_tx(&mut conn, page_node_key, revision_id).await
}

async fn load_latest_approval_decision_in_tx(
    conn: &mut PgConnection,
    page_node_key: &str,
    revision_id: &str,
) -> Result<Option<seo_domain::review_decision::CmsApproval>, DomainError> {
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
    .fetch_optional(&mut *conn)
    .await
    .map_err(classify_sqlx)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let raw_decision: String = row.get("decision");
    let decision = seo_domain::review_decision::HumanReviewDecision::parse(&raw_decision)
        .map_err(|_| DomainError::ContractViolation {
            message: format!("unsupported persisted CMS approval decision: {raw_decision}"),
        })?;
    Ok(Some(seo_domain::review_decision::CmsApproval {
        decision_key: row.get("decision_key"),
        page_node_key: row.get("page_node_key"),
        revision_id: row.get("revision_id"),
        actor_role: row.get("actor_role"),
        decision,
        reason: row.get("reason"),
        decided_at: row.get("decided_at"),
    }))
}

async fn resolve_seo_workflow_id(
    conn: &mut PgConnection,
    page_node_key: &str,
    revision_id: &str,
) -> Result<Option<String>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT e.event_payload ->> 'run_id' AS run_id
        FROM site.cms_publish_events e
        JOIN pipeline.execution_runs x
          ON x.run_id::text = e.event_payload ->> 'run_id'
         AND x.status NOT IN ('done', 'failed')
        WHERE e.page_node_key = $1
          AND e.revision_id = $2
          AND e.event_type = 'seo_page_review_requested'
        ORDER BY run_id
        LIMIT 2
        "#,
    )
    .bind(page_node_key)
    .bind(revision_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(classify_sqlx)?;
    if rows.len() > 1 {
        return Err(validation_failure(
            "review revision is owned by multiple active workflows",
        ));
    }
    Ok(rows
        .first()
        .and_then(|row| row.get::<Option<String>, _>("run_id")))
}
