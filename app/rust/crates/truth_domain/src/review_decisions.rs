//! Deterministic review and semantic identity decisions.
//! Database reads and workflow orchestration remain in the caller.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::contradictions::FactAssertion;
use crate::truth_candidates::{truth_scope_target, TruthParamValue};
use crate::truth_governance::SourceGovernanceRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedTruthCandidateRecord {
    pub section_id: i64,
    pub page_id: i64,
    pub rule_candidate_id: String,
    pub role: String,
    pub concept_canonical_key: String,
    pub raw_mention: String,
    pub params: TruthParamValue,
    #[serde(default)]
    pub scope: TruthParamValue,
    pub source_key: String,
    pub source_tier: String,
    pub confidence: f64,
    pub evidence_quote: String,
    pub span_start: usize,
    pub span_end: usize,
    pub source_snapshot_hash: String,
    #[serde(default)]
    pub provider_key: String,
    #[serde(default)]
    pub model_key: String,
    #[serde(default)]
    pub prompt_version: String,
    pub freshness_class: String,
    pub completeness_class: String,
    pub epistemic_status: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateValidationSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub candidates: Vec<ValidatedTruthCandidateRecord>,
    pub accepted_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub blocked_by_gate: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationCandidateDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub rule_candidate_id: String,
    pub role: String,
    pub concept_canonical_key: String,
    pub params: TruthParamValue,
    #[serde(default)]
    pub scope: TruthParamValue,
    pub source_key: String,
    pub source_tier: String,
    pub confidence: f64,
    pub freshness_class: String,
    pub completeness_class: String,
    pub evidence_quote: String,
    pub span_start: usize,
    pub span_end: usize,
    pub source_snapshot_hash: String,
    #[serde(default)]
    pub provider_key: String,
    #[serde(default)]
    pub model_key: String,
    #[serde(default)]
    pub prompt_version: String,
    pub decision: String,
    pub publish_admissibility: String,
    pub verification_method: String,
    pub adjudication_reason: String,
}

pub fn non_structured_candidate_reason(candidate: &ValidatedTruthCandidateRecord) -> String {
    if candidate.freshness_class != "fresh" {
        format!(
            "freshness_block; freshness_class={}",
            candidate.freshness_class
        )
    } else if candidate.completeness_class == "incomplete" {
        format!(
            "completeness_block; completeness_class={}",
            candidate.completeness_class
        )
    } else if candidate.epistemic_status == "needs_hitl" {
        "candidate_validation_requires_hitl".to_string()
    } else {
        "non_structured_input".to_string()
    }
}

pub fn validated_candidate_assertions(
    state: &CandidateValidationSectionState,
) -> Vec<FactAssertion> {
    state
        .candidates
        .iter()
        .filter(|candidate| candidate.epistemic_status != "rejected")
        .flat_map(|candidate| {
            let subject = format!(
                "section:{}:{}:{}:scope:{}",
                state.section_id,
                candidate.role,
                candidate.concept_canonical_key,
                candidate.scope.stable_signature()
            );
            let assertion = |predicate_key: String, value_normalized: String| FactAssertion {
                subject_key: subject.clone(),
                predicate_key,
                value_normalized,
                source_key: Some(candidate.source_key.clone()),
                confidence: candidate.confidence as f32,
            };
            match candidate.params.as_object() {
                Some(fields) if !fields.is_empty() => fields
                    .iter()
                    .map(|(key, value)| assertion(format!("param:{key}"), value.stable_signature()))
                    .collect::<Vec<_>>(),
                _ => vec![assertion(
                    "structured_params".to_string(),
                    candidate.params.stable_signature(),
                )],
            }
        })
        .collect()
}

