use std::collections::{BTreeMap, BTreeSet};

use primitives::truth_candidates::{
    TruthAdjudicationDecision, TruthAdjudicationResult, TruthStructuredCandidate,
};

#[derive(Debug, Clone, Default)]
pub struct SourceGovernanceRecord {
    pub source_type: String,
    pub trust_level: i64,
    pub authority_class: String,
    pub independence_group_key: String,
    pub freshness_ttl_days: i32,
    pub override_eligible: bool,
}

pub fn adjudicate_truth_candidates_with_governance(
    candidates: &[TruthStructuredCandidate],
    source_registry: &BTreeMap<String, SourceGovernanceRecord>,
) -> TruthAdjudicationResult {
    if candidates.is_empty() {
        return TruthAdjudicationResult {
            overall_status: "rejected".to_string(),
            rejected_count: 1,
            decisions: vec![TruthAdjudicationDecision {
                rule_candidate_id: String::new(),
                decision: "rejected".to_string(),
                publish_admissibility: "not_admissible".to_string(),
                verification_method: "truth_governance@1".to_string(),
                adjudication_reason: "no_structured_candidates".to_string(),
            }],
            ..TruthAdjudicationResult::default()
        };
    }

    let mut decisions = Vec::new();
    let mut structured = Vec::new();
    let mut rejected_count = 0usize;

    for candidate in candidates {
        if candidate.epistemic_status != "structured" {
            rejected_count += 1;
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "rejected".to_string(),
                publish_admissibility: "not_admissible".to_string(),
                verification_method: "truth_governance@1".to_string(),
                adjudication_reason: "non_structured_input".to_string(),
            });
            continue;
        }
        if let Some(reason) = freshness_or_completeness_block_reason(
            candidate.freshness_class.as_str(),
            candidate.completeness_class.as_str(),
        ) {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "needs_hitl".to_string(),
                publish_admissibility: "needs_hitl".to_string(),
                verification_method: "truth_governance@1".to_string(),
                adjudication_reason: reason,
            });
            continue;
        }
        structured.push(candidate);
    }

    let signal_tiers = BTreeSet::from_iter(
        structured
            .iter()
            .map(|candidate| candidate.source_tier.clone())
            .filter(|tier| !tier.trim().is_empty()),
    );
    let authority_classes = BTreeSet::from_iter(structured.iter().map(|candidate| {
        source_registry
            .get(&candidate.source_key)
            .map(|record| record.authority_class.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    }));

    let contradiction_count = contradiction_count(structured.as_slice());
    if contradiction_count > 0 {
        for candidate in structured {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "needs_hitl".to_string(),
                publish_admissibility: "needs_hitl".to_string(),
                verification_method: "truth_governance@1".to_string(),
                adjudication_reason: format!(
                    "contradictory_structured_candidates; source_tiers={}; authority_classes={}",
                    join_set(&signal_tiers),
                    join_set(&authority_classes),
                ),
            });
        }
        let needs_hitl_count = decisions
            .iter()
            .filter(|decision| decision.decision == "needs_hitl")
            .count();
        return TruthAdjudicationResult {
            overall_status: "needs_hitl".to_string(),
            verified_count: 0,
            needs_hitl_count,
            rejected_count,
            contradiction_count,
            decisions,
        };
    }

    let unique_independence_groups = BTreeSet::from_iter(
        structured
            .iter()
            .map(|candidate| independence_group(candidate, source_registry)),
    );
    let has_independent_corroboration = unique_independence_groups.len() >= 2;
    let has_authoritative_support = structured.iter().any(|candidate| {
        source_registry
            .get(&candidate.source_key)
            .map(|record| is_corroboration_authority(&record.authority_class))
            .unwrap_or(false)
    });

    if has_independent_corroboration {
        if !has_authoritative_support {
            for candidate in structured {
                decisions.push(TruthAdjudicationDecision {
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_governance@1".to_string(),
                    adjudication_reason: format!(
                        "weak_source_corroboration; independence_groups={}; source_tiers={}; authority_classes={}",
                        join_set(&unique_independence_groups),
                        join_set(&signal_tiers),
                        join_set(&authority_classes),
                    ),
                });
            }
            let needs_hitl_count = decisions
                .iter()
                .filter(|decision| decision.decision == "needs_hitl")
                .count();
            return TruthAdjudicationResult {
                overall_status: "needs_hitl".to_string(),
                verified_count: 0,
                needs_hitl_count,
                rejected_count,
                contradiction_count: 0,
                decisions,
            };
        }
        for candidate in structured {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "verified".to_string(),
                publish_admissibility: "admissible".to_string(),
                verification_method: "truth_governance@1".to_string(),
                adjudication_reason: format!(
                    "independent_corroboration; independence_groups={}; source_tiers={}; authority_classes={}",
                    join_set(&unique_independence_groups),
                    join_set(&signal_tiers),
                    join_set(&authority_classes),
                ),
            });
        }
        let verified_count = decisions
            .iter()
            .filter(|decision| decision.decision == "verified")
            .count();
        return TruthAdjudicationResult {
            overall_status: "verified".to_string(),
            verified_count,
            needs_hitl_count: 0,
            rejected_count,
            contradiction_count: 0,
            decisions,
        };
    }

    if structured.len() == 1 {
        let candidate = structured[0];
        let governance = source_registry
            .get(&candidate.source_key)
            .cloned()
            .unwrap_or_default();
        if governance.override_eligible && is_override_authority(&governance.authority_class) {
            return TruthAdjudicationResult {
                overall_status: "verified".to_string(),
                verified_count: 1,
                needs_hitl_count: 0,
                rejected_count,
                contradiction_count: 0,
                decisions: vec![TruthAdjudicationDecision {
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    decision: "verified".to_string(),
                    publish_admissibility: "admissible".to_string(),
                    verification_method: "authority_override@1".to_string(),
                    adjudication_reason: format!(
                        "single_authoritative_source_override; authority_class={}; freshness_ttl_days={}",
                        governance.authority_class,
                        governance.freshness_ttl_days,
                    ),
                }],
            };
        }
        if governance.override_eligible && !is_override_authority(&governance.authority_class) {
            return TruthAdjudicationResult {
                overall_status: "needs_hitl".to_string(),
                verified_count: 0,
                needs_hitl_count: 1,
                rejected_count,
                contradiction_count: 0,
                decisions: vec![TruthAdjudicationDecision {
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_governance@1".to_string(),
                    adjudication_reason: format!(
                        "override_not_allowed_for_authority_class; authority_class={}",
                        governance.authority_class,
                    ),
                }],
            };
        }
    }

    let reason = if !structured.is_empty()
        && unique_independence_groups.len() == 1
        && structured.len() >= 2
    {
        format!(
            "non_independent_corroboration; independence_group={}; source_tiers={}; authority_classes={}",
            join_set(&unique_independence_groups),
            join_set(&signal_tiers),
            join_set(&authority_classes),
        )
    } else {
        format!(
            "single_source_requires_corroboration; source_tiers={}; authority_classes={}",
            join_set(&signal_tiers),
            join_set(&authority_classes),
        )
    };

    for candidate in structured {
        decisions.push(TruthAdjudicationDecision {
            rule_candidate_id: candidate.rule_candidate_id.clone(),
            decision: "needs_hitl".to_string(),
            publish_admissibility: "needs_hitl".to_string(),
            verification_method: "truth_governance@1".to_string(),
            adjudication_reason: reason.clone(),
        });
    }

    let needs_hitl_count = decisions
        .iter()
        .filter(|decision| decision.decision == "needs_hitl")
        .count();
    TruthAdjudicationResult {
        overall_status: "needs_hitl".to_string(),
        verified_count: 0,
        needs_hitl_count,
        rejected_count,
        contradiction_count: 0,
        decisions,
    }
}

