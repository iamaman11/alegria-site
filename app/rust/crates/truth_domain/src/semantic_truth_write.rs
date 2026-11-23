//! Domain decisions for writing an adjudicated semantic Truth identity.
//! Temporal orchestrates and persistence adapters perform the external effects.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::review_decisions::TruthAdjudicationCandidateDecision;
use crate::truth_candidates::{truth_scope_target, TruthParamValue};
use crate::truth_governance::SourceGovernanceRecord;

// A semantic slot may have several independently adjudicated source candidates,
// but must have exactly one durable outcome, independent of input ordering.
#[derive(Debug)]
pub enum SemanticTruthWriteOutcome<'a> {
    Verified(&'a TruthAdjudicationCandidateDecision),
    Disputed,
    Rejected(&'a TruthAdjudicationCandidateDecision),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruthWriteError {
    EmptySemanticIdentity,
    UnsupportedCandidateDecision(String),
    NoAdmissibleCandidate,
    VerifiedWithoutEvidence(String),
    UnsupportedScope(String),
}

impl fmt::Display for TruthWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySemanticIdentity => {
                write!(f, "cannot write an empty semantic truth identity")
            }
            Self::UnsupportedCandidateDecision(value) => {
                write!(f, "unsupported candidate decision: {value}")
            }
            Self::NoAdmissibleCandidate => write!(f, "semantic slot has no admissible candidate"),
            Self::VerifiedWithoutEvidence(id) => {
                write!(
                    f,
                    "verified candidate {id} has no exact source evidence quote"
                )
            }
            Self::UnsupportedScope(id) => {
                write!(f, "candidate {id} uses an unsupported scoped predicate")
            }
        }
    }
}

impl std::error::Error for TruthWriteError {}

pub fn semantic_truth_write_outcome<'a>(
    candidates: &[&'a TruthAdjudicationCandidateDecision],
    source_registry: &BTreeMap<String, SourceGovernanceRecord>,
) -> Result<SemanticTruthWriteOutcome<'a>, TruthWriteError> {
    if candidates.is_empty() {
        return Err(TruthWriteError::EmptySemanticIdentity);
    }
    let mut verified = Vec::new();
    let mut unresolved = false;
    for candidate in candidates {
        match candidate.decision.as_str() {
            "verified" if candidate.publish_admissibility == "admissible" => {
                verified.push(*candidate);
            }
            "verified" | "needs_hitl" => unresolved = true,
            "rejected" => (),
            other => {
                return Err(TruthWriteError::UnsupportedCandidateDecision(
                    other.to_string(),
                ))
            }
        }
    }

    // A weak repetition may remain on HITL while the SAME claim is confirmed
    // by at least two distinct primary authorities. Any other unresolved claim,
    // contradicted value or rejected human review vetoes the projection.
    if candidates.iter().any(|candidate| {
        candidate.decision == "rejected"
            && candidate.adjudication_reason == "human_rejected_exact_source_evidence"
    }) {
        return Ok(SemanticTruthWriteOutcome::Disputed);
    }
    if unresolved {
        let same_verified_claim = verified
            .first()
            .map(|first| {
                verified.iter().all(|candidate| {
                    candidate.params.stable_signature() == first.params.stable_signature()
                        && candidate.scope.stable_signature() == first.scope.stable_signature()
                })
            })
            .unwrap_or(false);
        let groups = verified
            .iter()
            .filter_map(|candidate| {
                source_registry
                    .get(&candidate.source_key)
                    .and_then(|source| {
                        if source.authority_class == "primary_authority"
                            && source.trust_level >= 5
                            && !source.independence_group_key.trim().is_empty()
                        {
                            Some(source.independence_group_key.clone())
                        } else {
                            None
                        }
                    })
            })
            .collect::<BTreeSet<_>>();
        let unresolved_are_identical_weak_repetitions = verified.first().is_some_and(|first| {
            candidates
                .iter()
                .filter(|candidate| {
                    !(candidate.decision == "verified"
                        && candidate.publish_admissibility == "admissible")
                })
                .all(|candidate| {
                    candidate.decision == "needs_hitl"
                        && candidate.publish_admissibility == "needs_hitl"
                        && candidate.adjudication_reason == "candidate_validation_requires_hitl"
                        && candidate.params.stable_signature() == first.params.stable_signature()
                        && candidate.scope.stable_signature() == first.scope.stable_signature()
                        && source_registry
                            .get(&candidate.source_key)
                            .is_some_and(|source| {
                                source.authority_class == "forum" && source.trust_level <= 1
                            })
                })
        });
        if !same_verified_claim || groups.len() < 2 || !unresolved_are_identical_weak_repetitions {
            return Ok(SemanticTruthWriteOutcome::Disputed);
        }
    }

    if !verified.is_empty() {
        let signatures = verified
            .iter()
            .map(|candidate| candidate.params.stable_signature())
            .collect::<BTreeSet<_>>();
        if signatures.len() != 1 {
            return Ok(SemanticTruthWriteOutcome::Disputed);
        }
        verified.sort_by(|left, right| {
            right
                .confidence
                .total_cmp(&left.confidence)
                .then_with(|| left.rule_candidate_id.cmp(&right.rule_candidate_id))
        });
        return Ok(SemanticTruthWriteOutcome::Verified(verified[0]));
    }

    let rejected = candidates
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.rule_candidate_id.as_str())
        .ok_or(TruthWriteError::NoAdmissibleCandidate)?;
    Ok(SemanticTruthWriteOutcome::Rejected(rejected))
}

