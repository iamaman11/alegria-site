//! The single PostgreSQL owner of the verified Truth state transition.
//!
//! The caller first validates evidence, decides semantic outcomes in
//! `truth_domain`, and prepares optional external embeddings. This adapter
//! commits candidates, HITL, rule instances, projection Outbox, and the
//! typed step result atomically. No external Qdrant or Temporal call occurs
//! while the transaction is open.

use std::collections::BTreeMap;

use postgres_store::sqlx_hitl_adapter;
use primitives::errors::DomainError;
use seo_ports::knowledge::{VerifiedRuleEmbeddingBatch, VerifiedRuleUpsertBatch};
use sqlx::PgPool;
use truth_domain::review_decisions::TruthAdjudicationCandidateDecision;
use truth_domain::semantic_truth_write::{
    persisted_candidate_status, truth_candidate_write_fields, SemanticTruthWriteOutcome,
};

use super::proto_runtime_payload_store::RuntimeProtoPayload;
use super::{
    raw_crawl_adapter, sqlx_seo_adapter, sqlx_step_ledger_adapter, verified_rule_projection_adapter,
};

mod preflight;
use preflight::{
    serialize_truth_value, unstaged_global_verified_rule_ids, validate_prepared_truth_write,
};

/// Read-only idempotent replay before any external embedding call. The
/// transaction below repeats this check under the context lock.
pub async fn completed_verified_truth_write<O: RuntimeProtoPayload>(
    pool: &PgPool,
    run_id: &str,
    idempotency_key: &str,
) -> Result<Option<O>, DomainError> {
    sqlx_step_ledger_adapter::load_completed_step_result::<O>(
        pool,
        run_id,
        "verified_truth_write",
        idempotency_key,
    )
    .await
}

/// Prepared data, not a second Truth policy. An outcome is decided by the
/// domain and all source sections have already passed preflight validation.
pub struct PreparedTruthWrite<'a, I> {
    pub pool: &'a PgPool,
    pub run_id: &'a str,
    pub context_key: &'a str,
    pub step_input: &'a I,
    pub decisions: &'a [TruthAdjudicationCandidateDecision],
    pub semantic_slots: &'a [String],
    pub semantic_outcomes: BTreeMap<String, SemanticTruthWriteOutcome<'a>>,
    pub raw_by_section: &'a BTreeMap<i64, raw_crawl_adapter::RawSectionRecord>,
    pub projection_change: &'a VerifiedRuleUpsertBatch,
    pub projection_embeddings: Option<VerifiedRuleEmbeddingBatch>,
}

