//! Truth decisions for candidates loaded from durable storage. SQL effects and
//! JSON decoding remain with the persistence adapter.

use crate::evidence::{
    classify_candidate_completeness_from_flags, classify_candidate_freshness_local,
};
use crate::truth_candidates::{TruthAdjudicationResult, TruthParamValue, TruthStructuredCandidate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedCandidateClassification {
    pub epistemic_status: String,
    pub freshness_class: String,
    pub completeness_class: String,
}

/// Legacy records must not bypass scope, concept or evidence validation simply
/// because they were previously persisted as structured.
pub fn classify_persisted_candidate(
    epistemic_status: &str,
    scope: &TruthParamValue,
    unresolved_applicability: bool,
    concept_exists: bool,
    uncertainty_flags: &[String],
) -> PersistedCandidateClassification {
    let unrestricted_scope = matches!(scope, TruthParamValue::Object(fields) if fields.is_empty());
    let mut status = epistemic_status.to_string();
    if status != "rejected" && (!unrestricted_scope || unresolved_applicability || !concept_exists)
    {
        status = "needs_hitl".to_string();
    }

    let freshness_class = classify_candidate_freshness_local(uncertainty_flags);
    let completeness_class = classify_candidate_completeness_from_flags(&status, uncertainty_flags);
    if status == "structured" && (freshness_class != "fresh" || completeness_class != "complete") {
        status = "needs_hitl".to_string();
    }

    PersistedCandidateClassification {
        epistemic_status: status,
        freshness_class,
        completeness_class,
    }
}

/// Select the highest-confidence candidate among those actually verified by
/// adjudication; never promote an unrelated or rejected proposal.
pub fn canonical_verified_candidate_id<'a>(
    candidates: &'a [TruthStructuredCandidate],
    adjudication: &TruthAdjudicationResult,
) -> Option<&'a str> {
    candidates
        .iter()
        .filter(|candidate| {
            adjudication.decisions.iter().any(|decision| {
                decision.decision == "verified"
                    && decision.rule_candidate_id == candidate.rule_candidate_id
            })
        })
        .max_by(|left, right| {
            left.confidence
                .partial_cmp(&right.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|candidate| candidate.rule_candidate_id.as_str())
}

pub fn canonical_verification_method(adjudication: &TruthAdjudicationResult) -> String {
    adjudication
        .decisions
        .iter()
        .find(|decision| decision.decision == "verified")
        .map(|decision| decision.verification_method.clone())
        .unwrap_or_else(|| "truth_adjudication@1".to_string())
}

pub fn canonical_adjudication_reason(adjudication: &TruthAdjudicationResult) -> String {
    adjudication
        .decisions
        .first()
        .map(|decision| decision.adjudication_reason.clone())
        .unwrap_or_else(|| "truth_adjudication_result_missing".to_string())
}

pub fn canonical_role_key(role: &str) -> String {
    role.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::truth_candidates::TruthAdjudicationDecision;

    fn classify(
        status: &str,
        scope: TruthParamValue,
        flags: &[&str],
    ) -> PersistedCandidateClassification {
        classify_persisted_candidate(
            status,
            &scope,
            false,
            true,
            &flags
                .iter()
                .map(|flag| (*flag).to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn only_empty_object_scope_is_unrestricted() {
        assert_eq!(
            classify("structured", TruthParamValue::default(), &[]).epistemic_status,
            "structured"
        );
        for scope in [
            TruthParamValue::Null,
            TruthParamValue::List(vec![]),
            TruthParamValue::object([("applicant_type", TruthParamValue::Text("student".into()))]),
        ] {
            assert_eq!(
                classify("structured", scope, &[]).epistemic_status,
                "needs_hitl"
            );
        }
    }

    #[test]
    fn unmodeled_applicability_and_missing_concept_require_review() {
        for (unresolved, concept_exists) in [(true, true), (false, false)] {
            let result = classify_persisted_candidate(
                "structured",
                &TruthParamValue::default(),
                unresolved,
                concept_exists,
                &[],
            );
            assert_eq!(result.epistemic_status, "needs_hitl");
            assert_eq!(result.completeness_class, "partial");
        }
    }

    #[test]
    fn evidence_uncertainty_demotes_structured_but_preserves_classification() {
        let stale = classify("structured", TruthParamValue::default(), &["stale_source"]);
        assert_eq!(
            (
                &*stale.epistemic_status,
                &*stale.freshness_class,
                &*stale.completeness_class
            ),
            ("needs_hitl", "stale", "complete")
        );
        let incomplete = classify(
            "structured",
            TruthParamValue::default(),
            &["validator:missing_numeric_params"],
        );
        assert_eq!(
            (
                &*incomplete.epistemic_status,
                &*incomplete.completeness_class
            ),
            ("needs_hitl", "incomplete")
        );
        let rejected = classify("rejected", TruthParamValue::Null, &["stale_source"]);
        assert_eq!(rejected.epistemic_status, "rejected");
    }

    #[test]
    fn canonical_candidate_comes_only_from_verified_decisions() {
        let candidates = [
            TruthStructuredCandidate {
                rule_candidate_id: "low".into(),
                confidence: 0.6,
                ..Default::default()
            },
            TruthStructuredCandidate {
                rule_candidate_id: "rejected".into(),
                confidence: 1.0,
                ..Default::default()
            },
            TruthStructuredCandidate {
                rule_candidate_id: "high".into(),
                confidence: 0.9,
                ..Default::default()
            },
        ];
        let adjudication = TruthAdjudicationResult {
            decisions: [
                ("low", "verified"),
                ("rejected", "rejected"),
                ("high", "verified"),
            ]
            .into_iter()
            .map(|(id, decision)| TruthAdjudicationDecision {
                rule_candidate_id: id.into(),
                decision: decision.into(),
                verification_method: format!("method:{id}"),
                adjudication_reason: format!("reason:{id}"),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            canonical_verified_candidate_id(&candidates, &adjudication),
            Some("high")
        );
        assert_eq!(canonical_verification_method(&adjudication), "method:low");
        assert_eq!(canonical_adjudication_reason(&adjudication), "reason:low");
        assert_eq!(canonical_role_key("  VISA_FEE  "), "visa_fee");
    }

    #[test]
    fn missing_verified_candidate_and_decisions_fail_closed() {
        let candidates = [TruthStructuredCandidate {
            rule_candidate_id: "only".into(),
            confidence: 0.9,
            ..Default::default()
        }];
        let adjudication = TruthAdjudicationResult {
            decisions: vec![TruthAdjudicationDecision {
                rule_candidate_id: "absent".into(),
                decision: "verified".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            canonical_verified_candidate_id(&candidates, &adjudication),
            None
        );
        let empty = TruthAdjudicationResult::default();
        assert_eq!(canonical_verified_candidate_id(&candidates, &empty), None);
        assert_eq!(
            canonical_verification_method(&empty),
            "truth_adjudication@1"
        );
        assert_eq!(
            canonical_adjudication_reason(&empty),
            "truth_adjudication_result_missing"
        );
    }
}
