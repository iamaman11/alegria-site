//! Pure, side-effect-free validation of a prepared PostgreSQL Truth commit.
//!
//! Application/domain code owns which outcomes should be projected. This
//! boundary checks typed identity, durable SQL cardinality, and projection
//! references before the atomic transaction or its locks are opened.

use std::collections::BTreeSet;
use truth_domain::semantic_truth_write::validate_verified_truth_write_candidate;

use super::{
    raw_crawl_adapter, BTreeMap, DomainError, SemanticTruthWriteOutcome,
    TruthAdjudicationCandidateDecision, VerifiedRuleEmbeddingBatch, VerifiedRuleUpsertBatch,
};

/// Reject a malformed Activity payload *before* opening PostgreSQL. The
/// upstream adjudication normally guarantees unique candidate IDs and matching
/// semantic slots, but this SQL boundary must also be safe when called
/// directly or after a future workflow composition change.
pub(super) fn validate_prepared_truth_write(
    run_id: &str,
    context_key: &str,
    decisions: &[TruthAdjudicationCandidateDecision],
    semantic_slots: &[String],
    semantic_outcomes: &BTreeMap<String, SemanticTruthWriteOutcome<'_>>,
    raw_by_section: &BTreeMap<i64, raw_crawl_adapter::RawSectionRecord>,
    projection_change: &VerifiedRuleUpsertBatch,
    projection_embeddings: Option<&VerifiedRuleEmbeddingBatch>,
) -> Result<(), DomainError> {
    let invalid = |detail: &str| DomainError::ContractViolation {
        message: format!("verified_truth_write contract: {detail}"),
    };
    if decisions.len() != semantic_slots.len() {
        return Err(invalid("semantic slot count differs from candidates"));
    }
    if run_id.trim().is_empty() || context_key.trim().is_empty() {
        return Err(invalid("run and context identities must not be empty"));
    }
    let mut candidate_ids = BTreeSet::new();
    let mut required_slots = BTreeSet::new();
    for (decision, slot) in decisions.iter().zip(semantic_slots) {
        if decision.rule_candidate_id.trim().is_empty() || slot.trim().is_empty() {
            return Err(invalid(
                "candidate and semantic slot identities must not be empty",
            ));
        }
        validate_verified_truth_write_candidate(decision).map_err(|error| {
            DomainError::ValidationFailure {
                message: error.to_string(),
            }
        })?;
        // `extracted.rule_candidates.rule_candidate_id` is a PRIMARY KEY. A
        // duplicate would silently update one row with a different claim and
        // corrupt the verified-rule FK association within this transaction.
        if !candidate_ids.insert(decision.rule_candidate_id.as_str()) {
            return Err(invalid("duplicate extractor candidate identity"));
        }
        if !raw_by_section.contains_key(&decision.section_id) {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "verified truth write missing raw section {} for candidate/source registration",
                    decision.section_id
                ),
            });
        }
        required_slots.insert(slot.as_str());
    }
    if required_slots != semantic_outcomes.keys().map(String::as_str).collect() {
        return Err(invalid("semantic outcomes do not match candidate slots"));
    }
    for (slot, outcome) in semantic_outcomes {
        if let SemanticTruthWriteOutcome::Verified(selected)
        | SemanticTruthWriteOutcome::Rejected(selected) = outcome
        {
            if !decisions
                .iter()
                .zip(semantic_slots)
                .any(|(decision, decision_slot)| decision_slot == slot && decision == *selected)
            {
                return Err(invalid(
                    "semantic outcome references a different candidate decision",
                ));
            }
        }
    }
    // The application owns which verified global rules enter the projection;
    // SQL only checks that the submitted Outbox identities belong to verified
    // semantic outcomes. Do not repeat the business projection-selection rule.
    if projection_change.run_id != run_id || projection_change.context_key != context_key {
        return Err(invalid("projection run/context does not match Truth write"));
    }
    let mut projected_ids = BTreeSet::new();
    for candidate in &projection_change.upserts {
        if !projected_ids.insert(candidate.rule_instance_id.as_str())
            || !matches!(
                semantic_outcomes.get(&candidate.rule_instance_id),
                Some(SemanticTruthWriteOutcome::Verified(_))
            )
        {
            return Err(invalid(
                "Qdrant projection references a non-verified semantic identity",
            ));
        }
    }
    if let Some(embeddings) = projection_embeddings {
        if embeddings.model.trim().is_empty()
            || embeddings.vectors.len() != projection_change.upserts.len()
            || embeddings
                .vectors
                .iter()
                .any(|vector| vector.len() != 1024 || vector.iter().any(|value| !value.is_finite()))
        {
            return Err(invalid("Qdrant embedding shape does not match projection"));
        }
    }
    Ok(())
}