/// Own one COMMIT and its run-order fencing. The output constructor belongs
/// to the caller's typed Activity contract; the *actual* serialized output
/// is written to the step journal before committing.
pub async fn commit_verified_truth_write<'a, I, O>(
    prepared: PreparedTruthWrite<'a, I>,
    make_output: impl FnOnce(usize, usize, Vec<String>) -> O,
) -> Result<O, DomainError>
where
    I: RuntimeProtoPayload,
    O: RuntimeProtoPayload,
{
    let PreparedTruthWrite {
        pool,
        run_id,
        context_key,
        step_input,
        decisions,
        semantic_slots,
        semantic_outcomes,
        raw_by_section,
        projection_change,
        projection_embeddings,
    } = prepared;

    validate_prepared_truth_write(
        run_id,
        context_key,
        decisions,
        semantic_slots,
        &semantic_outcomes,
        raw_by_section,
        projection_change,
        projection_embeddings.as_ref(),
    )?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|error| DomainError::InfraUnavailable {
            message: format!("verified_truth_write begin transaction: {error}"),
        })?;
    // First serialize all writers to this context, then lock semantic
    // identities in a stable order. The journal replay is checked under lock.
    sqlx_seo_adapter::lock_truth_context_in_tx(&mut tx, context_key).await?;
    let replay: Option<O> = sqlx_step_ledger_adapter::load_completed_step_result_in_tx(
        &mut tx,
        run_id,
        "verified_truth_write",
        1,
        step_input,
    )
    .await?;
    if let Some(previous) = replay {
        return Ok(previous);
    }
    sqlx_seo_adapter::reject_stale_truth_run_in_tx(&mut tx, run_id, context_key).await?;
    sqlx_seo_adapter::lock_truth_rule_projections_in_tx(&mut tx, semantic_outcomes.keys()).await?;

    // Optional Voyage embedding is safe for a never-projected identity, but
    // not for a previously materialized global rule. Otherwise a new Truth
    // commit would leave its old Qdrant point live with no revisioned Outbox
    // replacement. The PG helper checks both active and tombstoned projection
    // history while these SAME per-rule locks are held. Scoped verification
    // may proceed because it is explicitly tombstoned below.
    let unstaged_global_rules = unstaged_global_verified_rule_ids(
        &semantic_outcomes,
        projection_change,
        projection_embeddings.as_ref(),
    );
    postgres_store::sqlx_outbox_adapter::lock_and_reject_projected_legacy_truth_write_in_tx(
        &mut tx,
        &unstaged_global_rules,
    )
    .await
    .map_err(|error| DomainError::InfraUnavailable {
        message: format!(
            "verified Truth global-rule update lacks atomic revisioned Qdrant intent: {error}"
        ),
    })?;

    let mut persisted_candidate_ids = BTreeMap::<String, String>::new();
    for (decision, semantic_slot) in decisions.iter().zip(semantic_slots) {
        // Validated above before acquiring any SQL locks.
        let raw_section = &raw_by_section[&decision.section_id];
        raw_crawl_adapter::ensure_source_in_tx(&mut tx, raw_section).await?;
        let fields = truth_candidate_write_fields(decision);
        let slot_outcome =
            semantic_outcomes
                .get(semantic_slot)
                .ok_or_else(|| DomainError::ValidationFailure {
                    message: format!("missing write verdict for semantic identity {semantic_slot}"),
                })?;
        let candidate_status = persisted_candidate_status(decision, slot_outcome);
        let persisted_candidate_id =
            sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication_in_tx(
                &mut tx,
                &sqlx_seo_adapter::VerifiedTruthCandidateWrite {
                    rule_candidate_id: decision.rule_candidate_id.clone(),
                    context_key: context_key.to_string(),
                    section_id: decision.section_id,
                    role: decision.role.clone(),
                    concept_canonical_key: decision.concept_canonical_key.clone(),
                    evidence_quote: fields.evidence_quote,
                    params: serialize_truth_value(&decision.params, "candidate params")?,
                    scope: serialize_truth_value(&decision.scope, "candidate scope")?,
                    severity: fields.severity,
                    is_numeric: fields.is_numeric,
                    is_incomplete: decision.completeness_class != "complete",
                    confidence: decision.confidence,
                    span_start: fields.span_start,
                    span_end: fields.span_end,
                    source_key: decision.source_key.clone(),
                    source_snapshot_hash: decision.source_snapshot_hash.clone(),
                    provider_key: decision.provider_key.clone(),
                    model_key: decision.model_key.clone(),
                    prompt_version: decision.prompt_version.clone(),
                    epistemic_status: candidate_status.to_string(),
                },
            )
            .await?;
        persisted_candidate_ids.insert(decision.rule_candidate_id.clone(), persisted_candidate_id);
    }

    // HITL reconciliation covers the entire context, not merely the newly
    // adjudicated slots, and is inside this same transaction.
    sqlx_hitl_adapter::reconcile_truth_candidate_reviews_in_tx(&mut tx, context_key, run_id)
        .await?;

    let mut changed_truth_keys = Vec::new();
    let mut demoted_rule_projection_ids = Vec::new();
    let mut suppressed_scoped_projection_ids = Vec::new();
    let mut verified_rule_count = 0;
    let mut demoted_rule_count = 0;
    for (rule_instance_id, outcome) in semantic_outcomes {
        let changed_key = format!("verified.rule_instance:{rule_instance_id}");
        match outcome {
            SemanticTruthWriteOutcome::Verified(decision) => {
                let role_type = decision.role.to_ascii_lowercase();
                sqlx_seo_adapter::upsert_verified_rule_instance_in_tx(
                    &mut tx,
                    &sqlx_seo_adapter::VerifiedRuleInstanceWrite {
                        rule_instance_id: rule_instance_id.clone(),
                        context_key: context_key.to_string(),
                        rule_type_key: role_type.clone(),
                        concept_key: decision.concept_canonical_key.clone(),
                        role_type: role_type.clone(),
                        params: serialize_truth_value(&decision.params, "verified params")?,
                        scope: serialize_truth_value(&decision.scope, "verified scope")?,
                        source_key: decision.source_key.clone(),
                        confidence: decision.confidence,
                        rule_candidate_id: persisted_candidate_ids
                            .get(&decision.rule_candidate_id)
                            .ok_or_else(|| DomainError::ValidationFailure {
                                message: format!(
                                    "verified candidate {} was not persisted",
                                    decision.rule_candidate_id
                                ),
                            })?
                            .clone(),
                        evidence_section_id: decision.section_id,
                        evidence_quote: decision.evidence_quote.clone(),
                        span_start: decision.span_start.max(0) as i32,
                        span_end: std::cmp::max(
                            decision.span_end as i32,
                            decision.span_start.max(0) as i32 + 1,
                        ),
                        source_snapshot_hash: decision.source_snapshot_hash.clone(),
                        verification_method: decision.verification_method.clone(),
                        adjudication_reason: decision.adjudication_reason.clone(),
                        publish_admissibility: decision.publish_admissibility.clone(),
                        freshness_class: decision.freshness_class.clone(),
                        completeness_class: decision.completeness_class.clone(),
                    },
                )
                .await?;
                verified_rule_count += 1;
                if !matches!(
                    truth_domain::truth_candidates::truth_scope_target(&decision.scope),
                    Some(truth_domain::truth_candidates::TruthScopeTarget::Global)
                ) {
                    suppressed_scoped_projection_ids.push(rule_instance_id.clone());
                }
            }
            SemanticTruthWriteOutcome::Disputed => {
                sqlx_seo_adapter::demote_verified_rule_instance_in_tx(
                    &mut tx,
                    &rule_instance_id,
                    "disputed",
                    "needs_hitl",
                    "truth_write_semantic_guard@1",
                    "mixed_or_conflicting_semantic_adjudication",
                )
                .await?;
                demoted_rule_count += 1;
                demoted_rule_projection_ids.push(rule_instance_id.clone());
            }
            SemanticTruthWriteOutcome::Rejected(decision) => {
                sqlx_seo_adapter::demote_verified_rule_instance_in_tx(
                    &mut tx,
                    &rule_instance_id,
                    "deprecated",
                    "not_admissible",
                    &decision.verification_method,
                    &decision.adjudication_reason,
                )
                .await?;
                demoted_rule_count += 1;
                demoted_rule_projection_ids.push(rule_instance_id.clone());
            }
        }
        changed_truth_keys.push(changed_key);
    }

    demoted_rule_projection_ids.extend(suppressed_scoped_projection_ids);
    demoted_rule_projection_ids.sort();
    demoted_rule_projection_ids.dedup();
    verified_rule_projection_adapter::stage_rule_point_deletes_in_tx(
        &mut tx,
        run_id,
        context_key,
        &demoted_rule_projection_ids,
    )
    .await?;
    if let Some(embeddings) = projection_embeddings {
        verified_rule_projection_adapter::stage_rule_upserts_in_tx(
            &mut tx,
            projection_change,
            embeddings,
        )
        .await?;
    }
    changed_truth_keys.sort();
    changed_truth_keys.dedup();
    let output = make_output(verified_rule_count, demoted_rule_count, changed_truth_keys);
    sqlx_step_ledger_adapter::complete_step_execution_with_output_in_tx(
        &mut tx,
        run_id,
        "verified_truth_write",
        1,
        step_input,
        &output,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|error| DomainError::InfraUnavailable {
            message: format!("verified_truth_write commit transaction: {error}"),
        })?;
    Ok(output)
}
