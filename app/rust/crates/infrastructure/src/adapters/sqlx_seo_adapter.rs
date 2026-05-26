use std::collections::{BTreeMap, HashMap};

use contracts::generated::alegria::sync::v1::{QdrantUpsertCommand, SeoGraphProjectionPayload};
use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload,
    SectionTemplateBinding, SeoScopePayload, SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use primitives::qdrant_point_id::qdrant_point_id_v1;
use prost::Message;
use runtime_models::{
    ContentGap, GraphPlanningContext, GraphPlanningCoverageSignal, GraphPlanningTopicSignal,
    GraphPlanningTripleSignal, KeywordCluster, LinkRecommendation, PageNode,
};
use seo_domain::{applicability, identity, rebuild};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::{types::Json, PgPool, Row};
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, validation_failure,
    RuntimeProtoPayload,
};
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;

#[derive(Debug, Clone)]
struct ScopeFields {
    scope_signature: String,
    market: String,
    locale: String,
    country_code: Option<String>,
    visa_type: Option<String>,
    applicant_profile: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialExtractionSweepOutputBlob {
    sections: Vec<EditorialExtractionSectionBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialExtractionSectionBlob {
    topics: Vec<EditorialTopicBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialTopicBlob {
    topic_type: String,
    topic_key_candidate: String,
    #[allow(dead_code)]
    confidence: f32,
}

#[derive(Debug, serde::Deserialize)]
struct TripleBuilderSweepOutputBlob {
    sections: Vec<TripleBuilderSectionBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct TripleBuilderSectionBlob {
    triples: Vec<TripleSignalBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct TripleSignalBlob {
    triple_id: String,
    subject_key: String,
    relation_type: String,
    object_key: String,
    evidence_section_id: String,
    confidence: f32,
}

#[derive(Debug, Clone)]
pub struct SeoScopeBootstrapReport {
    pub context_key: String,
    pub created_context: bool,
    pub seeded_registry_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalNavigationReport {
    pub navigation_tree_key: String,
    pub scope_count: u64,
    pub page_item_count: u64,
    pub silo_group_count: u64,
    pub rebuild_plan_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionSyncStatus {
    pub target_system: String,
    pub pending_events: i64,
    pub processing_events: i64,
    pub failed_events: i64,
    pub done_events: i64,
    pub max_open_lag_ms: i64,
    pub oldest_open_event_id: Option<String>,
    pub oldest_open_aggregate_key: Option<String>,
    pub oldest_open_event_type: Option<String>,
    pub latest_failed_aggregate_key: Option<String>,
    pub latest_failed_event_type: Option<String>,
    pub latest_failed_error: Option<String>,
}

impl ProjectionSyncStatus {
    pub fn open_event_count(&self) -> i64 {
        self.pending_events + self.processing_events
    }

    pub fn blocking_event_count(&self) -> i64 {
        self.open_event_count() + self.failed_events
    }
}

#[derive(Debug, Clone)]
struct ActivePageForNavigation {
    page_node_key: String,
    scope_signature: String,
    parent_page_node_key: String,
    page_type_key: String,
    canonical_slug: String,
    canonical_url_path: String,
    hierarchy_depth: i32,
    menu_group: String,
    market: String,
    locale: String,
    country_code: Option<String>,
    visa_type: Option<String>,
    applicant_profile: Option<String>,
}

fn blank_as_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn owned_blank_as_none(value: &str) -> Option<String> {
    blank_as_none(value).map(ToOwned::to_owned)
}

fn scope_fields(scope: Option<&SeoScopePayload>, fallback_scope_signature: &str) -> ScopeFields {
    let scope_signature = scope
        .and_then(|s| blank_as_none(&s.scope_signature))
        .unwrap_or(fallback_scope_signature)
        .to_string();
    let market = scope
        .and_then(|s| blank_as_none(&s.market))
        .unwrap_or("global")
        .to_string();
    let locale = scope
        .and_then(|s| blank_as_none(&s.locale))
        .unwrap_or("und")
        .to_string();

    ScopeFields {
        scope_signature,
        market,
        locale,
        country_code: scope.and_then(|s| owned_blank_as_none(&s.country_code)),
        visa_type: scope.and_then(|s| owned_blank_as_none(&s.visa_type)),
        applicant_profile: scope.and_then(|s| owned_blank_as_none(&s.applicant_profile)),
    }
}

fn non_empty(value: &str, label: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(validation_failure(format!(
            "SEO persistence requires {label}"
        )));
    }
    Ok(())
}

async fn upsert_runtime_blob<T: RuntimeProtoPayload>(
    pool: &PgPool,
    run_id: &str,
    field_name: &str,
    value: &T,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (payload_bytes, payload_hash) = encode_runtime_payload(value)?;
    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type = EXCLUDED.payload_type,
            schema_version = EXCLUDED.schema_version,
            payload_bytes = EXCLUDED.payload_bytes,
            payload_hash = EXCLUDED.payload_hash,
            updated_at = now()
        "#,
    )
    .bind(uuid)
    .bind(field_name)
    .bind(T::payload_type())
    .bind(T::schema_version())
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

async fn upsert_runtime_json_blob(
    pool: &PgPool,
    run_id: &str,
    field_name: &str,
    payload_type: &str,
    value: &Value,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let payload_bytes =
        serde_json::to_vec(value).map_err(|e| contract_violation(format!("json encode: {e}")))?;
    let payload_hash = primitives::hash::blake3_hex(&payload_bytes);
    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, $2, $3, 1, $4, $5)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type = EXCLUDED.payload_type,
            payload_bytes = EXCLUDED.payload_bytes,
            payload_hash = EXCLUDED.payload_hash,
            updated_at = now()
        "#,
    )
    .bind(uuid)
    .bind(field_name)
    .bind(payload_type)
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

fn make_idempotency_key(aggregate_key: &str, event_type: &str, payload_bytes: &[u8]) -> String {
    let payload_hash = primitives::hash::blake3_hex(payload_bytes);
    primitives::hash::content_hash_v1(&format!("{aggregate_key}|{event_type}|{payload_hash}"))
}

fn seo_graph_projection_event(
    artifact_type: &str,
    artifact_key: &str,
    scope_signature: &str,
) -> OutboxEnvelope {
    let payload_bytes = encode_payload(&SeoGraphProjectionPayload {
        artifact_type: artifact_type.to_string(),
        artifact_key: artifact_key.to_string(),
        scope_signature: scope_signature.to_string(),
    });
    OutboxEnvelope {
        run_id: String::new(),
        aggregate_type: artifact_type.to_string(),
        aggregate_key: artifact_key.to_string(),
        target_system: "neo4j".to_string(),
        event_type: "SeoGraphProjectionUpserted".to_string(),
        payload_type: "alegria.outbox.seo_graph_projection.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(
            artifact_key,
            "SeoGraphProjectionUpserted",
            &payload_bytes,
        ),
        payload_bytes,
    }
}

fn fingerprint_vector(text: &str) -> Vec<f32> {
    let digest = primitives::hash::blake3_hex(text.as_bytes());
    let mut vector = Vec::with_capacity(16);
    for chunk in digest.as_bytes().chunks(2).take(16) {
        let Ok(hex) = std::str::from_utf8(chunk) else {
            continue;
        };
        let value = u8::from_str_radix(hex, 16).unwrap_or(0);
        vector.push((value as f32 / 127.5) - 1.0);
    }
    if vector.is_empty() {
        vector.push(0.0);
    }
    vector
}

fn seo_qdrant_projection_event(
    collection_name: &str,
    artifact_type: &str,
    artifact_key: &str,
    scope_signature: &str,
    embedding_text: &str,
    mut metadata: HashMap<String, String>,
) -> OutboxEnvelope {
    metadata.insert("artifact_type".to_string(), artifact_type.to_string());
    metadata.insert("artifact_key".to_string(), artifact_key.to_string());
    metadata.insert("scope_signature".to_string(), scope_signature.to_string());
    metadata.insert(
        "embedding_model".to_string(),
        "deterministic-fingerprint".to_string(),
    );
    metadata.insert(
        "embedding_version".to_string(),
        "seo_projection_bootstrap@1".to_string(),
    );

    let vector = fingerprint_vector(embedding_text);
    let payload_bytes = encode_payload(&QdrantUpsertCommand {
        event_id: String::new(),
        collection_name: collection_name.to_string(),
        entity_type: artifact_type.to_string(),
        entity_key: artifact_key.to_string(),
        point_id: qdrant_point_id_v1(collection_name, artifact_type, artifact_key),
        vector,
        payload: None,
        distance: "cosine".to_string(),
        vector_size: 16,
        metadata,
    });
    OutboxEnvelope {
        run_id: String::new(),
        aggregate_type: artifact_type.to_string(),
        aggregate_key: artifact_key.to_string(),
        target_system: "qdrant".to_string(),
        event_type: "QdrantUpsertCommand".to_string(),
        payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(artifact_key, "QdrantUpsertCommand", &payload_bytes),
        payload_bytes,
    }
}

async fn emit_projection_events_for_run(
    pool: &PgPool,
    run_id: &str,
    events: Vec<OutboxEnvelope>,
) -> Result<(), DomainError> {
    let events = events
        .into_iter()
        .map(|mut event| {
            if event.run_id.trim().is_empty() {
                event.run_id = run_id.to_string();
            }
            event
        })
        .collect::<Vec<_>>();
    let _ = outbox_emit_many(pool, &events).await?;
    Ok(())
}

pub async fn read_projection_sync_status_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<ProjectionSyncStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH target_systems(target_system) AS (
            VALUES ('neo4j'), ('qdrant'), ('cms')
        )
        SELECT
            targets.target_system,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'pending')::BIGINT AS pending_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'processing')::BIGINT AS processing_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'failed')::BIGINT AS failed_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'done')::BIGINT AS done_events,
            COALESCE(
                MAX(
                    CASE
                        WHEN outbox.status IN ('pending', 'processing') THEN
                            GREATEST(
                                0,
                                FLOOR(EXTRACT(EPOCH FROM (now() - outbox.created_at)) * 1000)
                            )::BIGINT
                        ELSE 0
                    END
                ),
                0
            )::BIGINT AS max_open_lag_ms,
            oldest.event_id AS oldest_open_event_id,
            oldest.aggregate_key AS oldest_open_aggregate_key,
            oldest.event_type AS oldest_open_event_type,
            latest_failed.aggregate_key AS latest_failed_aggregate_key,
            latest_failed.event_type AS latest_failed_event_type,
            latest_failed.last_error AS latest_failed_error
        FROM target_systems targets
        LEFT JOIN system.sync_outbox outbox
            ON outbox.target_system = targets.target_system
           AND outbox.run_id = $1
        LEFT JOIN LATERAL (
            SELECT
                event_id::TEXT AS event_id,
                aggregate_key,
                event_type
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND run_id = $1
              AND status IN ('pending', 'processing')
            ORDER BY created_at ASC
            LIMIT 1
        ) oldest ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                aggregate_key,
                event_type,
                last_error
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND run_id = $1
              AND status = 'failed'
            ORDER BY updated_at DESC
            LIMIT 1
        ) latest_failed ON TRUE
        GROUP BY
            targets.target_system,
            oldest.event_id,
            oldest.aggregate_key,
            oldest.event_type,
            latest_failed.aggregate_key,
            latest_failed.event_type,
            latest_failed.last_error
        ORDER BY targets.target_system
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| ProjectionSyncStatus {
            target_system: row.get("target_system"),
            pending_events: row.get("pending_events"),
            processing_events: row.get("processing_events"),
            failed_events: row.get("failed_events"),
            done_events: row.get("done_events"),
            max_open_lag_ms: row.get("max_open_lag_ms"),
            oldest_open_event_id: row.get("oldest_open_event_id"),
            oldest_open_aggregate_key: row.get("oldest_open_aggregate_key"),
            oldest_open_event_type: row.get("oldest_open_event_type"),
            latest_failed_aggregate_key: row.get("latest_failed_aggregate_key"),
            latest_failed_event_type: row.get("latest_failed_event_type"),
            latest_failed_error: row.get("latest_failed_error"),
        })
        .collect())
}