/// Reject forged or unsupported verified source decisions before SQL writes.
pub fn validate_verified_truth_write_candidate(
    candidate: &TruthAdjudicationCandidateDecision,
) -> Result<(), TruthWriteError> {
    if candidate.decision != "verified" {
        return Ok(());
    }
    if candidate.evidence_quote.trim().is_empty() {
        return Err(TruthWriteError::VerifiedWithoutEvidence(
            candidate.rule_candidate_id.clone(),
        ));
    }
    if truth_scope_target(&candidate.scope).is_none() {
        return Err(TruthWriteError::UnsupportedScope(
            candidate.rule_candidate_id.clone(),
        ));
    }
    Ok(())
}

pub fn persisted_candidate_status<'a>(
    decision: &'a TruthAdjudicationCandidateDecision,
    outcome: &SemanticTruthWriteOutcome<'_>,
) -> &'a str {
    match outcome {
        SemanticTruthWriteOutcome::Disputed if decision.decision == "verified" => "needs_hitl",
        SemanticTruthWriteOutcome::Rejected(_) => "rejected",
        _ => decision.decision.as_str(),
    }
}

/// Pure fields derived from a Truth decision for the candidate write adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruthCandidateWriteFields {
    pub evidence_quote: String,
    pub span_start: i32,
    pub span_end: i32,
    pub severity: String,
    pub is_numeric: bool,
}

