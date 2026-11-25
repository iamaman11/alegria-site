use std::collections::{BTreeMap, BTreeSet, HashMap};

use contracts::generated::alegria::sync::v1::{QdrantUpsertCommand, SeoGraphProjectionPayload};
use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, SeoScopePayload, SeoSiteBuildInputPayload,
    SeoVerifiedFactSupportState, SerpIngestInputPayload, SerpIngestOutputPayload,
    SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use primitives::qdrant_point_id::qdrant_point_id_v1;
use prost::Message;
use runtime_models::{
    ContentGap, GraphPlanningContext, GraphPlanningCoverageSignal, GraphPlanningTopicSignal,
    GraphPlanningTripleSignal, KeywordCluster, LinkRecommendation, PageNode,
};
use seo_domain::rebuild::{RebuildDetectRequest, RebuildDetectResult, RebuildVerdict};
use seo_domain::{applicability, identity, rebuild};
use seo_ports::SectionTemplate;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::{types::Json, PgPool, Row};
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, validation_failure,
    RuntimeProtoPayload,
};
use super::sqlx_outbox_adapter::{OutboxEnvelope, VERIFIED_RULE_UNCERTAIN_MARKER};
use super::sqlx_runtime_outbox_adapter::{outbox_emit_many, outbox_emit_many_in_tx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionBacklogCounts {
    pub pending_events: i64,
    pub failed_events: i64,
    /// Subset of failures fenced before an external verified-rule Qdrant RPC.
    /// These rows may become `done` on the worker's successful acknowledgement;
    /// no observer may automatically requeue or clear an uncertain row.
    pub uncertain_events: i64,
}

pub async fn projection_backlog_counts(
    pool: &PgPool,
    run_id: &str,
    target_system: &str,
) -> Result<ProjectionBacklogCounts, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT
          -- Processing leases are still outstanding, not materialized points.
          COUNT(*) FILTER (WHERE status IN ('pending', 'processing')) AS pending_events,
          COUNT(*) FILTER (WHERE status = 'failed') AS failed_events,
          COUNT(*) FILTER (
            WHERE status = 'failed'
              AND target_system = 'qdrant'
              AND aggregate_type = 'rule_instance'
              AND last_error = $3
          ) AS uncertain_events
        FROM system.sync_outbox
        WHERE run_id = $1
          AND target_system = $2
        "#,
    )
    .bind(run_id)
    .bind(target_system)
    .bind(VERIFIED_RULE_UNCERTAIN_MARKER)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(ProjectionBacklogCounts {
        pending_events: row.get("pending_events"),
        failed_events: row.get("failed_events"),
        uncertain_events: row.get("uncertain_events"),
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

pub async fn upsert_qdrant_rule_point_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

/// Serialize context-wide Truth/HITL writes before acquiring any per-rule
/// projection locks. The caller owns the transaction and its final commit.
pub async fn lock_truth_context_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    context_key: &str,
) -> Result<(), DomainError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{context_key}"))
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
    Ok(())
}

/// Acquire the same sorted, transaction-scoped rule keys used by the Qdrant
/// Outbox materializer. Truth holds these while mutating canonical SQL state.
/// Lock order for the complete write is context first, then sorted rule IDs.
pub async fn lock_truth_rule_projections_in_tx<I, S>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule_ids: I,
) -> Result<(), DomainError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let sorted = rule_ids
        .into_iter()
        .map(|id| id.as_ref().to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    postgres_store::sqlx_outbox_adapter::lock_and_ensure_verified_rules_writable_in_tx(tx, &sorted)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("verified Truth mutation blocked by Qdrant uncertainty fence: {err}"),
        })
}