pub async fn read_projection_sync_status(
    pool: &PgPool,
) -> Result<Vec<ProjectionSyncStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH target_systems(target_system) AS (
            VALUES ('neo4j'), ('qdrant'), ('cms')
        )
        SELECT
            targets.target_system,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'pending')::BIGINT AS pending_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'processing')::BIGINT AS processing_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'failed')::BIGINT AS failed_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'done')::BIGINT AS done_events,
            COALESCE(
                MAX(
                    CASE
                        WHEN outbox.status IN ('pending', 'processing') THEN
                            GREATEST(
                                0,
                                FLOOR(EXTRACT(EPOCH FROM (now() - outbox.created_at)) * 1000)
                            )::BIGINT
                        ELSE 0
                    END
                ),
                0
            )::BIGINT AS max_open_lag_ms,
            oldest.event_id AS oldest_open_event_id,
            oldest.aggregate_key AS oldest_open_aggregate_key,
            oldest.event_type AS oldest_open_event_type,
            latest_failed.aggregate_key AS latest_failed_aggregate_key,
            latest_failed.event_type AS latest_failed_event_type,
            latest_failed.last_error AS latest_failed_error
        FROM target_systems targets
        LEFT JOIN system.sync_outbox outbox
            ON outbox.target_system = targets.target_system
        LEFT JOIN LATERAL (
            SELECT
                event_id::TEXT AS event_id,
                aggregate_key,
                event_type
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND status IN ('pending', 'processing')
            ORDER BY created_at ASC
            LIMIT 1
        ) oldest ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                aggregate_key,
                event_type,
                last_error
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND status = 'failed'
            ORDER BY updated_at DESC
            LIMIT 1
        ) latest_failed ON TRUE
        GROUP BY
            targets.target_system,
            oldest.event_id,
            oldest.aggregate_key,
            oldest.event_type,
            latest_failed.aggregate_key,
            latest_failed.event_type,
            latest_failed.last_error
        ORDER BY targets.target_system
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| ProjectionSyncStatus {
            target_system: row.get("target_system"),
            pending_events: row.get("pending_events"),
            processing_events: row.get("processing_events"),
            failed_events: row.get("failed_events"),
            done_events: row.get("done_events"),
            max_open_lag_ms: row.get("max_open_lag_ms"),
            oldest_open_event_id: row.get("oldest_open_event_id"),
            oldest_open_aggregate_key: row.get("oldest_open_aggregate_key"),
            oldest_open_event_type: row.get("oldest_open_event_type"),
            latest_failed_aggregate_key: row.get("latest_failed_aggregate_key"),
            latest_failed_event_type: row.get("latest_failed_event_type"),
            latest_failed_error: row.get("latest_failed_error"),
        })
        .collect())
}

fn json_text<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

fn verified_support_fragment(
    role_type: &str,
    concept_label: &str,
    params: &serde_json::Value,
) -> String {
    match role_type {
        "document_required" | "must_provide" | "form_required" => {
            let mut fragment = format!("Required document: {concept_label}.");
            if json_text(params, "subtype").is_some_and(|v| !v.trim().is_empty()) {
                fragment = format!(
                    "Required document: {concept_label} ({}) .",
                    json_text(params, "subtype").unwrap_or_default()
                );
            }
            fragment.replace(" )", ")")
        }
        "fee_item" | "must_pay" => {
            let amount = params
                .get("amount")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let currency = json_text(params, "currency").unwrap_or("EUR");
            if amount > 0.0 {
                format!("Fee item: {amount:.2} {currency} for {concept_label}.")
            } else {
                format!("Fee item: {concept_label}.")
            }
        }
        "timeline_item" | "timeline" => {
            let days = params
                .get("days")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();
            if days > 0 {
                format!("Processing timeline: {concept_label} takes {days} days.")
            } else {
                format!("Processing timeline: {concept_label}.")
            }
        }
        "where_to_apply" => {
            let channel = json_text(params, "channel").unwrap_or("official route");
            let location = json_text(params, "location_key").unwrap_or(concept_label);
            format!("Where to apply: {location} via {channel}.")
        }
        "appointment_rule" => format!("Appointment rule: {concept_label}."),
        "eligibility_rule" | "must_satisfy" => format!("Eligibility rule: {concept_label}."),
        "step" => {
            let idx = params
                .get("step_index")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();
            if idx > 0 {
                format!("Application step {idx}: {concept_label}.")
            } else {
                format!("Application step: {concept_label}.")
            }
        }
        "allows" => format!("Allowed condition: {concept_label}."),
        "forbids" => format!("Forbidden condition: {concept_label}."),
        _ => format!("Verified rule: {concept_label}."),
    }
}

async fn resolve_verified_fact_support(
    pool: &PgPool,
    context_key: &str,
    applicant_profile: &str,
) -> Result<applicability::ResolvedVerifiedSupportBundle<SeoVerifiedFactSupportState>, DomainError>
{
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id,
            r.role_type,
            r.params,
            r.effective_from::text AS effective_from,
            r.effective_to::text AS effective_to,
            COALESCE(r.freshness_class, 'unknown') AS freshness_class,
            COALESCE(c.label_ru, c.concept_key, r.concept_key) AS concept_label,
            COALESCE(s.source_label, '') AS source_label,
            COALESCE(s.source_type, 'editorial') AS source_type,
            COALESCE(s.trust_level, 3) AS trust_level,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_any
                WHERE rp_any.rule_instance_id = r.rule_instance_id
            ) AS has_profile_overrides,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_apply
                WHERE rp_apply.rule_instance_id = r.rule_instance_id
                  AND rp_apply.profile_key = $2
                  AND rp_apply.applicability = 'applies'
            ) AS has_apply_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_conditional
                WHERE rp_conditional.rule_instance_id = r.rule_instance_id
                  AND rp_conditional.profile_key = $2
                  AND rp_conditional.applicability = 'conditional'
            ) AS has_conditional_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_exclude
                WHERE rp_exclude.rule_instance_id = r.rule_instance_id
                  AND rp_exclude.profile_key = $2
                  AND rp_exclude.applicability = 'excludes'
            ) AS has_exclude_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'waive'
            ) AS has_waive_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'remove_requirement'
            ) AS has_remove_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'replace_value'
            ) AS has_replace_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'add_requirement'
            ) AS has_add_requirement_exception
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1
          AND r.status = 'verified'
          AND COALESCE(r.publish_admissibility, 'not_admissible') = 'admissible'
          AND r.source_key IS NOT NULL
          AND r.source_key <> ''
          AND r.evidence_section_id IS NOT NULL
          AND COALESCE(r.evidence_quote, '') <> ''
          AND COALESCE(r.source_snapshot_hash, '') <> ''
        ORDER BY r.role_type, r.rule_instance_id
        "#,
    )
    .bind(context_key)
    .bind(applicant_profile)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut candidates = Vec::new();
    for row in rows {
        let rule_instance_id: String = row.get("rule_instance_id");
        let has_profile_overrides: bool = row.get("has_profile_overrides");
        let has_apply_profile: bool = row.get("has_apply_profile");
        let has_conditional_profile: bool = row.get("has_conditional_profile");
        let has_exclude_profile: bool = row.get("has_exclude_profile");
        let has_waive_exception: bool = row.get("has_waive_exception");
        let has_remove_exception: bool = row.get("has_remove_exception");
        let has_replace_exception: bool = row.get("has_replace_exception");
        let has_add_requirement_exception: bool = row.get("has_add_requirement_exception");
        let params: Json<serde_json::Value> = row.get("params");
        let role_type: String = row.get("role_type");
        let concept_label: String = row.get("concept_label");
        let source_type: String = row.get("source_type");
        let trust_level: i32 = row.get("trust_level");
        let source_tier = match (source_type.as_str(), trust_level) {
            ("government", _) | (_, 5) => "official",
            ("vfs", _) | (_, 4) => "regulated_partner",
            ("internal", _) => "internal_verified",
            ("niche_agency", _) => "industry_reference",
            _ => "editorial_reference",
        };
        let effective_to = row
            .get::<Option<String>, _>("effective_to")
            .unwrap_or_default();
        let freshness_class: String = row.get("freshness_class");
        let support = SeoVerifiedFactSupportState {
            fragment_text: verified_support_fragment(&role_type, &concept_label, &params.0),
            support_ref: rule_instance_id.clone(),
            role_type,
            source_label: row.get("source_label"),
            source_tier: source_tier.to_string(),
            freshness_class,
            observed_at: row
                .get::<Option<String>, _>("effective_from")
                .unwrap_or_default(),
            valid_until: effective_to,
        };
        candidates.push((
            applicability::ApplicabilityRuleCandidate {
                rule_instance_id,
                has_profile_overrides,
                has_apply_profile,
                has_conditional_profile,
                has_exclude_profile,
                has_waive_exception,
                has_remove_exception,
                has_replace_exception,
                has_add_requirement_exception,
            },
            support,
        ));
    }
    Ok(applicability::resolve_support_candidates(
        applicant_profile,
        candidates,
    ))
}

pub async fn load_verified_support_bundle(
    pool: &PgPool,
    run_id: &str,
    context_key: &str,
    scope_signature: &str,
    applicant_profile: &str,
) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
    non_empty(context_key, "context_key")?;
    non_empty(scope_signature, "scope_signature")?;
    let normalized_profile = validate_applicant_profile_reference(pool, applicant_profile).await?;
    let resolution = resolve_verified_fact_support(pool, context_key, &normalized_profile).await?;
    if resolution.included.is_empty() {
        return Err(validation_failure(format!(
            "verified support bundle is empty for context_key `{context_key}` and applicant_profile `{normalized_profile}`"
        )));
    }
    let support_count = resolution.included.len();
    let supports = resolution.included;
    let excluded_rule_count = resolution.excluded_rules.len();
    let excluded_rules = resolution
        .excluded_rules
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    let unresolved_rule_count = resolution.unresolved_rules.len();
    let unresolved_rules = resolution
        .unresolved_rules
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    let applied_override_count = resolution.applied_overrides.len();
    let applied_overrides = resolution
        .applied_overrides
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    upsert_runtime_json_blob(
        pool,
        run_id,
        "verified_support_bundle",
        "alegria.temporal.v1.SeoVerifiedSupportBundleJson",
        &json!({
            "context_key": context_key,
            "scope_signature": scope_signature,
            "applicant_profile": normalized_profile,
            "support_count": support_count,
            "supports": supports.clone(),
            "excluded_rule_count": excluded_rule_count,
            "excluded_rules": excluded_rules,
            "unresolved_rule_count": unresolved_rule_count,
            "unresolved_rules": unresolved_rules,
            "applied_override_count": applied_override_count,
            "applied_overrides": applied_overrides,
        }),
    )
    .await?;
    Ok(supports)
}