// An unresolved low-trust repetition does not override two independently
// confirmed, identical primary-source assertions. The weak claim stays HITL.
fn independently_confirmed_weak_duplicate(
    weak: &TruthAdjudicationCandidateDecision,
    sections: &[CandidateValidationSectionState],
    decisions: &[TruthAdjudicationCandidateDecision],
    registry: &BTreeMap<String, SourceGovernanceRecord>,
) -> bool {
    if weak.decision != "needs_hitl"
        || weak.adjudication_reason != "candidate_validation_requires_hitl"
    {
        return false;
    }
    let Some(source) = registry.get(&weak.source_key) else {
        return false;
    };
    if source.authority_class != "forum" || source.trust_level > 1 {
        return false;
    }
    let decided = decisions
        .iter()
        .map(|decision| decision.rule_candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut independent_primary_groups = BTreeSet::new();
    for section in sections.iter().filter(|section| !section.blocked_by_gate) {
        for candidate in &section.candidates {
            if candidate.epistemic_status != "structured"
                || decided.contains(candidate.rule_candidate_id.as_str())
                || candidate.role != weak.role
                || candidate.concept_canonical_key != weak.concept_canonical_key
                || candidate.params.stable_signature() != weak.params.stable_signature()
                || candidate.scope.stable_signature() != weak.scope.stable_signature()
            {
                continue;
            }
            let Some(governance) = registry.get(&candidate.source_key) else {
                continue;
            };
            if governance.authority_class == "primary_authority"
                && governance.trust_level >= 5
                && !governance.independence_group_key.trim().is_empty()
            {
                independent_primary_groups.insert(governance.independence_group_key.clone());
            }
        }
    }
    independent_primary_groups.len() >= 2
}

fn held_scope_key(scope: &TruthParamValue) -> String {
    if truth_scope_target(scope).is_some() {
        scope.stable_signature()
    } else {
        TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()).stable_signature()
    }
}

pub fn semantic_identity_is_held(
    held: &BTreeSet<(String, String, String)>,
    role: &str,
    concept: &str,
    scope: &TruthParamValue,
) -> bool {
    let own_scope = scope.stable_signature();
    let global_scope =
        TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()).stable_signature();
    held.iter().any(|(held_role, held_concept, held_scope)| {
        held_role == role
            && held_concept == concept
            && (held_scope == &own_scope
                || held_scope == &global_scope
                || own_scope == global_scope)
    })
}

pub fn held_semantic_identities(
    sections: &[CandidateValidationSectionState],
    decisions: &[TruthAdjudicationCandidateDecision],
    registry: &BTreeMap<String, SourceGovernanceRecord>,
) -> BTreeSet<(String, String, String)> {
    let mut held = decisions
        .iter()
        .filter(|decision| {
            (decision.decision == "needs_hitl"
                || decision.adjudication_reason == "contradiction_block")
                && !independently_confirmed_weak_duplicate(decision, sections, decisions, registry)
        })
        .map(|decision| {
            (
                decision.role.clone(),
                decision.concept_canonical_key.clone(),
                held_scope_key(&decision.scope),
            )
        })
        .collect::<BTreeSet<_>>();
    for section in sections.iter().filter(|section| section.blocked_by_gate) {
        for candidate in &section.candidates {
            if candidate.epistemic_status != "rejected" {
                held.insert((
                    candidate.role.clone(),
                    candidate.concept_canonical_key.clone(),
                    held_scope_key(&candidate.scope),
                ));
            }
        }
    }
    held
}

pub fn apply_exact_review_rejection_veto(
    decisions: &mut [TruthAdjudicationCandidateDecision],
    rejected_ids: &BTreeSet<String>,
) -> BTreeSet<i64> {
    let held_slots = decisions
        .iter()
        .filter(|d| rejected_ids.contains(&d.rule_candidate_id))
        .map(|d| {
            (
                d.role.clone(),
                d.concept_canonical_key.clone(),
                d.scope.stable_signature(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut affected = BTreeSet::new();
    for decision in decisions {
        let slot = (
            decision.role.clone(),
            decision.concept_canonical_key.clone(),
            decision.scope.stable_signature(),
        );
        if rejected_ids.contains(&decision.rule_candidate_id) {
            decision.decision = "rejected".to_string();
            decision.publish_admissibility = "not_admissible".to_string();
            decision.verification_method = "truth_review_veto@1".to_string();
            decision.adjudication_reason = "human_rejected_exact_source_evidence".to_string();
            affected.insert(decision.section_id);
        } else if held_slots.contains(&slot) && decision.decision == "verified" {
            decision.decision = "needs_hitl".to_string();
            decision.publish_admissibility = "needs_hitl".to_string();
            decision.adjudication_reason =
                "reviewed_rejection_blocks_semantic_identity".to_string();
            affected.insert(decision.section_id);
        }
    }
    affected
}