/// A delayed older workflow must not overwrite a newer Truth commit for the
/// same context, even on its FIRST execution. This guard runs after the
/// context advisory lock and before any canonical mutations. The completed
/// step marker and its SQL state commit atomically in the Truth activity.
pub async fn reject_stale_truth_run_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: &str,
    context_key: &str,
) -> Result<(), DomainError> {
    let current_id = Uuid::parse_str(run_id)
        .map_err(|error| validation_failure(format!("invalid Truth run_id UUID: {error}")))?;
    let newer_committed: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM pipeline.execution_runs AS newer
             JOIN pipeline.step_executions AS done
               ON done.run_id = newer.run_id
              AND done.step_name = 'verified_truth_write'
              AND done.status = 'done'
             WHERE newer.context_key = this_run.context_key
               AND newer.run_id <> this_run.run_id
               -- UUID ordering cannot establish chronology when PostgreSQL
               -- timestamps tie. Intentionally sacrifice progress on a tie:
               -- the safety invariant forbids either ambiguous stale write.
               AND newer.created_at >= this_run.created_at
         )
         FROM pipeline.execution_runs AS this_run
         WHERE this_run.run_id = $1 AND this_run.context_key = $2",
    )
    .bind(current_id)
    .bind(context_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let Some(newer_committed) = newer_committed else {
        return Err(validation_failure(format!(
            "Truth run {run_id} does not belong to context {context_key}"
        )));
    };
    if newer_committed {
        return Err(DomainError::ConflictViolation {
            message: format!(
                "stale Truth run {run_id} cannot overwrite a newer completed Truth write for context {context_key}"
            ),
        });
    }
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
    pub scope: Value,
    pub severity: String,
    pub is_numeric: bool,
    pub is_incomplete: bool,
    pub confidence: f64,
    pub span_start: i32,
    pub span_end: i32,
    pub source_key: String,
    pub source_snapshot_hash: String,
    pub provider_key: String,
    pub model_key: String,
    pub prompt_version: String,
    pub epistemic_status: String,
}

pub async fn upsert_extracted_rule_candidate_from_adjudication(
    pool: &PgPool,
    candidate: &VerifiedTruthCandidateWrite,
) -> Result<String, DomainError> {
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let id =
        upsert_extracted_rule_candidate_from_adjudication_in_tx(&mut tx, candidate).await?;
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(id)
}