pub async fn resolve_rebuild_impacts(
    pool: &PgPool,
    changed_truth_keys: &[String],
) -> Result<BTreeMap<String, Value>, DomainError> {
    if changed_truth_keys.is_empty() {
        return Ok(BTreeMap::new());
    }

    let truth_support_refs = changed_truth_keys
        .iter()
        .map(|key| {
            key.strip_prefix("verified.rule_instance:")
                .or_else(|| key.strip_prefix("truth_support:"))
                .unwrap_or(key.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    let blueprint_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("site.page_blueprint:")
                .or_else(|| key.strip_prefix("blueprint:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let template_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("site.section_template:")
                .or_else(|| key.strip_prefix("template:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let serp_query_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("serp.query:")
                .or_else(|| key.strip_prefix("serp_pattern:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let source_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("source_key:")
                .or_else(|| key.strip_prefix("source:"))
                .or_else(|| key.strip_prefix("raw.source:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let navigation_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("navigation_state:")
                .or_else(|| key.strip_prefix("nav_state:"))
                .or_else(|| key.strip_prefix("site.navigation:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let keyword_cluster_refs = if serp_query_refs.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            r#"
            SELECT cluster_key
            FROM site.keyword_clusters
            WHERE seed_keyword = ANY($1)
            "#,
        )
        .bind(&serp_query_refs)
        .fetch_all(pool)
        .await
        .map_err(classify_sqlx)?
        .into_iter()
        .map(|row| row.get::<String, _>("cluster_key"))
        .collect::<Vec<_>>()
    };

    let rows = sqlx::query(
        r#"
        SELECT page_node_key, dependency_type, dependency_ref, reason_package
        FROM monitoring.seo_rebuild_dependencies
        WHERE status = 'active'
          AND (
            (dependency_type = 'truth_support' AND dependency_ref = ANY($1))
            OR (dependency_type = 'blueprint' AND dependency_ref = ANY($2))
            OR (dependency_type = 'section_template' AND dependency_ref = ANY($3))
            OR (dependency_type = 'serp_query' AND dependency_ref = ANY($4))
            OR (dependency_type = 'keyword_cluster' AND dependency_ref = ANY($5))
            OR (dependency_type = 'source_provenance' AND dependency_ref = ANY($6))
            OR (dependency_type = 'navigation_state' AND dependency_ref = ANY($7))
          )
        ORDER BY page_node_key, dependency_type, dependency_ref
        "#,
    )
    .bind(&truth_support_refs)
    .bind(&blueprint_refs)
    .bind(&template_refs)
    .bind(&serp_query_refs)
    .bind(&keyword_cluster_refs)
    .bind(&source_refs)
    .bind(&navigation_refs)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut impacted = BTreeMap::<String, Value>::new();
    for row in rows {
        let page_node_key: String = row.get("page_node_key");
        let dependency_type: String = row.get("dependency_type");
        let dependency_ref: String = row.get("dependency_ref");
        let reason_package: Json<Value> = row.get("reason_package");
        impacted
            .entry(page_node_key)
            .and_modify(|reason| {
                if let Some(reasons) = reason
                    .get_mut("matched_dependencies")
                    .and_then(Value::as_array_mut)
                {
                    reasons.push(json!({
                        "dependency_type": dependency_type,
                        "dependency_ref": dependency_ref,
                    }));
                }
            })
            .or_insert(json!({
                "matched_dependencies": [{
                    "dependency_type": dependency_type,
                    "dependency_ref": dependency_ref,
                }],
                "seed_reason_package": reason_package.0,
            }));
    }
    Ok(impacted)
}

pub fn normalize_applicant_profile(value: &str) -> Result<String, DomainError> {
    identity::normalize_applicant_profile(value)
}

pub async fn validate_applicant_profile_reference(
    pool: &PgPool,
    value: &str,
) -> Result<String, DomainError> {
    let normalized = normalize_applicant_profile(value)?;
    let row = sqlx::query(
        r#"
        SELECT status
        FROM kb.applicant_profiles
        WHERE profile_key = $1
        LIMIT 1
        "#,
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| {
        validation_failure(format!(
            "applicant_profile `{normalized}` is not registered in kb.applicant_profiles"
        ))
    })?;
    let status: String = row.get("status");
    if status != "active" {
        return Err(validation_failure(format!(
            "applicant_profile `{normalized}` is not active"
        )));
    }
    Ok(normalized)
}

pub async fn bootstrap_seo_scope(
    pool: &PgPool,
    requested_context_key: Option<&str>,
    country_code: &str,
    visa_family: &str,
    visa_subtype: Option<&str>,
    citizenship_code: &str,
) -> Result<SeoScopeBootstrapReport, DomainError> {
    let truth =
        identity::derive_truth_identity(country_code, visa_family, visa_subtype, citizenship_code)?;

    sqlx::query(
        r#"
        INSERT INTO kb.visa_families (key, label_ru)
        VALUES ($1, $2)
        ON CONFLICT (key) DO NOTHING
        "#,
    )
    .bind(&truth.visa_family)
    .bind(truth.visa_family.replace('_', " "))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    let context_key = requested_context_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&truth.context_key)
        .to_string();

    if let Some(row) = sqlx::query(
        r#"
        SELECT context_key
        FROM kb.visa_contexts
        WHERE country_code = $1
          AND visa_family = $2
          AND COALESCE(visa_subtype, '') = COALESCE($3, '')
          AND citizenship_code = $4
        "#,
    )
    .bind(&truth.country_code)
    .bind(&truth.visa_family)
    .bind(if truth.visa_subtype.is_empty() {
        None
    } else {
        Some(truth.visa_subtype.as_str())
    })
    .bind(&truth.citizenship_code)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    {
        let existing_context_key: String = row.get("context_key");
        if requested_context_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|requested| requested != existing_context_key)
        {
            return Err(validation_failure(format!(
                "seo scope already exists as context_key `{existing_context_key}`"
            )));
        }
        let seeded_registry_count = seed_runtime_registries(pool).await?;
        return Ok(SeoScopeBootstrapReport {
            context_key: existing_context_key,
            created_context: false,
            seeded_registry_count,
        });
    }

    let result = sqlx::query(
        r#"
        INSERT INTO kb.visa_contexts
            (context_key, country_code, visa_family, visa_subtype, citizenship_code, status)
        VALUES ($1, $2, $3, $4, $5, 'active')
        ON CONFLICT (context_key) DO UPDATE
        SET country_code = EXCLUDED.country_code,
            visa_family = EXCLUDED.visa_family,
            visa_subtype = EXCLUDED.visa_subtype,
            citizenship_code = EXCLUDED.citizenship_code,
            status = 'active',
            updated_at = now()
        "#,
    )
    .bind(&context_key)
    .bind(&truth.country_code)
    .bind(&truth.visa_family)
    .bind(if truth.visa_subtype.is_empty() {
        None
    } else {
        Some(truth.visa_subtype.as_str())
    })
    .bind(&truth.citizenship_code)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    let seeded_registry_count = seed_runtime_registries(pool).await?;
    Ok(SeoScopeBootstrapReport {
        context_key,
        created_context: result.rows_affected() > 0,
        seeded_registry_count,
    })
}

async fn seed_runtime_registries(pool: &PgPool) -> Result<u64, DomainError> {
    let page_types = [
        ("country_hub_page", "Country hub"),
        ("hub_page", "Topic hub"),
        ("detail_page", "Detail guide"),
        ("requirement_page", "Requirements guide"),
        ("fee_page", "Fee guide"),
        ("timeline_page", "Timeline guide"),
        ("faq_page", "FAQ guide"),
        ("checklist_page", "Checklist guide"),
        ("troubleshooting_page", "Troubleshooting guide"),
        ("comparison_page", "Comparison guide"),
        ("supporting_editorial", "Supporting editorial guide"),
    ];
    let mut inserted = 0u64;
    for (page_type, label) in page_types {
        let result = sqlx::query(
            r#"
            INSERT INTO site.registry_page_types (page_type_key, label)
            VALUES ($1, $2)
            ON CONFLICT (page_type_key) DO NOTHING
            "#,
        )
        .bind(page_type)
        .bind(label)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }

    let intents = [
        ("informational", "Informational"),
        ("commercial", "Commercial"),
        ("comparison", "Comparison"),
        ("troubleshooting", "Troubleshooting"),
    ];
    for (intent, label) in intents {
        let result = sqlx::query(
            r#"
            INSERT INTO site.registry_intent_types (intent_type_key, label)
            VALUES ($1, $2)
            ON CONFLICT (intent_type_key) DO NOTHING
            "#,
        )
        .bind(intent)
        .bind(label)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }

    let applicant_profiles = [
        ("standard", "other", "Стандартный заявитель"),
        ("minor", "age", "Несовершеннолетний заявитель"),
        ("student", "status", "Студент"),
        ("family", "family", "Семейный заявитель"),
    ];
    for (profile_key, profile_type, label_ru) in applicant_profiles {
        let result = sqlx::query(
            r#"
            INSERT INTO kb.applicant_profiles (profile_key, profile_type, label_ru, status)
            VALUES ($1, $2, $3, 'active')
            ON CONFLICT (profile_key) DO UPDATE
            SET profile_type = EXCLUDED.profile_type,
                label_ru = EXCLUDED.label_ru,
                updated_at = now()
            "#,
        )
        .bind(profile_key)
        .bind(profile_type)
        .bind(label_ru)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }
    Ok(inserted)
}

pub async fn ensure_seo_runtime_registries(pool: &PgPool) -> Result<u64, DomainError> {
    seed_runtime_registries(pool).await
}

pub async fn persist_global_navigation_from_active_pages(
    pool: &PgPool,
    market: &str,
    locale: &str,
    reason: &str,
) -> Result<GlobalNavigationReport, DomainError> {
    let market = if market.trim().is_empty() {
        "global"
    } else {
        market.trim()
    };
    let locale = if locale.trim().is_empty() {
        "und"
    } else {
        locale.trim()
    };
    let rows = sqlx::query(
        r#"
        SELECT
            p.page_node_key,
            p.scope_signature,
            COALESCE(p.parent_page_node_key, '') AS parent_page_node_key,
            p.page_type_key,
            p.canonical_slug,
            p.canonical_url_path,
            p.hierarchy_depth,
            p.menu_group,
            COALESCE(k.market, '') AS market,
            COALESCE(k.locale, '') AS locale,
            k.country_code,
            k.visa_type,
            k.applicant_profile
        FROM site.page_nodes p
        LEFT JOIN site.keyword_clusters k ON k.cluster_key = p.keyword_cluster_key
        WHERE p.lifecycle_state NOT IN ('blocked', 'deprecated')
        ORDER BY p.scope_signature, p.hierarchy_depth, p.canonical_url_path
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    let pages = rows
        .into_iter()
        .map(|row| ActivePageForNavigation {
            page_node_key: row.get("page_node_key"),
            scope_signature: row.get("scope_signature"),
            parent_page_node_key: row.get("parent_page_node_key"),
            page_type_key: row.get("page_type_key"),
            canonical_slug: row.get("canonical_slug"),
            canonical_url_path: row.get("canonical_url_path"),
            hierarchy_depth: row.get("hierarchy_depth"),
            menu_group: row.get("menu_group"),
            market: row.get("market"),
            locale: row.get("locale"),
            country_code: row.get("country_code"),
            visa_type: row.get("visa_type"),
            applicant_profile: row.get("applicant_profile"),
        })
        .collect::<Vec<_>>();
    if pages.is_empty() {
        return Ok(GlobalNavigationReport::default());
    }

    let mut scope_page_counts = BTreeMap::<String, i32>::new();
    let mut scope_fields_by_signature = BTreeMap::<
        String,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >::new();
    for page in &pages {
        *scope_page_counts
            .entry(page.scope_signature.clone())
            .or_default() += 1;
        scope_fields_by_signature
            .entry(page.scope_signature.clone())
            .or_insert_with(|| {
                (
                    if page.market.trim().is_empty() {
                        market.to_string()
                    } else {
                        page.market.clone()
                    },
                    if page.locale.trim().is_empty() {
                        locale.to_string()
                    } else {
                        page.locale.clone()
                    },
                    page.country_code.clone(),
                    page.visa_type.clone(),
                    page.applicant_profile.clone(),
                )
            });
    }

    let mut scope_count = 0u64;
    for (
        scope_signature,
        (scope_market, scope_locale, country_code, visa_type, applicant_profile),
    ) in &scope_fields_by_signature
    {
        sqlx::query(
            r#"
            INSERT INTO site.site_scopes
                (scope_signature, market, locale, country_code, visa_type, applicant_profile,
                 page_count, status, last_reconciled_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', now())
            ON CONFLICT (scope_signature) DO UPDATE
            SET market = EXCLUDED.market,
                locale = EXCLUDED.locale,
                country_code = EXCLUDED.country_code,
                visa_type = EXCLUDED.visa_type,
                applicant_profile = EXCLUDED.applicant_profile,
                page_count = EXCLUDED.page_count,
                status = 'active',
                last_reconciled_at = now(),
                updated_at = now()
            "#,
        )
        .bind(scope_signature)
        .bind(scope_market)
        .bind(scope_locale)
        .bind(country_code)
        .bind(visa_type)
        .bind(applicant_profile)
        .bind(*scope_page_counts.get(scope_signature).unwrap_or(&0))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        scope_count += 1;
    }

    let navigation_tree_key = primitives::seo::seo_artifact_key(
        "navigation_tree",
        &[market, locale, "navigation_policy@1"],
    );
    sqlx::query(
        r#"
        INSERT INTO site.navigation_trees
            (navigation_tree_key, market, locale, tree_version, policy_version, status, generated_at)
        VALUES ($1, $2, $3, 1, 'navigation_policy@1', 'active', now())
        ON CONFLICT (navigation_tree_key) DO UPDATE
        SET status = 'active',
            generated_at = now(),
            updated_at = now()
        "#,
    )
    .bind(&navigation_tree_key)
    .bind(market)
    .bind(locale)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query("DELETE FROM site.navigation_items WHERE navigation_tree_key = $1")
        .bind(&navigation_tree_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

    let mut silo_group_by_menu = BTreeMap::<String, String>::new();
    for page in &pages {
        let menu_group = if page.menu_group.trim().is_empty() {
            "global".to_string()
        } else {
            page.menu_group.clone()
        };
        if silo_group_by_menu.contains_key(&menu_group) {
            continue;
        }
        let silo_group_key = primitives::seo::seo_artifact_key("silo_group", &[&menu_group]);
        sqlx::query(
            r#"
            INSERT INTO site.silo_groups
                (silo_group_key, scope_signature, group_type, label, canonical_url_path, sort_order, status)
            VALUES ($1, $2, 'directory', $3, '', 1000, 'active')
            ON CONFLICT (silo_group_key) DO UPDATE
            SET label = EXCLUDED.label,
                status = 'active',
                updated_at = now()
            "#,
        )
        .bind(&silo_group_key)
        .bind(blank_as_none(&page.scope_signature))
        .bind(label_from_slug(&menu_group.replace(':', "-")))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        silo_group_by_menu.insert(menu_group, silo_group_key);
    }

    let page_item_key = pages
        .iter()
        .map(|page| {
            (
                page.page_node_key.clone(),
                primitives::seo::seo_artifact_key(
                    "navigation_item",
                    &[&navigation_tree_key, &page.page_node_key],
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut page_item_count = 0u64;
    for (idx, page) in pages.iter().enumerate() {
        let item_key = page_item_key
            .get(&page.page_node_key)
            .cloned()
            .unwrap_or_else(|| {
                primitives::seo::seo_artifact_key(
                    "navigation_item",
                    &[&navigation_tree_key, &page.page_node_key],
                )
            });
        let parent_item_key = page_item_key
            .get(&page.parent_page_node_key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty());
        let silo_group_key = silo_group_by_menu
            .get(if page.menu_group.trim().is_empty() {
                "global"
            } else {
                page.menu_group.as_str()
            })
            .map(String::as_str);
        sqlx::query(
            r#"
            INSERT INTO site.navigation_items
                (navigation_item_key, navigation_tree_key, parent_item_key, silo_group_key,
                 page_node_key, scope_signature, item_type, label, url_path,
                 hierarchy_depth, sort_order, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'page', $7, $8, $9, $10, 'active')
            ON CONFLICT (navigation_item_key) DO UPDATE
            SET parent_item_key = EXCLUDED.parent_item_key,
                silo_group_key = EXCLUDED.silo_group_key,
                label = EXCLUDED.label,
                url_path = EXCLUDED.url_path,
                hierarchy_depth = EXCLUDED.hierarchy_depth,
                sort_order = EXCLUDED.sort_order,
                status = 'active',
                updated_at = now()
            "#,
        )
        .bind(&item_key)
        .bind(&navigation_tree_key)
        .bind(parent_item_key)
        .bind(silo_group_key)
        .bind(&page.page_node_key)
        .bind(&page.scope_signature)
        .bind(label_from_page(page))
        .bind(&page.canonical_url_path)
        .bind(page.hierarchy_depth)
        .bind(idx as i32)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        page_item_count += 1;
    }

    let rebuild_plan_key =
        primitives::seo::seo_artifact_key("global_rebuild_plan", &[&navigation_tree_key, reason]);
    sqlx::query(
        r#"
        INSERT INTO site.global_rebuild_plan
            (rebuild_plan_key, trigger_type, priority, reason_payload, status)
        VALUES ($1, 'navigation_reconciled', 2, $2, 'queued')
        ON CONFLICT (rebuild_plan_key) DO UPDATE
        SET reason_payload = EXCLUDED.reason_payload,
            status = 'queued',
            updated_at = now()
        "#,
    )
    .bind(&rebuild_plan_key)
    .bind(Json(json!({
        "reason": reason,
        "navigation_tree_key": navigation_tree_key,
        "scope_count": scope_count,
        "page_item_count": page_item_count,
    })))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(GlobalNavigationReport {
        navigation_tree_key,
        scope_count,
        page_item_count,
        silo_group_count: silo_group_by_menu.len() as u64,
        rebuild_plan_count: 1,
    })
}

fn label_from_page(page: &ActivePageForNavigation) -> String {
    let source = if page.canonical_slug.trim().is_empty() {
        page.canonical_url_path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("page")
    } else {
        page.canonical_slug.as_str()
    };
    let mut label = label_from_slug(source);
    match page.page_type_key.as_str() {
        "country_hub_page" if !label.to_ascii_lowercase().contains("visa") => {
            label.push_str(" Visa");
        }
        "fee_page" if !label.to_ascii_lowercase().contains("fee") => label.push_str(" Fees"),
        "timeline_page" if !label.to_ascii_lowercase().contains("time") => {
            label.push_str(" Timeline");
        }
        _ => {}
    }
    label
}

fn label_from_slug(value: &str) -> String {
    let label = value
        .trim_matches('/')
        .split(['-', '_', '/', ':'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        "Page".to_string()
    } else {
        label
    }
}

fn graph_reason_payload(
    reason_code: &str,
    topic_keys: &[String],
    triple_refs: &[String],
    support_refs: &[String],
    graph_confidence: f64,
) -> Value {
    json!({
        "reason_code": reason_code,
        "topic_keys": topic_keys,
        "triple_refs": triple_refs,
        "support_refs": support_refs,
        "graph_confidence": graph_confidence,
        "source": "graph_planning_context",
    })
}

async fn load_latest_step_output_blob<T: DeserializeOwned>(
    pool: &PgPool,
    run_id: Uuid,
    step_name: &str,
) -> Result<Option<T>, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT payload_bytes
        FROM pipeline.step_payload_blobs
        WHERE run_id = $1
          AND step_name = $2
          AND payload_kind = 'output'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(step_name)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let payload_bytes: Vec<u8> = row.get("payload_bytes");
    serde_json::from_slice::<T>(&payload_bytes)
        .map(Some)
        .map_err(|e| contract_violation(format!("decode {step_name} output blob: {e}")))
}

pub async fn load_seo_site_build_input(
    pool: &PgPool,
    run_id: &str,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let run_row = sqlx::query(
        r#"
        SELECT context_key
        FROM pipeline.execution_runs
        WHERE run_id = $1
          AND workflow_type = 'seo_site_build'
        "#,
    )
    .bind(uuid)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure("seo_site_build execution_run row is required"))?;
    let context_key: String = run_row.get("context_key");

    let blob = sqlx::query(
        r#"
        SELECT payload_type, payload_bytes
        FROM pipeline.execution_run_blobs
        WHERE run_id = $1
          AND field_name = 'input_payload'
        LIMIT 1
        "#,
    )
    .bind(uuid)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload input_payload blob is required"))?;
    let payload_type: String = blob.get("payload_type");
    if payload_type != SeoSiteBuildInputPayload::payload_type() {
        return Err(contract_violation(format!(
            "expected {}, got {payload_type}",
            SeoSiteBuildInputPayload::payload_type()
        )));
    }
    let payload_bytes: Vec<u8> = blob.get("payload_bytes");
    let mut input = SeoSiteBuildInputPayload::decode_payload_bytes(&payload_bytes)?;
    if input.run_id != run_id {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload.run_id must match workflow id",
        ));
    }
    if input.context_key != context_key {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload.context_key must match execution_run.context_key",
        ));
    }
    let scope = input
        .scope
        .as_ref()
        .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload.scope is required"))?;
    let validated_scope = identity::derive_scope_from_payload(scope)?;
    validate_applicant_profile_reference(pool, &validated_scope.applicant_profile).await?;
    if input.queries.iter().all(|q| q.trim().is_empty()) {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload requires at least one non-empty query",
        ));
    }

    let ctx = sqlx::query(
        r#"
        SELECT country_code, visa_family, visa_subtype, citizenship_code
        FROM kb.visa_contexts
        WHERE context_key = $1
          AND status = 'active'
        "#,
    )
    .bind(&context_key)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| {
        validation_failure("context_key must reference an active kb.visa_contexts row")
    })?;
    let truth = identity::derive_truth_identity(
        &ctx.get::<String, _>("country_code"),
        &ctx.get::<String, _>("visa_family"),
        ctx.get::<Option<String>, _>("visa_subtype").as_deref(),
        &ctx.get::<String, _>("citizenship_code"),
    )?;
    identity::assert_scope_matches_context(&validated_scope, &truth)?;

    if input.query_batch_key.trim().is_empty() {
        input.query_batch_key = primitives::seo::seo_artifact_key(
            "query_batch",
            &[run_id, &validated_scope.scope_signature],
        );
    }
    if input.run_mode.trim().is_empty() {
        input.run_mode = "publish_with_hitl".to_string();
    }
    Ok(input)
}

pub async fn load_graph_planning_context(
    pool: &PgPool,
    run_id: &str,
    scope_signature: &str,
) -> Result<GraphPlanningContext, DomainError> {
    let run_uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;

    let topic_signals = load_latest_step_output_blob::<EditorialExtractionSweepOutputBlob>(
        pool,
        run_uuid,
        "editorial_extraction",
    )
    .await?
    .map(|payload| {
        payload
            .sections
            .into_iter()
            .flat_map(|section| {
                section.topics.into_iter().map(|topic| GraphPlanningTopicSignal {
                    topic_key: topic.topic_key_candidate.clone(),
                    topic_type: topic.topic_type,
                    support_refs: vec![format!("step://editorial_extraction/{}", topic.topic_key_candidate)],
                    graph_confidence: 0.82,
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let triple_signals = load_latest_step_output_blob::<TripleBuilderSweepOutputBlob>(
        pool,
        run_uuid,
        "triple_builder",
    )
    .await?
    .map(|payload| {
        payload
            .sections
            .into_iter()
            .flat_map(|section| {
                section.triples.into_iter().map(|triple| GraphPlanningTripleSignal {
                    triple_id: triple.triple_id,
                    subject_key: triple.subject_key,
                    relation_type: triple.relation_type,
                    object_key: triple.object_key,
                    support_refs: vec![format!("section://{}", triple.evidence_section_id)],
                    graph_confidence: triple.confidence as f64,
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let keyword_clusters = sqlx::query(
        r#"
        SELECT cluster_key, scope_signature, seed_keyword, dominant_intent, status, cluster_version,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.keyword_clusters
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, cluster_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| KeywordCluster {
        cluster_key: row.get("cluster_key"),
        scope_signature: row.get("scope_signature"),
        seed_keyword: row.get("seed_keyword"),
        dominant_intent: row.get("dominant_intent"),
        status: row.get("status"),
        cluster_version: row.get::<i32, _>("cluster_version") as u32,
        reason_code: row.get("reason_code"),
        topic_keys: row
            .get::<Json<Vec<String>>, _>("topic_keys")
            .0,
        triple_refs: row
            .get::<Json<Vec<String>>, _>("triple_refs")
            .0,
        graph_confidence: row.get("graph_confidence"),
        support_refs: row
            .get::<Json<Vec<String>>, _>("support_refs")
            .0,
    })
    .collect::<Vec<_>>();

    let page_nodes = sqlx::query(
        r#"
        SELECT page_node_key, scope_signature, COALESCE(keyword_cluster_key, '') AS keyword_cluster_key,
               COALESCE(blueprint_key, '') AS blueprint_key, page_type_key, dominant_intent,
               canonical_slug, canonical_url_path, lifecycle_state
        FROM site.page_nodes
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, page_node_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| PageNode {
        page_node_key: row.get("page_node_key"),
        scope_signature: row.get("scope_signature"),
        keyword_cluster_key: row.get("keyword_cluster_key"),
        blueprint_key: row.get("blueprint_key"),
        page_type_key: row.get("page_type_key"),
        dominant_intent: row.get("dominant_intent"),
        canonical_slug: row.get("canonical_slug"),
        canonical_url_path: row.get("canonical_url_path"),
        lifecycle_state: row.get("lifecycle_state"),
    })
    .collect::<Vec<_>>();

    let content_gaps = sqlx::query(
        r#"
        SELECT content_gap_key, scope_signature, COALESCE(page_node_key, '') AS page_node_key,
               missing_topic, severity, status,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.content_gaps
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, content_gap_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| ContentGap {
        content_gap_key: row.get("content_gap_key"),
        scope_signature: row.get("scope_signature"),
        page_node_key: row.get("page_node_key"),
        missing_topic: row.get("missing_topic"),
        severity: row.get("severity"),
        status: row.get("status"),
        reason_code: row.get("reason_code"),
        topic_keys: row
            .get::<Json<Vec<String>>, _>("topic_keys")
            .0,
        triple_refs: row
            .get::<Json<Vec<String>>, _>("triple_refs")
            .0,
        graph_confidence: row.get("graph_confidence"),
        support_refs: row
            .get::<Json<Vec<String>>, _>("support_refs")
            .0,
    })
    .collect::<Vec<_>>();

    let link_recommendation_rows = sqlx::query(
        r#"
        SELECT link_recommendation_key, scope_signature, source_page_key, target_page_key,
               link_role, anchor_strategy, required_flag, score::double precision AS score, status,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.link_recommendations
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, link_recommendation_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    let mut link_recommendations = Vec::with_capacity(link_recommendation_rows.len());
    for row in link_recommendation_rows {
        let recommendation = LinkRecommendation {
            link_recommendation_key: row.try_get("link_recommendation_key").map_err(classify_sqlx)?,
            scope_signature: row.try_get("scope_signature").map_err(classify_sqlx)?,
            source_page_key: row.try_get("source_page_key").map_err(classify_sqlx)?,
            target_page_key: row.try_get("target_page_key").map_err(classify_sqlx)?,
            link_role: row.try_get("link_role").map_err(classify_sqlx)?,
            anchor_strategy: row.try_get("anchor_strategy").map_err(classify_sqlx)?,
            required_flag: row.try_get("required_flag").map_err(classify_sqlx)?,
            score: row.try_get::<f64, _>("score").map_err(classify_sqlx)?,
            status: row.try_get("status").map_err(classify_sqlx)?,
            reason_code: row.try_get("reason_code").map_err(classify_sqlx)?,
            topic_keys: row
                .try_get::<Json<Vec<String>>, _>("topic_keys")
                .map_err(classify_sqlx)?
                .0,
            triple_refs: row
                .try_get::<Json<Vec<String>>, _>("triple_refs")
                .map_err(classify_sqlx)?
                .0,
            graph_confidence: row.try_get("graph_confidence").map_err(classify_sqlx)?,
            support_refs: row
                .try_get::<Json<Vec<String>>, _>("support_refs")
                .map_err(classify_sqlx)?
                .0,
        };
        link_recommendations.push(recommendation);
    }

    let coverage_signals = page_nodes
        .iter()
        .map(|page| {
            let cluster_topics = keyword_clusters
                .iter()
                .find(|cluster| cluster.cluster_key == page.keyword_cluster_key)
                .map(|cluster| cluster.topic_keys.clone())
                .unwrap_or_default();
            let missing_topic_keys = content_gaps
                .iter()
                .filter(|gap| gap.page_node_key == page.page_node_key || gap.page_node_key.is_empty())
                .map(|gap| gap.missing_topic.clone())
                .collect::<Vec<_>>();
            GraphPlanningCoverageSignal {
                page_node_key: page.page_node_key.clone(),
                keyword_cluster_key: page.keyword_cluster_key.clone(),
                covered_topic_keys: cluster_topics,
                missing_topic_keys,
                graph_confidence: 0.75,
            }
        })
        .collect::<Vec<_>>();

    Ok(GraphPlanningContext {
        scope_signature: scope_signature.to_string(),
        topic_signals,
        triple_signals,
        coverage_signals,
        keyword_clusters,
        page_nodes,
        content_gaps,
        link_recommendations,
    })
}

pub async fn load_section_templates(
    pool: &PgPool,
    page_type_key: &str,
    dominant_intent: &str,
) -> Result<Vec<SectionTemplateBinding>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT section_template_key, page_type_key, dominant_intent, section_role,
               template_version, template_body
        FROM site.section_templates
        WHERE page_type_key = $1
          AND dominant_intent = $2
          AND status = 'active'
        ORDER BY section_role, template_version DESC
        "#,
    )
    .bind(page_type_key)
    .bind(dominant_intent)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut seen_roles = std::collections::BTreeSet::new();
    let mut templates = Vec::new();
    for row in rows {
        let section_role: String = row.get("section_role");
        if !seen_roles.insert(section_role.clone()) {
            continue;
        }
        templates.push(SectionTemplateBinding {
            template_key: row.get("section_template_key"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            section_role: section_role.clone(),
            heading: runtime_models::seo_blocks::heading_for_role(&section_role).to_string(),
            template_body: row.get("template_body"),
            template_version: row.get::<i32, _>("template_version") as u32,
            required: true,
        });
    }
    Ok(templates)
}

pub async fn upsert_seo_site_build_input(
    pool: &PgPool,
    input: &SeoSiteBuildInputPayload,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(&input.run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    non_empty(&input.context_key, "context_key")?;
    let scope = input
        .scope
        .as_ref()
        .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload.scope is required"))?;
    let validated_scope = identity::derive_scope_from_payload(scope)?;
    validate_applicant_profile_reference(pool, &validated_scope.applicant_profile).await?;
    let payload_bytes = input.encode_payload_bytes()?;
    let payload_hash = primitives::hash::blake3_hex(&payload_bytes);

    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_runs
            (run_id, workflow_run_id, workflow_type, context_key, status)
        VALUES ($1, $2, 'seo_site_build', $3, 'created')
        ON CONFLICT (run_id) DO UPDATE
        SET workflow_run_id = EXCLUDED.workflow_run_id,
            workflow_type   = EXCLUDED.workflow_type,
            context_key     = EXCLUDED.context_key,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(uuid)
    .bind(&input.run_id)
    .bind(&input.context_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, 'input_payload', $2, $3, $4, $5)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type    = EXCLUDED.payload_type,
            schema_version  = EXCLUDED.schema_version,
            payload_bytes   = EXCLUDED.payload_bytes,
            payload_hash    = EXCLUDED.payload_hash,
            updated_at      = now()
        "#,
    )
    .bind(uuid)
    .bind(SeoSiteBuildInputPayload::payload_type())
    .bind(SeoSiteBuildInputPayload::schema_version())
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(())
}

pub async fn persist_serp_ingest_output(
    pool: &PgPool,
    input: &SerpIngestInputPayload,
    output: &SerpIngestOutputPayload,
) -> Result<(), DomainError> {
    let fallback_scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.as_str())
        .filter(|scope| !scope.trim().is_empty())
        .unwrap_or("default");
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);
    let query_batch_key = if output.query_batch_key.trim().is_empty() {
        &input.query_batch_key
    } else {
        &output.query_batch_key
    };
    non_empty(query_batch_key, "query_batch_key")?;

    sqlx::query(
        r#"
        INSERT INTO serp.query_batches
            (query_batch_key, scope_signature, market, locale, source_system, batch_version, status)
        VALUES ($1, $2, $3, $4, 'temporal_serp_ingest', 1, $5)
        ON CONFLICT (query_batch_key) DO UPDATE
        SET scope_signature = EXCLUDED.scope_signature,
            market          = EXCLUDED.market,
            locale          = EXCLUDED.locale,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(query_batch_key)
    .bind(&scope.scope_signature)
    .bind(&scope.market)
    .bind(&scope.locale)
    .bind(&output.status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    for (idx, query) in input.queries.iter().enumerate() {
        if query.trim().is_empty() {
            continue;
        }
        let job_id = format!("seo-query-{:04}", idx + 1);
        sqlx::query(
            r#"
            INSERT INTO serp.raw_snapshots
                (run_id, job_id, query, recorded_at, raw_result)
            VALUES ($1, $2, $3, now(), $4)
            ON CONFLICT (run_id, job_id) DO UPDATE
            SET query       = EXCLUDED.query,
                recorded_at = EXCLUDED.recorded_at,
                raw_result  = EXCLUDED.raw_result
            "#,
        )
        .bind(&input.run_id)
        .bind(job_id)
        .bind(query.trim())
        .bind(Json(json!({
            "query": query.trim(),
            "query_batch_key": query_batch_key,
            "source_system": "seo_site_build_input",
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(())
}

pub async fn persist_serp_normalize_output(
    pool: &PgPool,
    input: &SerpNormalizeInputPayload,
    output: &SerpNormalizeOutputPayload,
) -> Result<(), DomainError> {
    let fallback_scope_signature = if output.scope_signature.trim().is_empty() {
        "default"
    } else {
        &output.scope_signature
    };
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);
    let query_batch_key = output
        .serp_patterns
        .first()
        .and_then(|p| blank_as_none(&p.query_batch_key))
        .or_else(|| blank_as_none(&input.query_batch_key))
        .ok_or_else(|| validation_failure("SEO persistence requires query_batch_key"))?;

    sqlx::query(
        r#"
        INSERT INTO serp.query_batches
            (query_batch_key, scope_signature, market, locale, source_system, batch_version, status)
        VALUES ($1, $2, $3, $4, 'temporal_serp_normalize', 1, 'done')
        ON CONFLICT (query_batch_key) DO UPDATE
        SET scope_signature = EXCLUDED.scope_signature,
            market          = EXCLUDED.market,
            locale          = EXCLUDED.locale,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(query_batch_key)
    .bind(&scope.scope_signature)
    .bind(&scope.market)
    .bind(&scope.locale)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut projection_events = Vec::new();
    for pattern in &output.serp_patterns {
        non_empty(&pattern.serp_pattern_key, "serp_pattern_key")?;
        non_empty(&pattern.query, "serp_pattern.query")?;
        sqlx::query(
            r#"
            INSERT INTO serp.serp_patterns
                (serp_pattern_key, query_batch_key, scope_signature, query, pattern_type,
                 dominant_intent, reliability_score, evidence_ref, pattern_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, 'seo_serp_pattern@1', $9)
            ON CONFLICT (serp_pattern_key) DO UPDATE
            SET query_batch_key   = EXCLUDED.query_batch_key,
                scope_signature   = EXCLUDED.scope_signature,
                query             = EXCLUDED.query,
                pattern_type      = EXCLUDED.pattern_type,
                dominant_intent   = EXCLUDED.dominant_intent,
                reliability_score = EXCLUDED.reliability_score,
                evidence_ref      = EXCLUDED.evidence_ref,
                status            = EXCLUDED.status,
                updated_at        = now()
            "#,
        )
        .bind(&pattern.serp_pattern_key)
        .bind(&pattern.query_batch_key)
        .bind(&pattern.scope_signature)
        .bind(&pattern.query)
        .bind(&pattern.pattern_type)
        .bind(&pattern.dominant_intent)
        .bind(pattern.reliability_score)
        .bind(blank_as_none(&pattern.evidence_ref))
        .bind(&pattern.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        sqlx::query(
            r#"
            INSERT INTO serp.serp_pattern_observations
                (observation_key, serp_pattern_key, observed_at, observation_payload)
            VALUES ($1, $2, now(), $3)
            ON CONFLICT (observation_key) DO UPDATE
            SET observation_payload = EXCLUDED.observation_payload,
                observed_at = now()
            "#,
        )
        .bind(primitives::seo::seo_artifact_key(
            "serp_pattern_observation",
            &[
                &pattern.serp_pattern_key,
                &pattern.query_batch_key,
                &pattern.query,
            ],
        ))
        .bind(&pattern.serp_pattern_key)
        .bind(Json(json!({
            "query": pattern.query,
            "pattern_type": pattern.pattern_type,
            "dominant_intent": pattern.dominant_intent,
            "status": pattern.status,
            "evidence_ref": pattern.evidence_ref,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if pattern.status == "partial" {
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                VALUES ($1, 'unsafe_serp_pattern', 'open', 'seo_ops', 'seo_ops',
                        $2, 'serp_normalize', $3, 'important', $4, $5)
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    decision_payload = EXCLUDED.decision_payload,
                    updated_at = now()
                "#,
            )
            .bind(primitives::seo::seo_artifact_key(
                "seo_hitl_task",
                &[
                    &pattern.serp_pattern_key,
                    "unsafe_serp_pattern",
                    "serp_normalize@1",
                ],
            ))
            .bind(&pattern.scope_signature)
            .bind(&pattern.serp_pattern_key)
            .bind(Json(json!({
                "query": pattern.query,
                "dominant_intent": pattern.dominant_intent,
                "reliability_score": pattern.reliability_score,
            })))
            .bind(Json(json!([{
                "event": "unsafe_serp_pattern_detected",
                "query": pattern.query,
            }])))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }

        projection_events.push(seo_graph_projection_event(
            "serp_pattern",
            &pattern.serp_pattern_key,
            &pattern.scope_signature,
        ));
        projection_events.push(seo_qdrant_projection_event(
            "seo_serp_patterns",
            "serp_pattern",
            &pattern.serp_pattern_key,
            &pattern.scope_signature,
            &format!(
                "{} {} {} {}",
                pattern.query, pattern.pattern_type, pattern.dominant_intent, pattern.status
            ),
            HashMap::from([
                ("query".to_string(), pattern.query.clone()),
                ("pattern_type".to_string(), pattern.pattern_type.clone()),
                (
                    "dominant_intent".to_string(),
                    pattern.dominant_intent.clone(),
                ),
                ("status".to_string(), pattern.status.clone()),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_opportunity_build_output(
    pool: &PgPool,
    input: &OpportunityBuildInputPayload,
    output: &OpportunityBuildOutputPayload,
) -> Result<(), DomainError> {
    let fallback_scope_signature = input
        .scope
        .as_ref()
        .and_then(|s| blank_as_none(&s.scope_signature))
        .unwrap_or("default");
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);

    let mut projection_events = Vec::new();
    for cluster in &output.keyword_clusters {
        non_empty(&cluster.cluster_key, "cluster_key")?;
        non_empty(&cluster.seed_keyword, "seed_keyword")?;
        let cluster_scope = if cluster.scope_signature.trim().is_empty() {
            &scope.scope_signature
        } else {
            &cluster.scope_signature
        };
        sqlx::query(
            r#"
            INSERT INTO site.keyword_clusters
                (cluster_key, scope_signature, market, locale, country_code, visa_type,
                 applicant_profile, seed_keyword, dominant_intent, cluster_version,
                 derivation_version, status, reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'seo_cluster@1', $11, $12, 'graph_planning@1')
            ON CONFLICT (cluster_key) DO UPDATE
            SET scope_signature   = EXCLUDED.scope_signature,
                market            = EXCLUDED.market,
                locale            = EXCLUDED.locale,
                country_code      = EXCLUDED.country_code,
                visa_type         = EXCLUDED.visa_type,
                applicant_profile = EXCLUDED.applicant_profile,
                seed_keyword      = EXCLUDED.seed_keyword,
                dominant_intent   = EXCLUDED.dominant_intent,
                cluster_version   = EXCLUDED.cluster_version,
                reason_payload    = EXCLUDED.reason_payload,
                reason_version    = EXCLUDED.reason_version,
                status            = EXCLUDED.status,
                updated_at        = now()
            "#,
        )
        .bind(&cluster.cluster_key)
        .bind(cluster_scope)
        .bind(&scope.market)
        .bind(&scope.locale)
        .bind(&scope.country_code)
        .bind(&scope.visa_type)
        .bind(&scope.applicant_profile)
        .bind(&cluster.seed_keyword)
        .bind(&cluster.dominant_intent)
        .bind(cluster.cluster_version as i32)
        .bind(&cluster.status)
        .bind(Json(graph_reason_payload(
            &cluster.reason_code,
            &cluster.topic_keys,
            &cluster.triple_refs,
            &cluster.support_refs,
            cluster.graph_confidence,
        )))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "keyword_cluster",
            &cluster.cluster_key,
            cluster_scope,
        ));
        projection_events.push(seo_qdrant_projection_event(
            "seo_keyword_clusters",
            "keyword_cluster",
            &cluster.cluster_key,
            cluster_scope,
            &format!(
                "{} {} {} {}",
                cluster.seed_keyword, cluster.dominant_intent, cluster.status, cluster.cluster_key
            ),
            HashMap::from([
                ("seed_keyword".to_string(), cluster.seed_keyword.clone()),
                (
                    "dominant_intent".to_string(),
                    cluster.dominant_intent.clone(),
                ),
                ("status".to_string(), cluster.status.clone()),
            ]),
        ));
    }

    for pattern in &input.serp_patterns {
        if pattern.query.trim().is_empty() {
            continue;
        }
        sqlx::query(
            r#"
            INSERT INTO serp.opportunity_candidates
                (opportunity_key, scope_signature, query_batch_key, serp_pattern_key,
                 seed_keyword, dominant_intent, opportunity_score, recommended_action,
                 status, scoring_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7::numeric,
                    'create_or_refresh_page', 'candidate', 'seo_opportunity@1')
            ON CONFLICT (opportunity_key) DO UPDATE
            SET opportunity_score = EXCLUDED.opportunity_score,
                recommended_action = EXCLUDED.recommended_action,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(primitives::seo::seo_artifact_key(
            "opportunity",
            &[
                &pattern.scope_signature,
                &pattern.query,
                &pattern.dominant_intent,
                "seo_opportunity@1",
            ],
        ))
        .bind(&pattern.scope_signature)
        .bind(blank_as_none(&pattern.query_batch_key))
        .bind(blank_as_none(&pattern.serp_pattern_key))
        .bind(&pattern.query)
        .bind(&pattern.dominant_intent)
        .bind(pattern.reliability_score)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    for gap in &output.content_gaps {
        non_empty(&gap.content_gap_key, "content_gap_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.content_gaps
                (content_gap_key, scope_signature, page_node_key, missing_topic,
                 severity, detector_version, status, reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, 'seo_content_gap@1', $6, $7, 'graph_planning@1')
            ON CONFLICT (content_gap_key) DO UPDATE
            SET page_node_key = EXCLUDED.page_node_key,
                missing_topic = EXCLUDED.missing_topic,
                severity      = EXCLUDED.severity,
                reason_payload = EXCLUDED.reason_payload,
                reason_version = EXCLUDED.reason_version,
                status        = EXCLUDED.status,
                updated_at    = now()
            "#,
        )
        .bind(&gap.content_gap_key)
        .bind(&gap.scope_signature)
        .bind(blank_as_none(&gap.page_node_key))
        .bind(&gap.missing_topic)
        .bind(&gap.severity)
        .bind(&gap.status)
        .bind(Json(graph_reason_payload(
            &gap.reason_code,
            &gap.topic_keys,
            &gap.triple_refs,
            &gap.support_refs,
            gap.graph_confidence,
        )))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "content_gap",
            &gap.content_gap_key,
            &gap.scope_signature,
        ));
        projection_events.push(seo_qdrant_projection_event(
            "seo_content_gaps",
            "content_gap",
            &gap.content_gap_key,
            &gap.scope_signature,
            &format!("{} {} {}", gap.missing_topic, gap.severity, gap.status),
            HashMap::from([
                ("missing_topic".to_string(), gap.missing_topic.clone()),
                ("severity".to_string(), gap.severity.clone()),
                ("status".to_string(), gap.status.clone()),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_ia_build_output(
    pool: &PgPool,
    input: &IaBuildInputPayload,
    output: &IaBuildOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    for blueprint in &output.page_blueprints {
        non_empty(&blueprint.blueprint_key, "blueprint_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_blueprints
                (blueprint_key, page_type_key, dominant_intent, scope_class,
                 blueprint_version, title_pattern, section_plan, derivation_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'seo_blueprint@1', $8)
            ON CONFLICT (blueprint_key) DO UPDATE
            SET page_type_key      = EXCLUDED.page_type_key,
                dominant_intent    = EXCLUDED.dominant_intent,
                scope_class        = EXCLUDED.scope_class,
                blueprint_version  = EXCLUDED.blueprint_version,
                title_pattern      = EXCLUDED.title_pattern,
                section_plan       = EXCLUDED.section_plan,
                status             = EXCLUDED.status,
                updated_at         = now()
            "#,
        )
        .bind(&blueprint.blueprint_key)
        .bind(&blueprint.page_type_key)
        .bind(&blueprint.dominant_intent)
        .bind(&blueprint.scope_class)
        .bind(blueprint.blueprint_version as i32)
        .bind(&blueprint.title_pattern)
        .bind(Json(json!({
            "required_sections": blueprint.required_sections.clone(),
            "canonical_url_family": blueprint.canonical_url_family.clone(),
        })))
        .bind(&blueprint.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "page_blueprint",
            &blueprint.blueprint_key,
            "",
        ));
        projection_events.push(seo_qdrant_projection_event(
            "seo_page_blueprints",
            "page_blueprint",
            &blueprint.blueprint_key,
            "",
            &format!(
                "{} {} {} {}",
                blueprint.page_type_key,
                blueprint.dominant_intent,
                blueprint.scope_class,
                blueprint.status
            ),
            HashMap::from([
                ("page_type_key".to_string(), blueprint.page_type_key.clone()),
                (
                    "dominant_intent".to_string(),
                    blueprint.dominant_intent.clone(),
                ),
                ("scope_class".to_string(), blueprint.scope_class.clone()),
                ("status".to_string(), blueprint.status.clone()),
            ]),
        ));
    }

    for page in &output.page_nodes {
        non_empty(&page.page_node_key, "page_node_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_nodes
                (page_node_key, scope_signature, keyword_cluster_key, blueprint_key,
                 page_type_key, dominant_intent, canonical_slug, canonical_url_path,
                 parent_page_node_key, hierarchy_depth, menu_group, breadcrumb_policy,
                 canonical_url_family, lifecycle_state, derivation_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    'seo_page_node@1')
            ON CONFLICT (page_node_key) DO UPDATE
            SET scope_signature     = EXCLUDED.scope_signature,
                keyword_cluster_key = EXCLUDED.keyword_cluster_key,
                blueprint_key       = EXCLUDED.blueprint_key,
                page_type_key       = EXCLUDED.page_type_key,
                dominant_intent     = EXCLUDED.dominant_intent,
                canonical_slug      = EXCLUDED.canonical_slug,
                canonical_url_path  = EXCLUDED.canonical_url_path,
                parent_page_node_key = EXCLUDED.parent_page_node_key,
                hierarchy_depth     = EXCLUDED.hierarchy_depth,
                menu_group          = EXCLUDED.menu_group,
                breadcrumb_policy   = EXCLUDED.breadcrumb_policy,
                canonical_url_family = EXCLUDED.canonical_url_family,
                lifecycle_state     = EXCLUDED.lifecycle_state,
                updated_at          = now()
            "#,
        )
        .bind(&page.page_node_key)
        .bind(&page.scope_signature)
        .bind(blank_as_none(&page.keyword_cluster_key))
        .bind(blank_as_none(&page.blueprint_key))
        .bind(&page.page_type_key)
        .bind(&page.dominant_intent)
        .bind(&page.canonical_slug)
        .bind(&page.canonical_url_path)
        .bind(blank_as_none(&page.parent_page_node_key))
        .bind(page.hierarchy_depth as i32)
        .bind(&page.menu_group)
        .bind(&page.breadcrumb_policy)
        .bind(&page.canonical_url_family)
        .bind(&page.lifecycle_state)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if !page.blueprint_key.trim().is_empty() {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'blueprint', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(primitives::seo::seo_artifact_key(
                "rebuild_dependency",
                &[&page.page_node_key, "blueprint", &page.blueprint_key],
            ))
            .bind(&page.page_node_key)
            .bind(&page.blueprint_key)
            .bind(Json(json!({
                "page_type_key": page.page_type_key,
                "dominant_intent": page.dominant_intent,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
        if !page.keyword_cluster_key.trim().is_empty() {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'keyword_cluster', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(primitives::seo::seo_artifact_key(
                "rebuild_dependency",
                &[&page.page_node_key, "keyword_cluster", &page.keyword_cluster_key],
            ))
            .bind(&page.page_node_key)
            .bind(&page.keyword_cluster_key)
            .bind(Json(json!({
                "page_type_key": page.page_type_key,
                "dominant_intent": page.dominant_intent,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;

            if let Some(seed_keyword) = sqlx::query_scalar::<_, String>(
                r#"
                SELECT seed_keyword
                FROM site.keyword_clusters
                WHERE cluster_key = $1
                LIMIT 1
                "#,
            )
            .bind(&page.keyword_cluster_key)
            .fetch_optional(pool)
            .await
            .map_err(classify_sqlx)?
            {
                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'serp_query', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "rebuild_dependency",
                    &[&page.page_node_key, "serp_query", &seed_keyword],
                ))
                .bind(&page.page_node_key)
                .bind(&seed_keyword)
                .bind(Json(json!({
                    "keyword_cluster_key": page.keyword_cluster_key,
                    "page_type_key": page.page_type_key,
                    "dominant_intent": page.dominant_intent,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
            }
        }

        let navigation_state_ref = format!(
            "{}|{}|{}",
            page.menu_group, page.breadcrumb_policy, page.canonical_url_family
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_rebuild_dependencies
                (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
            VALUES ($1, $2, 'navigation_state', $3, $4, 'active')
            ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
            SET reason_package = EXCLUDED.reason_package,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(primitives::seo::seo_artifact_key(
            "rebuild_dependency",
            &[&page.page_node_key, "navigation_state", &navigation_state_ref],
        ))
        .bind(&page.page_node_key)
        .bind(&navigation_state_ref)
        .bind(Json(json!({
            "menu_group": page.menu_group,
            "breadcrumb_policy": page.breadcrumb_policy,
            "canonical_url_family": page.canonical_url_family,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "page_node",
            &page.page_node_key,
            &page.scope_signature,
        ));
        projection_events.push(seo_qdrant_projection_event(
            "seo_link_targets",
            "page_node",
            &page.page_node_key,
            &page.scope_signature,
            &format!(
                "{} {} {} {}",
                page.canonical_url_path,
                page.page_type_key,
                page.dominant_intent,
                page.lifecycle_state
            ),
            HashMap::from([
                (
                    "canonical_url_path".to_string(),
                    page.canonical_url_path.clone(),
                ),
                ("page_type_key".to_string(), page.page_type_key.clone()),
                ("dominant_intent".to_string(), page.dominant_intent.clone()),
                ("lifecycle_state".to_string(), page.lifecycle_state.clone()),
                ("menu_group".to_string(), page.menu_group.clone()),
                (
                    "canonical_url_family".to_string(),
                    page.canonical_url_family.clone(),
                ),
            ]),
        ));
    }

    for conflict in &output.cannibalization_conflicts {
        non_empty(&conflict.conflict_key, "conflict_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.cannibalization_conflicts
                (conflict_key, scope_signature, page_key_a, page_key_b,
                 conflict_reason, severity, detector_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'seo_cannibalization@1', $7)
            ON CONFLICT (conflict_key) DO UPDATE
            SET conflict_reason = EXCLUDED.conflict_reason,
                severity        = EXCLUDED.severity,
                status          = EXCLUDED.status,
                updated_at      = now()
            "#,
        )
        .bind(&conflict.conflict_key)
        .bind(&conflict.scope_signature)
        .bind(&conflict.page_key_a)
        .bind(&conflict.page_key_b)
        .bind(&conflict.conflict_reason)
        .bind(&conflict.severity)
        .bind(&conflict.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "cannibalization_conflict",
            &conflict.conflict_key,
            &conflict.scope_signature,
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_link_recommend_output(
    pool: &PgPool,
    input: &LinkRecommendInputPayload,
    output: &LinkRecommendOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    for link in &output.link_recommendations {
        non_empty(&link.link_recommendation_key, "link_recommendation_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.link_recommendations
                (link_recommendation_key, scope_signature, source_page_key, target_page_key,
                 link_role, anchor_strategy, required_flag, score, scoring_version, status,
                 reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8::numeric, 'seo_link_score@1', $9, $10, 'graph_planning@1')
            ON CONFLICT (link_recommendation_key) DO UPDATE
            SET required_flag   = EXCLUDED.required_flag,
                score           = EXCLUDED.score,
                reason_payload  = EXCLUDED.reason_payload,
                reason_version  = EXCLUDED.reason_version,
                status          = EXCLUDED.status,
                updated_at      = now()
            "#,
        )
        .bind(&link.link_recommendation_key)
        .bind(&link.scope_signature)
        .bind(&link.source_page_key)
        .bind(&link.target_page_key)
        .bind(&link.link_role)
        .bind(&link.anchor_strategy)
        .bind(link.required_flag)
        .bind(link.score)
        .bind(&link.status)
        .bind(Json(graph_reason_payload(
            &link.reason_code,
            &link.topic_keys,
            &link.triple_refs,
            &link.support_refs,
            link.graph_confidence,
        )))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if link.required_flag {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'required_link', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(primitives::seo::seo_artifact_key(
                "rebuild_dependency",
                &[&link.source_page_key, "required_link", &link.target_page_key],
            ))
            .bind(&link.source_page_key)
            .bind(&link.target_page_key)
            .bind(Json(json!({
                "link_role": link.link_role,
                "anchor_strategy": link.anchor_strategy,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }

        projection_events.push(seo_graph_projection_event(
            "link_recommendation",
            &link.link_recommendation_key,
            &link.scope_signature,
        ));
    }
    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_draft_assemble_output(
    pool: &PgPool,
    run_id: &str,
    output: &DraftAssembleOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    non_empty(run_id, "run_id")?;
    if let Some(brief) = output.page_brief.as_ref() {
        non_empty(&brief.page_brief_key, "page_brief_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_briefs
                (page_brief_key, page_node_key, blueprint_key, title, meta_description,
                 required_sections, required_links, evidence_manifest, brief_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (page_brief_key) DO UPDATE
            SET title            = EXCLUDED.title,
                meta_description = EXCLUDED.meta_description,
                status           = EXCLUDED.status,
                updated_at       = now()
            "#,
        )
        .bind(&brief.page_brief_key)
        .bind(&brief.page_node_key)
        .bind(&brief.blueprint_key)
        .bind(&brief.title)
        .bind(&brief.meta_description)
        .bind(Json(json!(brief.required_sections)))
        .bind(Json(json!(brief.required_links)))
        .bind(Json(json!({
            "truth_snapshot_ref": brief.truth_snapshot_ref,
            "metadata_obligations": brief.metadata_obligations.clone(),
            "target_audience": brief.target_audience.clone(),
            "goal": brief.goal.clone(),
            "editorial_brief_key": output
                .editorial_brief
                .as_ref()
                .map(|brief| brief.editorial_brief_key.clone())
                .unwrap_or_default(),
            "llm_request_key": output
                .llm_request
                .as_ref()
                .map(|request| request.request_key.clone())
                .unwrap_or_default(),
        })))
        .bind(brief.brief_version as i32)
        .bind(&brief.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "page_brief",
            &brief.page_brief_key,
            "",
        ));
    }

    if let Some(plan) = output.content_block_plan.as_ref() {
        upsert_runtime_blob(pool, run_id, "content_block_plan", plan).await?;
    }

    if let Some(draft) = output.draft.as_ref() {
        non_empty(&draft.page_draft_key, "page_draft_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_drafts
                (page_draft_key, page_node_key, page_brief_key, draft_revision,
                 body_markdown, traceability_manifest, qa_verdict, qa_blockers,
                 truth_snapshot_ref, assembly_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'seo_draft_assemble@1')
            ON CONFLICT (page_draft_key) DO UPDATE
            SET body_markdown      = EXCLUDED.body_markdown,
                traceability_manifest = EXCLUDED.traceability_manifest,
                qa_verdict         = EXCLUDED.qa_verdict,
                qa_blockers        = EXCLUDED.qa_blockers,
                truth_snapshot_ref = EXCLUDED.truth_snapshot_ref,
                assembly_version   = EXCLUDED.assembly_version,
                updated_at         = now()
            "#,
        )
        .bind(&draft.page_draft_key)
        .bind(&draft.page_node_key)
        .bind(&draft.page_brief_key)
        .bind(draft.draft_revision as i32)
        .bind(&draft.body_markdown)
        .bind(Json(json!({
            "traceability_entries": draft.traceability_entries.clone(),
            "claim_ledger": draft.claim_ledger.clone(),
            "content_blocks": draft.content_blocks.clone(),
            "faq_json": draft.faq_json.clone(),
            "schema_markup_json": draft.schema_markup_json.clone(),
            "llm_provider_key": draft.llm_provider_key.clone(),
            "llm_model_key": draft.llm_model_key.clone(),
            "generation_request_key": draft.generation_request_key.clone(),
        })))
        .bind(&draft.qa_verdict)
        .bind(Json(json!([])))
        .bind(&draft.truth_snapshot_ref)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        set_page_lifecycle(pool, &draft.page_node_key, "draft_ready", "draft_assembled").await?;

        for claim in &draft.claim_ledger {
            for support_ref in &claim.support_refs {
                if support_ref.trim().is_empty() {
                    continue;
                }
                sqlx::query(
                    r#"
                    INSERT INTO site.page_support_bindings
                        (page_support_binding_key, page_node_key, page_draft_key, support_ref,
                         source_section_key, fragment_kind, traceability_label)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (page_node_key, page_draft_key, support_ref, source_section_key, fragment_kind) DO UPDATE
                    SET traceability_label = EXCLUDED.traceability_label,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "page_support_binding",
                    &[
                        &draft.page_node_key,
                        &draft.page_draft_key,
                        support_ref,
                        &claim.source_section_key,
                        &claim.claim_kind,
                    ],
                ))
                .bind(&draft.page_node_key)
                .bind(&draft.page_draft_key)
                .bind(support_ref)
                .bind(&claim.source_section_key)
                .bind(&claim.claim_kind)
                .bind(&claim.traceability_label)
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;

                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'truth_support', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "rebuild_dependency",
                    &[&draft.page_node_key, "truth_support", support_ref],
                ))
                .bind(&draft.page_node_key)
                .bind(support_ref)
                .bind(Json(json!({
                    "page_draft_key": draft.page_draft_key,
                    "source_section_key": claim.source_section_key,
                    "claim_kind": claim.claim_kind,
                    "traceability_label": claim.traceability_label,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;

                if let Some(source_key) = sqlx::query_scalar::<_, String>(
                    r#"
                    SELECT source_key
                    FROM verified.rule_instances
                    WHERE rule_instance_id = $1
                      AND source_key IS NOT NULL
                      AND source_key <> ''
                    LIMIT 1
                    "#,
                )
                .bind(support_ref)
                .fetch_optional(pool)
                .await
                .map_err(classify_sqlx)?
                {
                    sqlx::query(
                        r#"
                        INSERT INTO monitoring.seo_rebuild_dependencies
                            (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                        VALUES ($1, $2, 'source_provenance', $3, $4, 'active')
                        ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                        SET reason_package = EXCLUDED.reason_package,
                            status = EXCLUDED.status,
                            updated_at = now()
                        "#,
                    )
                    .bind(primitives::seo::seo_artifact_key(
                        "rebuild_dependency",
                        &[&draft.page_node_key, "source_provenance", &source_key],
                    ))
                    .bind(&draft.page_node_key)
                    .bind(&source_key)
                    .bind(Json(json!({
                        "page_draft_key": draft.page_draft_key,
                        "support_ref": support_ref,
                        "source_section_key": claim.source_section_key,
                    })))
                    .execute(pool)
                    .await
                    .map_err(classify_sqlx)?;
                }
            }
        }

        if let Some(plan) = output.content_block_plan.as_ref() {
            for item in &plan.items {
                if item.template_key.trim().is_empty() {
                    continue;
                }
                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'section_template', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "rebuild_dependency",
                    &[&draft.page_node_key, "section_template", &item.template_key],
                ))
                .bind(&draft.page_node_key)
                .bind(&item.template_key)
                .bind(Json(json!({
                    "page_draft_key": draft.page_draft_key,
                    "section_role": item.section_role,
                    "block_type": item.block_type,
                    "required": item.required,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
            }
        }

        projection_events.push(seo_qdrant_projection_event(
            "seo_draft_support_sections",
            "page_draft",
            &draft.page_draft_key,
            "",
            &draft.body_markdown,
            HashMap::from([
                ("page_node_key".to_string(), draft.page_node_key.clone()),
                ("page_brief_key".to_string(), draft.page_brief_key.clone()),
                ("qa_verdict".to_string(), draft.qa_verdict.clone()),
                (
                    "truth_snapshot_ref".to_string(),
                    draft.truth_snapshot_ref.clone(),
                ),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_draft_normalize_output(
    pool: &PgPool,
    input: &DraftNormalizeInputPayload,
    output: &DraftNormalizeOutputPayload,
) -> Result<(), DomainError> {
    upsert_runtime_blob(pool, &input.run_id, "draft_normalize_output", output).await?;
    let output_as_assemble = DraftAssembleOutputPayload {
        page_brief: None,
        draft: output.draft.clone(),
        editorial_brief: None,
        llm_request: None,
        content_block_plan: output.content_block_plan.clone(),
    };
    persist_draft_assemble_output(pool, &input.run_id, &output_as_assemble).await
}

pub async fn persist_content_contract_validate_output(
    pool: &PgPool,
    input: &ContentContractValidateInputPayload,
    output: &ContentContractValidateOutputPayload,
) -> Result<(), DomainError> {
    upsert_runtime_blob(pool, &input.run_id, "content_contract_validation", output).await?;
    if let Some(draft) = input.draft.as_ref() {
        non_empty(&draft.page_draft_key, "page_draft_key")?;
        sqlx::query(
            r#"
            UPDATE site.page_drafts
            SET qa_blockers = $1,
                updated_at = now()
            WHERE page_draft_key = $2
            "#,
        )
        .bind(Json(json!({
            "content_contract_verdict": output.verdict,
            "blocking_reasons": output.blocking_reasons,
            "blockers": output.blockers,
        })))
        .bind(&draft.page_draft_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(())
}

pub async fn persist_draft_qa_output(
    pool: &PgPool,
    input: &DraftQaInputPayload,
    output: &DraftQaOutputPayload,
) -> Result<(), DomainError> {
    let Some(draft) = input.draft.as_ref() else {
        return Err(validation_failure("draft_qa persistence requires draft"));
    };
    non_empty(&draft.page_draft_key, "page_draft_key")?;
    sqlx::query(
        r#"
        UPDATE site.page_drafts
        SET qa_verdict  = $1,
            qa_blockers = $2,
            updated_at  = now()
        WHERE page_draft_key = $3
        "#,
    )
    .bind(&output.verdict)
    .bind(Json(json!({
        "blocking_reasons": output.blocking_reasons,
        "blockers": output.blockers,
        "required_next_action": output.required_next_action,
    })))
    .bind(&draft.page_draft_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let lifecycle_state = "review_required";
    set_page_lifecycle(pool, &draft.page_node_key, lifecycle_state, "draft_qa").await?;

    for reason in &output.blocking_reasons {
        let quality_failure_key = primitives::seo::seo_artifact_key(
            "seo_quality_failure",
            &[&draft.page_draft_key, reason, "draft_qa@1"],
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_quality_failures
                (quality_failure_key, page_draft_key, failure_type, severity, status, details)
            VALUES ($1, $2, $3, 'blocking', 'open', $4)
            ON CONFLICT (quality_failure_key) DO UPDATE
            SET status     = EXCLUDED.status,
                details    = EXCLUDED.details,
                updated_at = now()
            "#,
        )
        .bind(quality_failure_key)
        .bind(&draft.page_draft_key)
        .bind(reason)
        .bind(Json(json!({
            "verdict": output.verdict,
            "supported_claims": output.supported_claims,
            "unsupported_claims": output.unsupported_claims,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    emit_projection_events_for_run(
        pool,
        &input.run_id,
        vec![seo_qdrant_projection_event(
            "seo_draft_support_sections",
            "page_draft",
            &draft.page_draft_key,
            "",
            &draft.body_markdown,
            HashMap::from([
                ("page_node_key".to_string(), draft.page_node_key.clone()),
                ("page_brief_key".to_string(), draft.page_brief_key.clone()),
                ("qa_verdict".to_string(), output.verdict.clone()),
                (
                    "blocking_reasons".to_string(),
                    output.blocking_reasons.join("|"),
                ),
            ]),
        )],
    )
    .await?;

    Ok(())
}

pub async fn persist_rebuild_detect_output(
    pool: &PgPool,
    input: &RebuildDetectInputPayload,
    output: &RebuildDetectOutputPayload,
) -> Result<(), DomainError> {
    if output.verdict != "rebuild_required" {
        return Ok(());
    }

    let mut projection_events = Vec::new();
    let impact_by_page = resolve_rebuild_impacts(pool, &input.changed_truth_keys).await?;
    let output_impact_by_page = output
        .impacts
        .iter()
        .map(|impact| {
            let reason_package = serde_json::from_str::<Value>(&impact.reason_package_json)
                .unwrap_or_else(|_| {
                    json!({
                        "fallback_reason": impact.reason_package_json,
                    })
                });
            (
                impact.page_node_key.clone(),
                (impact.trigger_type.clone(), reason_package, impact.priority),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for page_node_key in &output.impacted_page_node_keys {
        if page_node_key.trim().is_empty() {
            continue;
        }
        let changed_truth_key = input
            .changed_truth_keys
            .first()
            .map(String::as_str)
            .unwrap_or("truth_change");
        let (trigger_type, reason_package, priority) = output_impact_by_page
            .get(page_node_key)
            .cloned()
            .unwrap_or((
                rebuild::TRUTH_CHANGE.to_string(),
                rebuild::canonical_reason_package(rebuild::TRUTH_CHANGE, &input.changed_truth_keys),
                rebuild::trigger_priority(rebuild::TRUTH_CHANGE),
            ));
        let reason_package = if let Some(impact_reason) = impact_by_page.get(page_node_key) {
            let mut merged = reason_package;
            if let Some(matched) = impact_reason.get("matched_dependencies").cloned() {
                merged["matched_dependencies"] = matched;
            }
            if let Some(seed) = impact_reason.get("seed_reason_package").cloned() {
                merged["seed_reason_package"] = seed;
            }
            merged
        } else {
            reason_package
        };
        if let Some(lifecycle_state) = rebuild::lifecycle_state_for_trigger(&trigger_type) {
            set_page_lifecycle(pool, page_node_key, lifecycle_state, &trigger_type).await?;
        }
        projection_events.push(seo_graph_projection_event("page_node", page_node_key, ""));
        let rebuild_request_key = primitives::seo::seo_artifact_key(
            "seo_rebuild",
            &[
                page_node_key,
                &trigger_type,
                changed_truth_key,
                "rebuild_detect@1",
            ],
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_rebuild_backlog
                (rebuild_request_key, page_node_key, trigger_type, priority, status, reason)
            VALUES ($1, $2, $3, $4, 'queued', $5)
            ON CONFLICT (rebuild_request_key) DO UPDATE
                SET status     = 'queued',
                trigger_type = EXCLUDED.trigger_type,
                priority   = EXCLUDED.priority,
                reason     = EXCLUDED.reason,
                updated_at = now()
            "#,
        )
        .bind(rebuild_request_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(reason_package.to_string())
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        let global_rebuild_plan_key = primitives::seo::seo_artifact_key(
            "global_rebuild_plan",
            &[page_node_key, &trigger_type, changed_truth_key],
        );
        sqlx::query(
            r#"
            INSERT INTO site.global_rebuild_plan
                (rebuild_plan_key, affected_page_node_key, trigger_type, priority, reason_payload, status)
            VALUES ($1, $2, $3, $4, $5, 'queued')
            ON CONFLICT (rebuild_plan_key) DO UPDATE
            SET affected_page_node_key = EXCLUDED.affected_page_node_key,
                trigger_type = EXCLUDED.trigger_type,
                priority = EXCLUDED.priority,
                reason_payload = EXCLUDED.reason_payload,
                status = 'queued',
                updated_at = now()
            "#,
        )
        .bind(global_rebuild_plan_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(Json(reason_package.clone()))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_applicant_profile;
    use seo_domain::applicability::{
        decide_support_resolution, ApplicabilityRuleCandidate, SupportResolutionDecision,
    };

    fn candidate() -> ApplicabilityRuleCandidate {
        ApplicabilityRuleCandidate {
            rule_instance_id: "rule:1".to_string(),
            has_profile_overrides: false,
            has_apply_profile: false,
            has_conditional_profile: false,
            has_exclude_profile: false,
            has_waive_exception: false,
            has_remove_exception: false,
            has_replace_exception: false,
            has_add_requirement_exception: false,
        }
    }

    #[test]
    fn supports_apply_without_profile_overrides() {
        let decision = decide_support_resolution(&candidate());
        assert_eq!(decision, SupportResolutionDecision::Include);
    }

    #[test]
    fn excludes_conditional_without_context() {
        let mut flags = candidate();
        flags.has_profile_overrides = true;
        flags.has_conditional_profile = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Unresolved("unresolved_conditional_applicability")
        );
    }

    #[test]
    fn excludes_replace_value_override_from_auto_support() {
        let mut flags = candidate();
        flags.has_replace_exception = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Unresolved("unresolved_exception_override")
        );
    }

    #[test]
    fn excludes_profile_mismatch() {
        let mut flags = candidate();
        flags.has_profile_overrides = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Excluded("profile_not_applicable")
        );
    }

    #[test]
    fn normalizes_known_profiles_only() {
        assert_eq!(normalize_applicant_profile("Minor").unwrap(), "minor");
        assert!(normalize_applicant_profile("base").is_err());
    }
}

async fn set_page_lifecycle(
    pool: &PgPool,
    page_node_key: &str,
    to_state: &str,
    reason: &str,
) -> Result<(), DomainError> {
    if page_node_key.trim().is_empty() {
        return Ok(());
    }
    let previous_state: Option<String> = sqlx::query_scalar(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
        RETURNING lifecycle_state
        "#,
    )
    .bind(to_state)
    .bind(page_node_key)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;

    if previous_state.is_some() {
        sqlx::query(
            r#"
            INSERT INTO site.page_lifecycle_events
                (page_node_key, from_state, to_state, reason, actor, event_payload)
            VALUES ($1, NULL, $2, $3, 'seo_system', $4)
            "#,
        )
        .bind(page_node_key)
        .bind(to_state)
        .bind(reason)
        .bind(Json(json!({ "source": "temporal_activity" })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    Ok(())
}
