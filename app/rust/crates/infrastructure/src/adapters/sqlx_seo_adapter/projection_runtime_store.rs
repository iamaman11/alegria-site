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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionBacklogCounts {
    pub pending_events: i64,
    pub failed_events: i64,
}

pub async fn projection_backlog_counts(
    pool: &PgPool,
    run_id: &str,
    target_system: &str,
) -> Result<ProjectionBacklogCounts, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE status = 'pending') AS pending_events,
          COUNT(*) FILTER (WHERE status = 'failed') AS failed_events
        FROM system.sync_outbox
        WHERE run_id = $1
          AND target_system = $2
        "#,
    )
    .bind(run_id)
    .bind(target_system)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(ProjectionBacklogCounts {
        pending_events: row.get("pending_events"),
        failed_events: row.get("failed_events"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeoPreflightStoreCounts {
    pub context_count: i64,
    pub page_type_count: i64,
    pub page_node_count: i64,
    pub navigation_item_count: i64,
    pub verified_rule_count: i64,
    pub pending_rule_count: i64,
    pub qdrant_point_count: i64,
}

pub async fn load_seo_preflight_store_counts(
    pool: &PgPool,
    context_key: &str,
) -> Result<SeoPreflightStoreCounts, DomainError> {
    let context_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM kb.visa_contexts WHERE context_key = $1 AND status = 'active'",
    )
    .bind(context_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let page_type_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.registry_page_types WHERE status = 'active'",
    )
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let page_node_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_nodes")
        .fetch_one(pool)
        .await
        .map_err(classify_sqlx)?;
    let navigation_item_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM site.navigation_items")
            .fetch_one(pool)
            .await
            .map_err(classify_sqlx)?;
    let verified_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(context_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let pending_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'pending'",
    )
    .bind(context_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let qdrant_point_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM kb.qdrant_points")
            .fetch_one(pool)
            .await
            .map_err(classify_sqlx)?;

    Ok(SeoPreflightStoreCounts {
        context_count,
        page_type_count,
        page_node_count,
        navigation_item_count,
        verified_rule_count,
        pending_rule_count,
        qdrant_point_count,
    })
}

pub async fn delete_qdrant_rule_points(
    pool: &PgPool,
    collection_name: &str,
    rule_instance_ids: &[String],
) -> Result<(), DomainError> {
    sqlx::query(
        "DELETE FROM kb.qdrant_points
         WHERE collection_name = $1
           AND entity_type = 'rule_instance'
           AND entity_key = ANY($2)",
    )
    .bind(collection_name)
    .bind(rule_instance_ids)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn upsert_qdrant_rule_point(
    pool: &PgPool,
    point_id: &str,
    rule_instance_id: &str,
    collection_name: &str,
    embedding_model: &str,
    embedding_version: &str,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO kb.qdrant_points
             (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
         VALUES ($1, 'rule_instance', $2, $3, $4, $5)
         ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
         SET point_id = EXCLUDED.point_id,
             embedding_model = EXCLUDED.embedding_model,
             embedding_version = EXCLUDED.embedding_version,
             updated_at = now()",
    )
    .bind(point_id)
    .bind(rule_instance_id)
    .bind(collection_name)
    .bind(embedding_model)
    .bind(embedding_version)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct VerifiedTruthCandidateWrite {
    pub rule_candidate_id: String,
    pub context_key: String,
    pub section_id: i64,
    pub role: String,
    pub concept_canonical_key: String,
    pub evidence_quote: String,
    pub params: Value,
    pub severity: String,
    pub is_numeric: bool,
    pub is_incomplete: bool,
    pub confidence: f64,
    pub span_start: i32,
    pub span_end: i32,
    pub source_key: String,
    pub source_snapshot_hash: String,
    pub epistemic_status: String,
}

pub async fn upsert_extracted_rule_candidate_from_adjudication(
    pool: &PgPool,
    candidate: &VerifiedTruthCandidateWrite,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO extracted.rule_candidates (
             rule_candidate_id, context_key, raw_section_id, role, concept_canonical_key,
             raw_mention, params, scope, severity, applies_to_profiles, exceptions_raw,
             conditions_raw, alternatives, modality_raw, derivation_type, is_numeric, is_range,
             is_incomplete, confidence, evidence_section_id, evidence_quote, span_start, span_end,
             source_key, source_snapshot_hash, llm_provider, llm_model, prompt_version,
             epistemic_status, uncertainty_flags
         )
         VALUES (
             $1, $2, $3, $4, $5, $6, $7, '{}'::jsonb, $8, '[]'::jsonb,
             '', '', '[]'::jsonb, '', 'direct', $9, false, $10, $11, $12,
             $13, $14, $15, $16, $17, 'deterministic', 'cutover-runtime', 'cutover@1', $18, '[]'::jsonb
         )
         ON CONFLICT (rule_candidate_id) DO UPDATE
         SET role = EXCLUDED.role,
             concept_canonical_key = EXCLUDED.concept_canonical_key,
             raw_mention = EXCLUDED.raw_mention,
             params = EXCLUDED.params,
             severity = EXCLUDED.severity,
             is_numeric = EXCLUDED.is_numeric,
             is_incomplete = EXCLUDED.is_incomplete,
             confidence = EXCLUDED.confidence,
             evidence_section_id = EXCLUDED.evidence_section_id,
             evidence_quote = EXCLUDED.evidence_quote,
             span_start = EXCLUDED.span_start,
             span_end = EXCLUDED.span_end,
             source_key = EXCLUDED.source_key,
             source_snapshot_hash = EXCLUDED.source_snapshot_hash,
             llm_provider = EXCLUDED.llm_provider,
             llm_model = EXCLUDED.llm_model,
             prompt_version = EXCLUDED.prompt_version,
             epistemic_status = EXCLUDED.epistemic_status,
             updated_at = now()",
    )
    .bind(&candidate.rule_candidate_id)
    .bind(&candidate.context_key)
    .bind(candidate.section_id)
    .bind(&candidate.role)
    .bind(&candidate.concept_canonical_key)
    .bind(&candidate.evidence_quote)
    .bind(Json::<Value>(candidate.params.clone()))
    .bind(&candidate.severity)
    .bind(candidate.is_numeric)
    .bind(candidate.is_incomplete)
    .bind(candidate.confidence)
    .bind(candidate.section_id)
    .bind(&candidate.evidence_quote)
    .bind(candidate.span_start)
    .bind(candidate.span_end)
    .bind(&candidate.source_key)
    .bind(&candidate.source_snapshot_hash)
    .bind(&candidate.epistemic_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct VerifiedRuleInstanceWrite {
    pub rule_instance_id: String,
    pub context_key: String,
    pub rule_type_key: String,
    pub concept_key: String,
    pub role_type: String,
    pub params: Value,
    pub source_key: String,
    pub confidence: f64,
    pub rule_candidate_id: String,
    pub evidence_section_id: i64,
    pub evidence_quote: String,
    pub span_start: i32,
    pub span_end: i32,
    pub source_snapshot_hash: String,
    pub verification_method: String,
    pub adjudication_reason: String,
    pub publish_admissibility: String,
    pub freshness_class: String,
    pub completeness_class: String,
}

pub async fn upsert_verified_rule_instance(
    pool: &PgPool,
    rule: &VerifiedRuleInstanceWrite,
) -> Result<(), DomainError> {
    sqlx::query(
        "INSERT INTO verified.rule_instances (
             rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status,
             source_key, confidence, effective_from, rule_candidate_id, evidence_section_id,
             evidence_quote, span_start, span_end, source_snapshot_hash, verification_method,
             adjudication_reason, publish_admissibility, freshness_class, completeness_class,
             registry_version, prompt_version, model_version, pipeline_version
         )
         VALUES (
             $1, $2, $3, $4, $5, $6, 'verified', $7, $8, current_date,
             $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23
         )
         ON CONFLICT (rule_instance_id) DO UPDATE
         SET rule_type_key = EXCLUDED.rule_type_key,
             concept_key = EXCLUDED.concept_key,
             role_type = EXCLUDED.role_type,
             params = EXCLUDED.params,
             status = EXCLUDED.status,
             source_key = EXCLUDED.source_key,
             confidence = EXCLUDED.confidence,
             effective_from = EXCLUDED.effective_from,
             rule_candidate_id = EXCLUDED.rule_candidate_id,
             evidence_section_id = EXCLUDED.evidence_section_id,
             evidence_quote = EXCLUDED.evidence_quote,
             span_start = EXCLUDED.span_start,
             span_end = EXCLUDED.span_end,
             source_snapshot_hash = EXCLUDED.source_snapshot_hash,
             verification_method = EXCLUDED.verification_method,
             adjudication_reason = EXCLUDED.adjudication_reason,
             publish_admissibility = EXCLUDED.publish_admissibility,
             freshness_class = EXCLUDED.freshness_class,
             completeness_class = EXCLUDED.completeness_class,
             registry_version = EXCLUDED.registry_version,
             prompt_version = EXCLUDED.prompt_version,
             model_version = EXCLUDED.model_version,
             pipeline_version = EXCLUDED.pipeline_version,
             updated_at = now()",
    )
    .bind(&rule.rule_instance_id)
    .bind(&rule.context_key)
    .bind(&rule.rule_type_key)
    .bind(&rule.concept_key)
    .bind(&rule.role_type)
    .bind(Json::<Value>(rule.params.clone()))
    .bind(&rule.source_key)
    .bind(rule.confidence)
    .bind(&rule.rule_candidate_id)
    .bind(rule.evidence_section_id)
    .bind(&rule.evidence_quote)
    .bind(rule.span_start)
    .bind(rule.span_end)
    .bind(&rule.source_snapshot_hash)
    .bind(&rule.verification_method)
    .bind(&rule.adjudication_reason)
    .bind(&rule.publish_admissibility)
    .bind(&rule.freshness_class)
    .bind(&rule.completeness_class)
    .bind("registry@1")
    .bind("cutover@1")
    .bind("deterministic")
    .bind("truth_adjudication_runtime@1")
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn demote_verified_rule_instance(
    pool: &PgPool,
    rule_instance_id: &str,
    status: &str,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> Result<(), DomainError> {
    sqlx::query(
        "UPDATE verified.rule_instances
         SET status = $2,
             publish_admissibility = $3,
             verification_method = $4,
             adjudication_reason = $5,
             updated_at = now()
         WHERE rule_instance_id = $1",
    )
    .bind(rule_instance_id)
    .bind(status)
    .bind(publish_admissibility)
    .bind(verification_method)
    .bind(adjudication_reason)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