/// The evidence-key unique constraint can collide with an extractor-specific
/// candidate ID. A PostgreSQL SAVEPOINT restores the outer transaction before
/// reconciling that canonical ID, so the whole Truth write can commit once.
pub async fn upsert_extracted_rule_candidate_from_adjudication_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    candidate: &VerifiedTruthCandidateWrite,
) -> Result<String, DomainError> {
    // Keep primary-key replay idempotent. Reconcile a second extractor ID
    // for the same unique source evidence after detecting that exact conflict.
    sqlx::query("SAVEPOINT truth_candidate_insert")
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
    let insert = sqlx::query(
        "INSERT INTO extracted.rule_candidates (
             rule_candidate_id, context_key, raw_section_id, role, concept_canonical_key,
             raw_mention, params, scope, severity, applies_to_profiles, exceptions_raw,
             conditions_raw, alternatives, modality_raw, derivation_type, is_numeric, is_range,
             is_incomplete, confidence, evidence_section_id, evidence_quote, span_start, span_end,
             source_key, source_snapshot_hash, llm_provider, llm_model, prompt_version,
             epistemic_status, uncertainty_flags
         )
         VALUES (
             $1, $2, $3, $4, $5, $6, $7, $19, $8, '[]'::jsonb,
             '', '', '[]'::jsonb, '', 'direct', $9, false, $10, $11, $12,
             $13, $14, $15, $16, $17, $20, $21, $22, $18, '[]'::jsonb
         )
         ON CONFLICT (rule_candidate_id) DO UPDATE
         SET role = EXCLUDED.role,
             concept_canonical_key = EXCLUDED.concept_canonical_key,
             raw_mention = EXCLUDED.raw_mention,
             params = EXCLUDED.params,
             scope = EXCLUDED.scope,
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
    .bind(Json::<Value>(candidate.scope.clone()))
    .bind(&candidate.provider_key)
    .bind(&candidate.model_key)
    .bind(&candidate.prompt_version)
    .execute(&mut **tx)
    .await;

    match insert {
        Ok(_) => {
            sqlx::query("RELEASE SAVEPOINT truth_candidate_insert")
                .execute(&mut **tx)
                .await
                .map_err(classify_sqlx)?;
            Ok(candidate.rule_candidate_id.clone())
        }
        Err(sqlx::Error::Database(ref db))
            if db.is_unique_violation()
                && db.constraint()
                    == Some("rule_candidates_raw_section_id_role_concept_canonical_key_e_key") =>
        {
            sqlx::query("ROLLBACK TO SAVEPOINT truth_candidate_insert")
                .execute(&mut **tx)
                .await
                .map_err(classify_sqlx)?;
            // One evidence key can have multiple extractor proposals but only
            // ONE durable SQL identity. Keep existing HITL and verified FKs.
            // A conflicting claim, source, context or rejected review fails closed.
            let canonical_id: Option<String> = sqlx::query_scalar(
                r#"
                UPDATE extracted.rule_candidates AS c
                SET llm_provider = CASE
                        WHEN $16 = 'chat_manual' OR c.llm_provider <> 'chat_manual'
                        THEN $16 ELSE c.llm_provider END,
                    llm_model = CASE
                        WHEN $16 = 'chat_manual' OR c.llm_provider <> 'chat_manual'
                        THEN $17 ELSE c.llm_model END,
                    prompt_version = CASE
                        WHEN $16 = 'chat_manual' OR c.llm_provider <> 'chat_manual'
                        THEN $18 ELSE c.prompt_version END,
                    confidence = CASE
                        WHEN $16 = 'chat_manual' OR c.llm_provider <> 'chat_manual'
                        THEN $15 ELSE c.confidence END,
                    -- A disagreement in evidence metadata is not a second
                    -- factual claim. Retain one canonical FK but fail closed:
                    -- never silently promote a disputed extraction to VERIFIED.
                    severity = CASE
                        WHEN c.severity = $12 THEN c.severity
                        ELSE 'unknown'
                    END,
                    is_incomplete = c.is_incomplete OR $14,
                    epistemic_status = CASE
                        WHEN c.epistemic_status = 'rejected' OR $19 = 'rejected'
                            THEN 'rejected'
                        WHEN c.epistemic_status = 'needs_hitl' OR $19 = 'needs_hitl'
                            OR c.severity <> $12 OR c.is_incomplete <> $14
                            THEN 'needs_hitl'
                        WHEN c.epistemic_status = $19 THEN c.epistemic_status
                        ELSE 'needs_hitl'
                    END,
                    updated_at = now()
                WHERE c.raw_section_id = $1 AND c.role = $2
                  AND c.concept_canonical_key = $3
                  AND c.evidence_quote = $4
                  AND c.span_start = $5 AND c.span_end = $6
                  AND c.context_key = $7
                  AND c.params = $8::jsonb AND c.scope = $9::jsonb
                  AND c.source_key = $10 AND c.source_snapshot_hash = $11
                  -- Core fact, source and snapshot must match exactly;
                  -- advisory severity/completeness can differ across extractors.
                  AND c.is_numeric = $13
                  AND NOT EXISTS (
                      SELECT 1 FROM site.seo_hitl_tasks AS t
                      WHERE t.blocking_execution_key = c.rule_candidate_id
                        AND t.scope_signature = c.context_key
                        AND t.task_type = 'truth_candidate_review'
                        AND t.queue_state = 'in_review'
                        AND t.decision_payload->'review'->>'decision' = 'reject'
                  )
                RETURNING c.rule_candidate_id
                "#,
            )
            .bind(candidate.section_id)
            .bind(&candidate.role)
            .bind(&candidate.concept_canonical_key)
            .bind(&candidate.evidence_quote)
            .bind(candidate.span_start)
            .bind(candidate.span_end)
            .bind(&candidate.context_key)
            .bind(Json::<Value>(candidate.params.clone()))
            .bind(Json::<Value>(candidate.scope.clone()))
            .bind(&candidate.source_key)
            .bind(&candidate.source_snapshot_hash)
            .bind(&candidate.severity)
            .bind(candidate.is_numeric)
            .bind(candidate.is_incomplete)
            .bind(candidate.confidence)
            .bind(&candidate.provider_key)
            .bind(&candidate.model_key)
            .bind(&candidate.prompt_version)
            .bind(&candidate.epistemic_status)
            .fetch_optional(&mut **tx)
            .await
            .map_err(classify_sqlx)?;
            sqlx::query("RELEASE SAVEPOINT truth_candidate_insert")
                .execute(&mut **tx)
                .await
                .map_err(classify_sqlx)?;
            canonical_id.ok_or_else(|| DomainError::ValidationFailure {
                message: "duplicate source evidence has incompatible claim, context, snapshot, or rejected human review".to_string(),
            })
        }
        Err(err) => {
            sqlx::query("ROLLBACK TO SAVEPOINT truth_candidate_insert")
                .execute(&mut **tx)
                .await
                .map_err(classify_sqlx)?;
            sqlx::query("RELEASE SAVEPOINT truth_candidate_insert")
                .execute(&mut **tx)
                .await
                .map_err(classify_sqlx)?;
            Err(classify_sqlx(err))
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedRuleInstanceWrite {
    pub scope: Value,
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

/// One parser is shared by extraction validation and final SQL persistence.
/// Unsupported scopes are never accepted as global facts.
fn verified_scope_target(
    scope: &Value,
) -> Option<truth_domain::truth_candidates::TruthScopeTarget> {
    let typed = serde_json::from_value::<truth_domain::truth_candidates::TruthParamValue>(
        scope.clone(),
    )
    .ok()?;
    truth_domain::truth_candidates::truth_scope_target(&typed)
}

fn validate_verified_rule_instance_write(
    rule: &VerifiedRuleInstanceWrite,
) -> Result<truth_domain::truth_candidates::TruthScopeTarget, DomainError> {
    if rule
        .params
        .get("conditions_key")
        .is_some_and(|condition| match condition {
            Value::Null => false,
            Value::String(key) => !key.trim().is_empty(),
            _ => true,
        })
    {
        return Err(validation_failure(
            "unresolved verified conditions_key requires explicit condition evaluation",
        ));
    }
    verified_scope_target(&rule.scope).ok_or_else(|| {
        validation_failure("verified rule scope is not a supported exact applicant_profile")
    })
}

pub async fn upsert_verified_rule_instance(
    pool: &PgPool,
    rule: &VerifiedRuleInstanceWrite,
) -> Result<(), DomainError> {
    // Invalid scope never opens a connection, including through this pool API.
    validate_verified_rule_instance_write(rule)?;
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    // The standalone legacy writer has no Outbox intent or staged registry
    // mutation in its transaction. It may seed an as-yet unprojected identity,
    // but must never mutate one that has ever been projected (including a
    // previously tombstoned point). Canonical Truth writes use the in-tx
    // writer and stage the revisioned Qdrant intent before commit instead.
    postgres_store::sqlx_outbox_adapter::lock_and_reject_projected_legacy_truth_write_in_tx(
        &mut tx,
        std::slice::from_ref(&rule.rule_instance_id),
    )
    .await
    .map_err(|error| DomainError::InfraUnavailable {
        message: format!("legacy verified Truth upsert without Qdrant intent rejected: {error}"),
    })?;
    upsert_verified_rule_instance_in_tx(&mut tx, rule).await?;
    tx.commit().await.map_err(classify_sqlx)
}

pub async fn upsert_verified_rule_instance_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule: &VerifiedRuleInstanceWrite,
) -> Result<(), DomainError> {
    // Validate the precise scope BEFORE any SQL mutation.
    let target = validate_verified_rule_instance_write(rule)?;
    // This public low-level writer can also be called without the enclosing
    // Truth workflow. Protect it independently from outstanding provider
    // effects; the canonical caller's earlier lock is reentrant in this tx.
    postgres_store::sqlx_outbox_adapter::lock_and_ensure_verified_rules_writable_in_tx(
        tx,
        std::slice::from_ref(&rule.rule_instance_id),
    )
    .await
    .map_err(|err| DomainError::InfraUnavailable {
        message: format!("verified Truth upsert blocked by Qdrant uncertainty fence: {err}"),
    })?;
    if let truth_domain::truth_candidates::TruthScopeTarget::ApplicantProfile(profile) = &target {
        let registered: Option<String> = sqlx::query_scalar(
            "SELECT profile_key FROM kb.applicant_profiles
             WHERE profile_key = $1 AND status = 'active' FOR SHARE",
        )
        .bind(profile)
        .fetch_optional(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
        if registered.is_none() {
            return Err(validation_failure(format!(
                "unregistered or inactive verified applicant_profile: {profile}"
            )));
        }
    }
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
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;

    // Never erase curated profile restrictions while refreshing a verified
    // rule. A global rule may already have explicit human-defined overrides.
    if let truth_domain::truth_candidates::TruthScopeTarget::ApplicantProfile(profile) = target {
        let has_conflicting_binding: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM verified.rule_instance_profiles
                 WHERE rule_instance_id = $1
                   AND (profile_key <> $2 OR applicability <> 'applies'
                        OR condition_json IS NOT NULL)
             )",
        )
        .bind(&rule.rule_instance_id)
        .bind(&profile)
        .fetch_one(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
        if has_conflicting_binding {
            return Err(validation_failure(
                "existing curated profile binding conflicts with verified scope",
            ));
        }
        sqlx::query(
            "INSERT INTO verified.rule_instance_profiles
             (rule_instance_id, profile_key, applicability, condition_json)
             VALUES ($1, $2, 'applies', NULL)
             ON CONFLICT (rule_instance_id, profile_key) DO NOTHING",
        )
        .bind(&rule.rule_instance_id)
        .bind(profile)
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
    }
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
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    // A standalone status change cannot atomically issue the Qdrant
    // tombstone. Protect the entire lifetime of an ever-projected rule, not
    // merely identities with a currently active registry row.
    postgres_store::sqlx_outbox_adapter::lock_and_reject_projected_legacy_truth_write_in_tx(
        &mut tx,
        &[rule_instance_id.to_owned()],
    )
    .await
    .map_err(|error| DomainError::InfraUnavailable {
        message: format!("legacy verified Truth demotion without Qdrant intent rejected: {error}"),
    })?;
    demote_verified_rule_instance_in_tx(
        &mut tx,
        rule_instance_id,
        status,
        publish_admissibility,
        verification_method,
        adjudication_reason,
    )
    .await?;
    tx.commit().await.map_err(classify_sqlx)
}

pub async fn demote_verified_rule_instance_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule_instance_id: &str,
    status: &str,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> Result<(), DomainError> {
    postgres_store::sqlx_outbox_adapter::lock_and_ensure_verified_rules_writable_in_tx(
        tx,
        &[rule_instance_id.to_string()],
    )
    .await
    .map_err(|err| DomainError::InfraUnavailable {
        message: format!("verified Truth demotion blocked by Qdrant uncertainty fence: {err}"),
    })?;
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
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

#[cfg(test)]
mod verified_scope_persistence_tests {
    use super::*;

    #[test]
    fn final_sqlx_writer_accepts_only_exact_profile_or_global_scope() {
        assert!(matches!(
            verified_scope_target(&serde_json::json!({})),
            Some(truth_domain::truth_candidates::TruthScopeTarget::Global)
        ));
        assert!(matches!(
            verified_scope_target(&serde_json::json!({"applicant_profile":"student"})),
            Some(truth_domain::truth_candidates::TruthScopeTarget::ApplicantProfile(p))
                if p == "student"
        ));
        for scope in [
            serde_json::json!({"country":"ES"}),
            serde_json::json!({"applicant_profile":"Student"}),
            serde_json::json!({"applicant_profile":"student","age":17}),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            assert!(verified_scope_target(&scope).is_none());
        }
    }

    #[tokio::test]
    async fn unsupported_scoped_verified_writer_fails_before_attempting_sql() {
        // An intentionally unreachable database proves rejection belongs
        // to the SQLx write boundary rather than the orchestration layer.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://postgres:unused@127.0.0.1:65534/unused")
            .expect("lazy pool");
        let rule = VerifiedRuleInstanceWrite {
            scope: serde_json::json!({"applicant_type": "student"}),
            rule_instance_id: "rule:scoped".to_string(),
            context_key: "ctx".to_string(),
            rule_type_key: "fee_item".to_string(),
            concept_key: "fee".to_string(),
            role_type: "fee_item".to_string(),
            params: serde_json::json!({"amount": 35}),
            source_key: "https://example.gov/fees".to_string(),
            confidence: 0.95,
            rule_candidate_id: "candidate:1".to_string(),
            evidence_section_id: 7,
            evidence_quote: "Fee: 35 EUR".to_string(),
            span_start: 0,
            span_end: 11,
            source_snapshot_hash: "source-sha".to_string(),
            verification_method: "test".to_string(),
            adjudication_reason: "test".to_string(),
            publish_admissibility: "admissible".to_string(),
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
        };
        let result = upsert_verified_rule_instance(&pool, &rule).await;
        assert!(matches!(result, Err(DomainError::ValidationFailure { .. })));
    }
}