pub fn truth_candidate_write_fields(
    decision: &TruthAdjudicationCandidateDecision,
) -> TruthCandidateWriteFields {
    let evidence_quote = if decision.evidence_quote.trim().is_empty() {
        decision.concept_canonical_key.clone()
    } else {
        decision.evidence_quote.clone()
    };
    let span_start = decision.span_start.max(0) as i32;
    let span_end = std::cmp::max(decision.span_end as i32, span_start + 1);
    let severity = decision
        .params
        .as_object()
        .and_then(|params| params.get("severity"))
        .and_then(|value| match value {
            TruthParamValue::Text(value) => Some(value.as_str()),
            _ => None,
        })
        .filter(|value| matches!(*value, "mandatory" | "recommended" | "optional" | "unknown"))
        .unwrap_or("unknown")
        .to_string();
    let is_numeric = decision
        .params
        .as_object()
        .map(|params| {
            params.contains_key("amount")
                || params.contains_key("min_amount")
                || params.contains_key("max_amount")
                || params.contains_key("days")
                || params.contains_key("min_days")
                || params.contains_key("max_days")
                || params.contains_key("duration_days")
        })
        .unwrap_or(false);
    TruthCandidateWriteFields {
        evidence_quote,
        span_start,
        span_end,
        severity,
        is_numeric,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_verdict(
        id: &str,
        decision: &str,
        amount: i64,
        confidence: f64,
    ) -> TruthAdjudicationCandidateDecision {
        TruthAdjudicationCandidateDecision {
            section_id: 13,
            page_id: 4,
            rule_candidate_id: id.to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            params: TruthParamValue::object([("amount", TruthParamValue::Integer(amount))]),
            scope: TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()),
            source_key: format!("https://example.gov/{id}"),
            source_tier: "government".to_string(),
            confidence,
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            evidence_quote: "consular fee".to_string(),
            span_start: 0,
            span_end: 12,
            source_snapshot_hash: "snapshot".to_string(),
            provider_key: "deterministic".to_string(),
            model_key: "cutover-runtime".to_string(),
            prompt_version: "cutover@1".to_string(),
            decision: decision.to_string(),
            publish_admissibility: match decision {
                "verified" => "admissible",
                "needs_hitl" => "needs_hitl",
                _ => "not_admissible",
            }
            .to_string(),
            verification_method: "test".to_string(),
            adjudication_reason: "test_fixture".to_string(),
        }
    }

    #[test]
    fn rejected_source_never_demotes_independently_verified_semantic_slot() {
        let official = candidate_verdict("official", "verified", 80, 0.95);
        let corroborating = candidate_verdict("corroborating", "verified", 80, 0.90);
        let fabricated = candidate_verdict("fabricated", "rejected", 120, 0.99);
        for order in [
            vec![&official, &corroborating, &fabricated],
            vec![&fabricated, &corroborating, &official],
        ] {
            match semantic_truth_write_outcome(&order, &BTreeMap::new()).unwrap() {
                SemanticTruthWriteOutcome::Verified(candidate) => {
                    assert_eq!(candidate.rule_candidate_id, "official");
                }
                _ => panic!("rejected proposals must not demote verified evidence"),
            }
        }
    }

    #[test]
    fn unresolved_or_conflicting_verified_sources_hold_entire_semantic_slot() {
        let official = candidate_verdict("official", "verified", 80, 0.95);
        let corroborating = candidate_verdict("corroborating", "verified", 80, 0.90);
        let unresolved = candidate_verdict("unresolved", "needs_hitl", 120, 0.91);
        let inconsistent = candidate_verdict("inconsistent", "verified", 120, 0.98);
        assert!(matches!(
            semantic_truth_write_outcome(
                &[&official, &corroborating, &unresolved],
                &BTreeMap::new()
            )
            .unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        assert!(matches!(
            semantic_truth_write_outcome(
                &[&official, &corroborating, &inconsistent],
                &BTreeMap::new()
            )
            .unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
    }

    #[test]
    fn independent_official_verification_survives_only_identical_forum_hitl() {
        let first = candidate_verdict("first", "verified", 80, 0.98);
        let second = candidate_verdict("second", "verified", 80, 0.96);
        let mut forum = candidate_verdict("forum", "needs_hitl", 80, 0.6);
        forum.adjudication_reason = "candidate_validation_requires_hitl".to_string();
        forum.source_tier = "low_trust".to_string();
        let registry = BTreeMap::from([
            (
                first.source_key.clone(),
                SourceGovernanceRecord {
                    authority_class: "primary_authority".to_string(),
                    trust_level: 5,
                    independence_group_key: "official-first".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
            (
                second.source_key.clone(),
                SourceGovernanceRecord {
                    authority_class: "primary_authority".to_string(),
                    trust_level: 5,
                    independence_group_key: "official-second".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
            (
                forum.source_key.clone(),
                SourceGovernanceRecord {
                    authority_class: "forum".to_string(),
                    trust_level: 1,
                    ..SourceGovernanceRecord::default()
                },
            ),
        ]);
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &forum], &registry).unwrap(),
            SemanticTruthWriteOutcome::Verified(candidate)
                if candidate.rule_candidate_id == "first"
        ));
        forum.params = TruthParamValue::object([("amount", TruthParamValue::Integer(120))]);
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &forum], &registry).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        forum.params = first.params.clone();
        let mut same_owner = registry.clone();
        same_owner
            .get_mut(&second.source_key)
            .unwrap()
            .independence_group_key = "official-first".to_string();
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &forum], &same_owner).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        forum.adjudication_reason = "resolution_loop_requires_hitl".to_string();
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &forum], &registry).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        forum.adjudication_reason = "candidate_validation_requires_hitl".to_string();
        forum.source_key = first.source_key.clone();
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &forum], &registry).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        forum.source_key = "https://example.gov/forum".to_string();
        let mut review_rejected = candidate_verdict("review-rejected", "rejected", 80, 0.8);
        review_rejected.adjudication_reason = "human_rejected_exact_source_evidence".to_string();
        assert!(matches!(
            semantic_truth_write_outcome(&[&first, &second, &review_rejected], &registry).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
    }

    #[test]
    fn all_rejected_identity_is_demoted_once_and_unknown_status_fails_closed() {
        let rejected = candidate_verdict("rejected", "rejected", 80, 0.1);
        match semantic_truth_write_outcome(&[&rejected], &BTreeMap::new()).unwrap() {
            SemanticTruthWriteOutcome::Rejected(candidate) => {
                assert_eq!(candidate.rule_candidate_id, "rejected");
            }
            _ => panic!("all rejected evidence must demote the slot"),
        }
        let unexpected = candidate_verdict("unknown", "approved", 80, 1.0);
        assert!(semantic_truth_write_outcome(&[&unexpected], &BTreeMap::new()).is_err());
    }

    #[test]
    fn verified_write_requires_exact_evidence_and_supported_scope() {
        let mut verified = candidate_verdict("official", "verified", 80, 0.95);
        verified.evidence_quote = " ".to_string();
        assert_eq!(
            validate_verified_truth_write_candidate(&verified),
            Err(TruthWriteError::VerifiedWithoutEvidence("official".into()))
        );
        verified.evidence_quote = "consular fee".to_string();
        verified.scope =
            TruthParamValue::object([("applicant_type", TruthParamValue::Text("student".into()))]);
        assert_eq!(
            validate_verified_truth_write_candidate(&verified),
            Err(TruthWriteError::UnsupportedScope("official".into()))
        );
        verified.scope = TruthParamValue::object([(
            "applicant_profile",
            TruthParamValue::Text("student".into()),
        )]);
        assert!(validate_verified_truth_write_candidate(&verified).is_ok());
        verified.decision = "rejected".into();
        verified.evidence_quote.clear();
        assert!(validate_verified_truth_write_candidate(&verified).is_ok());
    }

    #[test]
    fn semantic_slot_outcome_controls_persisted_status() {
        let verified = candidate_verdict("verified", "verified", 80, 0.95);
        let unresolved = candidate_verdict("unresolved", "needs_hitl", 80, 0.6);
        let rejected = candidate_verdict("rejected", "rejected", 80, 0.1);
        assert_eq!(
            persisted_candidate_status(&verified, &SemanticTruthWriteOutcome::Disputed),
            "needs_hitl"
        );
        assert_eq!(
            persisted_candidate_status(&unresolved, &SemanticTruthWriteOutcome::Disputed),
            "needs_hitl"
        );
        assert_eq!(
            persisted_candidate_status(&verified, &SemanticTruthWriteOutcome::Verified(&verified)),
            "verified"
        );
        assert_eq!(
            persisted_candidate_status(&verified, &SemanticTruthWriteOutcome::Rejected(&rejected)),
            "rejected"
        );
    }

    #[test]
    fn candidate_write_fields_preserve_evidence_and_numeric_semantics() {
        let mut candidate = candidate_verdict("official", "needs_hitl", 80, 0.9);
        candidate.evidence_quote.clear();
        candidate.span_start = 9;
        candidate.span_end = 7;
        candidate.params = TruthParamValue::object([
            ("amount", TruthParamValue::Integer(80)),
            ("severity", TruthParamValue::Text("mandatory".into())),
        ]);
        assert_eq!(
            truth_candidate_write_fields(&candidate),
            TruthCandidateWriteFields {
                evidence_quote: "consular_fee".into(),
                span_start: 9,
                span_end: 10,
                severity: "mandatory".into(),
                is_numeric: true,
            }
        );
        candidate.params =
            TruthParamValue::object([("severity", TruthParamValue::Text("invalid".into()))]);
        let fields = truth_candidate_write_fields(&candidate);
        assert_eq!(fields.severity, "unknown");
        assert!(!fields.is_numeric);
    }
}
