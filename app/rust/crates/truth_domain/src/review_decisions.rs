//! Deterministic review and semantic identity decisions.
//! Database reads and workflow orchestration remain in the caller.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::contradictions::{ContradictionGateOutput, FactAssertion};
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

/// The single domain decision for a section before source corroboration.
/// Workflow steps provide their evaluated facts; neither Temporal nor SQL
/// decides which already-known gate outcome controls publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionAdjudicationDisposition {
    SchemaBlocked,
    CandidateNeedsHitl,
    ContradictionRejected,
    ContradictionNeedsHitl,
    ResolutionNeedsHitl,
    Governed,
}

pub fn classify_adjudication_section(
    section: &CandidateValidationSectionState,
    resolution_decision: &str,
    resolution_needs_hitl: bool,
    contradiction: &ContradictionGateOutput,
) -> SectionAdjudicationDisposition {
    if section.blocked_by_gate {
        return SectionAdjudicationDisposition::SchemaBlocked;
    }
    if !section
        .candidates
        .iter()
        .any(|candidate| candidate.epistemic_status == "structured")
        && section
            .candidates
            .iter()
            .any(|candidate| candidate.epistemic_status == "needs_hitl")
    {
        return SectionAdjudicationDisposition::CandidateNeedsHitl;
    }
    if contradiction.is_blocked {
        return SectionAdjudicationDisposition::ContradictionRejected;
    }
    if contradiction.needs_hitl {
        return SectionAdjudicationDisposition::ContradictionNeedsHitl;
    }
    if resolution_needs_hitl || resolution_decision == "pause_for_hitl" {
        return SectionAdjudicationDisposition::ResolutionNeedsHitl;
    }
    SectionAdjudicationDisposition::Governed
}

impl SectionAdjudicationDisposition {
    pub fn fixed_status(self) -> Option<&'static str> {
        match self {
            Self::SchemaBlocked => Some("blocked"),
            Self::CandidateNeedsHitl | Self::ContradictionNeedsHitl | Self::ResolutionNeedsHitl => {
                Some("needs_hitl")
            }
            Self::ContradictionRejected => Some("rejected"),
            Self::Governed => None,
        }
    }
}

/// A section is publishable if it has any verified evidence; otherwise an
/// unresolved candidate takes precedence over a final rejection. Use this
/// same verdict before and after an exact human-review veto.
pub fn adjudication_section_status_from_counts(
    verified_count: usize,
    needs_hitl_count: usize,
) -> &'static str {
    if verified_count > 0 {
        "verified"
    } else if needs_hitl_count > 0 {
        "needs_hitl"
    } else {
        "rejected"
    }
}