pub(super) fn serialize_truth_value(
    value: &truth_domain::truth_candidates::TruthParamValue,
    label: &str,
) -> Result<serde_json::Value, DomainError> {
    serde_json::to_value(value).map_err(|error| DomainError::ValidationFailure {
        message: format!("verified_truth_write could not encode {label}: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use seo_ports::knowledge::VerifiedRuleProjectionCandidate;

    fn decision(id: &str) -> TruthAdjudicationCandidateDecision {
        serde_json::from_value(serde_json::json!({
            "section_id": 42,
            "page_id": 7,
            "rule_candidate_id": id,
            "role": "FEE_ITEM",
            "concept_canonical_key": "tuition_fee",
            "params": {},
            "scope": {},
            "source_key": "official",
            "source_tier": "primary",
            "confidence": 0.99,
            "freshness_class": "fresh",
            "completeness_class": "complete",
            "evidence_quote": "fee schedule",
            "span_start": 0,
            "span_end": 12,
            "source_snapshot_hash": "hash",
            "decision": "verified",
            "publish_admissibility": "admissible",
            "verification_method": "source_check",
            "adjudication_reason": "source_verified"
        }))
        .expect("valid candidate fixture")
    }

    fn raw_sections() -> BTreeMap<i64, raw_crawl_adapter::RawSectionRecord> {
        BTreeMap::from([(
            42,
            raw_crawl_adapter::RawSectionRecord {
                id: 42,
                page_id: 7,
                source_url: "https://official.example.test/fees".into(),
                source_domain: "official.example.test".into(),
                source_dtype: "html".into(),
                heading_path: "fees".into(),
                section_type: "fees".into(),
                content_md: "fee schedule".into(),
                content_hash: "hash".into(),
            },
        )])
    }

    fn projection() -> VerifiedRuleUpsertBatch {
        VerifiedRuleUpsertBatch {
            run_id: "run".into(),
            context_key: "context".into(),
            upserts: vec![VerifiedRuleProjectionCandidate {
                rule_instance_id: "slot".into(),
                role_type: "fee_item".into(),
                concept_key: "tuition_fee".into(),
                source_key: "official".into(),
                evidence_quote: "fee schedule".into(),
            }],
        }
    }

    #[test]
    fn complete_verified_truth_write_matches_its_projection_before_sql() {
        let decisions = vec![decision("candidate")];
        let slots = vec!["slot".into()];
        // A composed step may reconstruct a typed decision rather than retain
        // the exact input allocation; SQL compares its immutable value.
        let selected = decisions[0].clone();
        let outcomes = BTreeMap::from([(
            "slot".into(),
            SemanticTruthWriteOutcome::Verified(&selected),
        )]);
        let sections = raw_sections();
        let change = projection();
        assert!(validate_prepared_truth_write(
            "run", "context", &decisions, &slots, &outcomes, &sections, &change, None,
        )
        .is_ok());
    }

    #[test]
    fn duplicate_candidate_identity_cannot_overwrite_a_different_claim() {
        let decisions = vec![decision("reused"), decision("reused")];
        let slots = vec!["slot".into(), "different-slot".into()];
        let outcomes = BTreeMap::from([(
            "slot".into(),
            SemanticTruthWriteOutcome::Verified(&decisions[0]),
        )]);
        let error = validate_prepared_truth_write(
            "run",
            "context",
            &decisions,
            &slots,
            &outcomes,
            &raw_sections(),
            &projection(),
            None,
        )
        .unwrap_err();
        assert!(matches!(&error, DomainError::ContractViolation { .. }));
        assert!(error.to_string().contains("duplicate extractor candidate"));
    }

    #[test]
    fn orphan_semantic_outcome_or_wrong_projection_is_rejected() {
        let decisions = vec![decision("candidate")];
        let slots = vec!["slot".into()];
        let outcomes = BTreeMap::from([(
            "orphan-slot".into(),
            SemanticTruthWriteOutcome::Verified(&decisions[0]),
        )]);
        let sections = raw_sections();
        let change = projection();
        assert!(matches!(
            validate_prepared_truth_write(
                "run", "context", &decisions, &slots, &outcomes, &sections, &change, None,
            ),
            Err(DomainError::ContractViolation { .. })
        ));

        let outcomes = BTreeMap::from([(
            "slot".into(),
            SemanticTruthWriteOutcome::Verified(&decisions[0]),
        )]);
        let mut wrong = change;
        wrong.run_id = "another-run".into();
        assert!(matches!(
            validate_prepared_truth_write(
                "run", "context", &decisions, &slots, &outcomes, &sections, &wrong, None,
            ),
            Err(DomainError::ContractViolation { .. })
        ));
        wrong.run_id = "run".into();
        wrong.upserts[0].rule_instance_id = "unknown-semantic-slot".into();
        assert!(matches!(
            validate_prepared_truth_write(
                "run", "context", &decisions, &slots, &outcomes, &sections, &wrong, None,
            ),
            Err(DomainError::ContractViolation { .. })
        ));
    }

    #[test]
    fn incomplete_embedding_batch_cannot_stage_partial_outbox() {
        let decisions = vec![decision("candidate")];
        let slots = vec!["slot".into()];
        let outcomes = BTreeMap::from([(
            "slot".into(),
            SemanticTruthWriteOutcome::Verified(&decisions[0]),
        )]);
        let embeddings = VerifiedRuleEmbeddingBatch {
            model: "voyage".into(),
            vectors: Vec::new(),
        };
        assert!(matches!(
            validate_prepared_truth_write(
                "run",
                "context",
                &decisions,
                &slots,
                &outcomes,
                &raw_sections(),
                &projection(),
                Some(&embeddings),
            ),
            Err(DomainError::ContractViolation { .. })
        ));
    }
}