fn contradiction_count(candidates: &[&TruthStructuredCandidate]) -> usize {
    let signatures = BTreeSet::from_iter(
        candidates
            .iter()
            .map(|candidate| stable_value_signature(candidate)),
    );
    signatures.len().saturating_sub(1)
}

fn stable_value_signature(candidate: &TruthStructuredCandidate) -> String {
    format!(
        "{}|{}|{}",
        candidate.role,
        candidate.concept_canonical_key,
        serde_json::to_string(&candidate.params).unwrap_or_else(|_| "null".to_string())
    )
}

fn independence_group(
    candidate: &TruthStructuredCandidate,
    registry: &BTreeMap<String, SourceGovernanceRecord>,
) -> String {
    registry
        .get(&candidate.source_key)
        .map(|record| record.independence_group_key.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if candidate.source_key.trim().is_empty() {
                "unknown".to_string()
            } else {
                candidate.source_key.clone()
            }
        })
}

fn is_override_authority(authority_class: &str) -> bool {
    matches!(
        authority_class,
        "primary_authority" | "delegated_authority" | "official_publisher"
    )
}

fn is_corroboration_authority(authority_class: &str) -> bool {
    matches!(
        authority_class,
        "primary_authority" | "delegated_authority" | "official_publisher"
    )
}

fn freshness_or_completeness_block_reason(
    freshness_class: &str,
    completeness_class: &str,
) -> Option<String> {
    if freshness_class != "fresh" {
        return Some(format!(
            "freshness_block; freshness_class={freshness_class}"
        ));
    }
    if completeness_class != "complete" {
        return Some(format!(
            "completeness_block; completeness_class={completeness_class}"
        ));
    }
    None
}