/// Preserve the candidate's complete provenance while applying the domain's
/// fixed section verdict. Source-governance decisions use their own existing
/// adjudication port, below this section-level gate.
pub fn fixed_adjudication_candidate_decision(
    section: &CandidateValidationSectionState,
    candidate: &ValidatedTruthCandidateRecord,
    disposition: SectionAdjudicationDisposition,
) -> Option<TruthAdjudicationCandidateDecision> {
    use SectionAdjudicationDisposition as Disposition;
    let (decision, admissibility, reason, partial) = match disposition {
        Disposition::SchemaBlocked | Disposition::Governed => return None,
        Disposition::CandidateNeedsHitl => {
            if candidate.epistemic_status != "needs_hitl" {
                return None;
            }
            (
                "needs_hitl",
                "needs_hitl",
                non_structured_candidate_reason(candidate),
                false,
            )
        }
        Disposition::ContradictionRejected => (
            "rejected",
            "not_admissible",
            "contradiction_block".to_string(),
            false,
        ),
        Disposition::ContradictionNeedsHitl => (
            "needs_hitl",
            "needs_hitl",
            "contradiction_gate_requires_hitl".to_string(),
            true,
        ),
        Disposition::ResolutionNeedsHitl => (
            "needs_hitl",
            "needs_hitl",
            "resolution_loop_requires_hitl".to_string(),
            true,
        ),
    };
    Some(TruthAdjudicationCandidateDecision {
        section_id: section.section_id,
        page_id: section.page_id,
        rule_candidate_id: candidate.rule_candidate_id.clone(),
        role: candidate.role.clone(),
        concept_canonical_key: candidate.concept_canonical_key.clone(),
        params: candidate.params.clone(),
        scope: candidate.scope.clone(),
        source_key: candidate.source_key.clone(),
        source_tier: candidate.source_tier.clone(),
        confidence: candidate.confidence,
        freshness_class: candidate.freshness_class.clone(),
        completeness_class: if partial {
            "partial".to_string()
        } else {
            candidate.completeness_class.clone()
        },
        evidence_quote: candidate.evidence_quote.clone(),
        span_start: candidate.span_start,
        span_end: candidate.span_end,
        source_snapshot_hash: candidate.source_snapshot_hash.clone(),
        provider_key: candidate.provider_key.clone(),
        model_key: candidate.model_key.clone(),
        prompt_version: candidate.prompt_version.clone(),
        decision: decision.to_string(),
        publish_admissibility: admissibility.to_string(),
        verification_method: "truth_adjudication@1".to_string(),
        adjudication_reason: reason,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contradictions::{self, ContradictionGateInput};

    fn candidate(id: &str, amount: i64, applicant: &str) -> ValidatedTruthCandidateRecord {
        ValidatedTruthCandidateRecord {
            section_id: 17,
            page_id: 3,
            rule_candidate_id: id.to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            raw_mention: "published consular fee".to_string(),
            params: TruthParamValue::object([
                ("amount", TruthParamValue::Integer(amount)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ]),
            scope: TruthParamValue::object([(
                "applicant_profile",
                TruthParamValue::Text(applicant.to_string()),
            )]),
            source_key: format!("https://official.example/{id}"),
            source_tier: "government".to_string(),
            confidence: 0.98,
            evidence_quote: "published consular fee".to_string(),
            span_start: 0,
            span_end: "published consular fee".len(),
            source_snapshot_hash: "source-snapshot".to_string(),
            provider_key: "chat_manual".to_string(),
            model_key: "model".to_string(),
            prompt_version: "truth@1".to_string(),
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            epistemic_status: "structured".to_string(),
            issues: Vec::new(),
        }
    }

    fn section(candidates: Vec<ValidatedTruthCandidateRecord>) -> CandidateValidationSectionState {
        CandidateValidationSectionState {
            section_id: 17,
            page_id: 3,
            accepted_count: candidates.len(),
            candidates,
            needs_hitl_count: 0,
            rejected_count: 0,
            blocked_by_gate: false,
            status: "accepted".to_string(),
        }
    }

    fn contradiction(blocked: bool, needs_hitl: bool) -> ContradictionGateOutput {
        ContradictionGateOutput {
            is_blocked: blocked,
            conflict_count: usize::from(blocked || needs_hitl),
            conflicts: Vec::new(),
            needs_hitl,
            hitl_reason: needs_hitl.then(|| "contradictions_detected".to_string()),
            decision: if blocked {
                "block_publish"
            } else if needs_hitl {
                "hitl_review"
            } else {
                "pass"
            }
            .to_string(),
        }
    }

    #[test]
    fn fixed_adjudication_gate_precedence_preserves_reason_and_provenance() {
        use SectionAdjudicationDisposition as Disposition;
        let source = candidate("adult-fee", 35, "adult");
        let mut state = section(vec![source.clone()]);
        let conflict = contradiction(true, true);
        state.blocked_by_gate = true;
        assert_eq!(
            classify_adjudication_section(&state, "pause_for_hitl", true, &conflict),
            Disposition::SchemaBlocked
        );
        state.blocked_by_gate = false;
        assert_eq!(
            classify_adjudication_section(&state, "pause_for_hitl", true, &conflict),
            Disposition::ContradictionRejected
        );
        let rejected = fixed_adjudication_candidate_decision(
            &state,
            &source,
            Disposition::ContradictionRejected,
        )
        .unwrap();
        assert_eq!(rejected.decision, "rejected");
        assert_eq!(rejected.publish_admissibility, "not_admissible");
        assert_eq!(rejected.adjudication_reason, "contradiction_block");
        assert_eq!(rejected.source_snapshot_hash, source.source_snapshot_hash);
        assert_eq!(rejected.scope, source.scope);
        assert_eq!((rejected.section_id, rejected.page_id), (17, 3));

        let held = contradiction(false, true);
        assert_eq!(
            classify_adjudication_section(&state, "pause_for_hitl", true, &held),
            Disposition::ContradictionNeedsHitl
        );
        let held = fixed_adjudication_candidate_decision(
            &state,
            &source,
            Disposition::ContradictionNeedsHitl,
        )
        .unwrap();
        assert_eq!(held.adjudication_reason, "contradiction_gate_requires_hitl");
        assert_eq!(held.completeness_class, "partial");
        assert_eq!(
            Disposition::ContradictionNeedsHitl.fixed_status(),
            Some("needs_hitl")
        );

        assert_eq!(
            classify_adjudication_section(
                &state,
                "pause_for_hitl",
                false,
                &contradiction(false, false)
            ),
            Disposition::ResolutionNeedsHitl
        );
        assert_eq!(
            classify_adjudication_section(&state, "pass", false, &contradiction(false, false)),
            Disposition::Governed
        );
        assert_eq!(Disposition::Governed.fixed_status(), None);
        assert_eq!(adjudication_section_status_from_counts(1, 1), "verified");
        assert_eq!(adjudication_section_status_from_counts(0, 1), "needs_hitl");
        assert_eq!(adjudication_section_status_from_counts(0, 0), "rejected");
    }

    #[test]
    fn non_structured_hitl_candidate_does_not_invent_decisions_for_rejected_inputs() {
        use SectionAdjudicationDisposition as Disposition;
        let mut needs_hitl = candidate("needs-review", 35, "adult");
        needs_hitl.epistemic_status = "needs_hitl".to_string();
        let mut rejected = candidate("rejected", 90, "adult");
        rejected.epistemic_status = "rejected".to_string();
        let state = section(vec![needs_hitl.clone(), rejected.clone()]);
        let verdict =
            classify_adjudication_section(&state, "pass", false, &contradiction(false, false));
        assert_eq!(verdict, Disposition::CandidateNeedsHitl);
        let held = fixed_adjudication_candidate_decision(&state, &needs_hitl, verdict).unwrap();
        assert_eq!(held.decision, "needs_hitl");
        assert_eq!(
            held.adjudication_reason,
            "candidate_validation_requires_hitl"
        );
        assert!(fixed_adjudication_candidate_decision(&state, &rejected, verdict).is_none());
    }

    #[test]
    fn contradiction_and_human_veto_respect_explicit_applicant_scopes() {
        let adult_left = candidate("adult-left", 35, "adult");
        let adult_right = candidate("adult-right", 90, "adult");
        let child = candidate("minor", 0, "minor");
        let same_scope = section(vec![adult_left.clone(), adult_right]);
        let conflict = contradictions::execute(&ContradictionGateInput {
            run_id: "same-scope".to_string(),
            facts: validated_candidate_assertions(&same_scope),
        });
        assert!(conflict.is_blocked);
        assert!(conflict
            .conflicts
            .iter()
            .any(|item| item.predicate_key == "param:amount"));
        let distinct_scopes = section(vec![adult_left.clone(), child.clone()]);
        let independent = contradictions::execute(&ContradictionGateInput {
            run_id: "distinct-scopes".to_string(),
            facts: validated_candidate_assertions(&distinct_scopes),
        });
        assert!(!independent.is_blocked);
        assert_eq!(independent.conflict_count, 0);

        let mut decisions = vec![adult_left, child]
            .iter()
            .map(|candidate| {
                let mut decision = fixed_adjudication_candidate_decision(
                    &distinct_scopes,
                    candidate,
                    SectionAdjudicationDisposition::ResolutionNeedsHitl,
                )
                .unwrap();
                decision.decision = "verified".to_string();
                decision.publish_admissibility = "admissible".to_string();
                decision
            })
            .collect::<Vec<_>>();
        let rejected = BTreeSet::from(["adult-left".to_string()]);
        let changed = apply_exact_review_rejection_veto(&mut decisions, &rejected);
        assert_eq!(changed, BTreeSet::from([17]));
        assert_eq!(decisions[0].decision, "rejected");
        assert_eq!(
            decisions[0].adjudication_reason,
            "human_rejected_exact_source_evidence"
        );
        assert_eq!(decisions[1].decision, "verified");
    }
}