fn join_set(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.iter().cloned().collect::<Vec<_>>().join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use primitives::truth_candidates::TruthParamValue;

    fn candidate(id: &str, source_key: &str, amount: i64) -> TruthStructuredCandidate {
        TruthStructuredCandidate {
            rule_candidate_id: id.to_string(),
            context_key: "ctx".to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            params: TruthParamValue::Object(BTreeMap::from([
                (
                    "amount".to_string(),
                    TruthParamValue::Decimal(amount as f64),
                ),
                (
                    "currency".to_string(),
                    TruthParamValue::Text("EUR".to_string()),
                ),
            ])),
            source_key: source_key.to_string(),
            source_tier: "government".to_string(),
            confidence: 0.9,
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            evidence_quote: "Consular fee".to_string(),
            epistemic_status: "structured".to_string(),
        }
    }

    #[test]
    fn independent_sources_verify() {
        let registry = BTreeMap::from([
            (
                "a".to_string(),
                SourceGovernanceRecord {
                    authority_class: "primary_authority".to_string(),
                    independence_group_key: "gov-a".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
            (
                "b".to_string(),
                SourceGovernanceRecord {
                    authority_class: "delegated_authority".to_string(),
                    independence_group_key: "gov-b".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
        ]);
        let result = adjudicate_truth_candidates_with_governance(
            &[candidate("1", "a", 80), candidate("2", "b", 80)],
            &registry,
        );
        assert_eq!(result.overall_status, "verified");
    }

    #[test]
    fn non_independent_same_group_needs_hitl() {
        let registry = BTreeMap::from([
            (
                "a".to_string(),
                SourceGovernanceRecord {
                    authority_class: "primary_authority".to_string(),
                    independence_group_key: "shared-gov".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
            (
                "b".to_string(),
                SourceGovernanceRecord {
                    authority_class: "official_publisher".to_string(),
                    independence_group_key: "shared-gov".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
        ]);
        let result = adjudicate_truth_candidates_with_governance(
            &[candidate("1", "a", 80), candidate("2", "b", 80)],
            &registry,
        );
        assert_eq!(result.overall_status, "needs_hitl");
        assert!(result.decisions[0]
            .adjudication_reason
            .contains("non_independent_corroboration"));
    }

    #[test]
    fn authoritative_single_source_override_verifies() {
        let registry = BTreeMap::from([(
            "a".to_string(),
            SourceGovernanceRecord {
                authority_class: "primary_authority".to_string(),
                override_eligible: true,
                freshness_ttl_days: 14,
                ..SourceGovernanceRecord::default()
            },
        )]);
        let result =
            adjudicate_truth_candidates_with_governance(&[candidate("1", "a", 80)], &registry);
        assert_eq!(result.overall_status, "verified");
        assert_eq!(
            result.decisions[0].verification_method,
            "authority_override@1"
        );
    }

    #[test]
    fn weak_source_corroboration_does_not_verify() {
        let registry = BTreeMap::from([
            (
                "a".to_string(),
                SourceGovernanceRecord {
                    source_type: "editorial".to_string(),
                    trust_level: 2,
                    authority_class: "editorial".to_string(),
                    independence_group_key: "media-a".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
            (
                "b".to_string(),
                SourceGovernanceRecord {
                    source_type: "forum".to_string(),
                    trust_level: 1,
                    authority_class: "forum".to_string(),
                    independence_group_key: "forum-b".to_string(),
                    ..SourceGovernanceRecord::default()
                },
            ),
        ]);
        let result = adjudicate_truth_candidates_with_governance(
            &[candidate("1", "a", 80), candidate("2", "b", 80)],
            &registry,
        );
        assert_eq!(result.overall_status, "needs_hitl");
        assert!(result.decisions.iter().all(|decision| decision
            .adjudication_reason
            .contains("weak_source_corroboration")));
    }

    #[test]
    fn stale_candidate_is_blocked_before_verification() {
        let registry = BTreeMap::from([(
            "a".to_string(),
            SourceGovernanceRecord {
                authority_class: "primary_authority".to_string(),
                independence_group_key: "gov-a".to_string(),
                ..SourceGovernanceRecord::default()
            },
        )]);
        let mut stale = candidate("1", "a", 80);
        stale.freshness_class = "stale".to_string();
        let result = adjudicate_truth_candidates_with_governance(&[stale], &registry);
        assert_eq!(result.overall_status, "needs_hitl");
        assert!(result.decisions[0]
            .adjudication_reason
            .contains("freshness_block; freshness_class=stale"));
    }

    #[test]
    fn override_is_denied_for_non_authoritative_source_class() {
        let registry = BTreeMap::from([(
            "a".to_string(),
            SourceGovernanceRecord {
                authority_class: "editorial".to_string(),
                override_eligible: true,
                ..SourceGovernanceRecord::default()
            },
        )]);
        let result =
            adjudicate_truth_candidates_with_governance(&[candidate("1", "a", 80)], &registry);
        assert_eq!(result.overall_status, "needs_hitl");
        assert!(result.decisions[0]
            .adjudication_reason
            .contains("override_not_allowed_for_authority_class"));
    }
}
