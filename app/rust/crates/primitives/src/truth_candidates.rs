use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TruthParamValue {
    Null,
    Bool(bool),
    Integer(i64),
    Decimal(f64),
    Text(String),
    List(Vec<TruthParamValue>),
    Object(BTreeMap<String, TruthParamValue>),
}

impl Default for TruthParamValue {
    fn default() -> Self {
        Self::Object(BTreeMap::new())
    }
}

impl TruthParamValue {
    pub fn object<K, I>(entries: I) -> Self
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, TruthParamValue)>,
    {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, TruthParamValue>> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    fn contains_numeric_token(&self) -> bool {
        match self {
            Self::Integer(_) | Self::Decimal(_) => true,
            Self::Text(value) => value.chars().any(|ch| ch.is_ascii_digit()),
            Self::List(values) => values.iter().any(Self::contains_numeric_token),
            Self::Object(values) => values.values().any(Self::contains_numeric_token),
            Self::Null | Self::Bool(_) => false,
        }
    }

    fn stable_signature(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(value) => format!("bool:{value}"),
            Self::Integer(value) => format!("int:{value}"),
            Self::Decimal(value) => format!("decimal:{value:.8}"),
            Self::Text(value) => format!("text:{}", stable_escape(value)),
            Self::List(values) => format!(
                "list:[{}]",
                values
                    .iter()
                    .map(Self::stable_signature)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Object(values) => format!(
                "object:{{{}}}",
                values
                    .iter()
                    .map(|(key, value)| format!(
                        "{}={}",
                        stable_escape(key),
                        value.stable_signature()
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

fn stable_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace('=', "\\=")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthCandidateRuntime {
    #[serde(default)]
    pub rule_candidate_id: String,
    #[serde(default)]
    pub context_key: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub concept_canonical_key: String,
    #[serde(default)]
    pub raw_mention: String,
    #[serde(default)]
    pub params: TruthParamValue,
    #[serde(default)]
    pub scope: TruthParamValue,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub derivation_type: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence_section_id: i64,
    #[serde(default)]
    pub evidence_quote: String,
    #[serde(default)]
    pub span_start: usize,
    #[serde(default)]
    pub span_end: usize,
    #[serde(default)]
    pub source_key: String,
    #[serde(default)]
    pub source_tier: String,
    #[serde(default)]
    pub source_snapshot_hash: String,
    #[serde(default)]
    pub is_numeric: bool,
    #[serde(default)]
    pub is_range: bool,
    #[serde(default)]
    pub is_incomplete: bool,
    #[serde(default)]
    pub uncertainty_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthCandidateIssue {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthCandidateValidationResult {
    #[serde(default)]
    pub epistemic_status: String,
    #[serde(default)]
    pub issues: Vec<TruthCandidateIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthStructuredCandidate {
    #[serde(default)]
    pub rule_candidate_id: String,
    #[serde(default)]
    pub context_key: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub concept_canonical_key: String,
    #[serde(default)]
    pub params: TruthParamValue,
    #[serde(default)]
    pub source_key: String,
    #[serde(default)]
    pub source_tier: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub freshness_class: String,
    #[serde(default)]
    pub completeness_class: String,
    #[serde(default)]
    pub evidence_quote: String,
    #[serde(default)]
    pub epistemic_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthAdjudicationDecision {
    #[serde(default)]
    pub rule_candidate_id: String,
    #[serde(default)]
    pub decision: String,
    #[serde(default)]
    pub publish_admissibility: String,
    #[serde(default)]
    pub verification_method: String,
    #[serde(default)]
    pub adjudication_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthAdjudicationResult {
    #[serde(default)]
    pub overall_status: String,
    #[serde(default)]
    pub verified_count: usize,
    #[serde(default)]
    pub needs_hitl_count: usize,
    #[serde(default)]
    pub rejected_count: usize,
    #[serde(default)]
    pub contradiction_count: usize,
    #[serde(default)]
    pub decisions: Vec<TruthAdjudicationDecision>,
}

pub fn validate_truth_candidate(
    candidate: &TruthCandidateRuntime,
    raw_text: &str,
) -> TruthCandidateValidationResult {
    let mut issues = Vec::new();
    let mut rejected = false;

    let reject = |issues: &mut Vec<TruthCandidateIssue>, code: &str, message: &str| {
        issues.push(TruthCandidateIssue {
            code: code.to_string(),
            severity: "error".to_string(),
            message: message.to_string(),
        });
    };
    let needs_hitl = |issues: &mut Vec<TruthCandidateIssue>, code: &str, message: &str| {
        issues.push(TruthCandidateIssue {
            code: code.to_string(),
            severity: "warning".to_string(),
            message: message.to_string(),
        });
    };

    if !is_allowed_role(&candidate.role) {
        reject(
            &mut issues,
            "invalid_role",
            "candidate role is outside the allowed registry",
        );
        rejected = true;
    }
    if candidate.concept_canonical_key.trim().is_empty() {
        reject(
            &mut issues,
            "missing_concept_key",
            "candidate is missing concept_canonical_key",
        );
        rejected = true;
    }
    if candidate.raw_mention.trim().is_empty() {
        reject(
            &mut issues,
            "missing_raw_mention",
            "candidate is missing raw_mention",
        );
        rejected = true;
    }
    if candidate.params.as_object().is_none() {
        reject(
            &mut issues,
            "params_not_object",
            "candidate params must be a JSON object",
        );
        rejected = true;
    }
    if candidate.scope.as_object().is_none() {
        reject(
            &mut issues,
            "scope_not_object",
            "candidate scope must be a JSON object",
        );
        rejected = true;
    }
    if candidate.evidence_section_id <= 0 {
        reject(
            &mut issues,
            "invalid_evidence_section_id",
            "candidate evidence_section_id must reference a persisted raw section",
        );
        rejected = true;
    }
    if candidate.evidence_quote.trim().is_empty() {
        reject(
            &mut issues,
            "missing_evidence_quote",
            "candidate is missing evidence_quote",
        );
        rejected = true;
    }
    if candidate.span_start >= candidate.span_end || candidate.span_end > raw_text.len() {
        reject(
            &mut issues,
            "invalid_evidence_span",
            "candidate evidence span is outside raw_text bounds",
        );
        rejected = true;
    } else if !raw_text.is_char_boundary(candidate.span_start)
        || !raw_text.is_char_boundary(candidate.span_end)
    {
        reject(
            &mut issues,
            "non_boundary_span",
            "candidate evidence span must align to UTF-8 character boundaries",
        );
        rejected = true;
    } else {
        let snippet = raw_text
            .get(candidate.span_start..candidate.span_end)
            .unwrap_or_default()
            .trim();
        if snippet.is_empty() {
            reject(
                &mut issues,
                "empty_evidence_span",
                "candidate evidence span resolves to empty text",
            );
            rejected = true;
        } else if snippet != candidate.evidence_quote.trim() {
            reject(
                &mut issues,
                "evidence_quote_mismatch",
                "candidate evidence_quote must exactly match the referenced raw_text span",
            );
            rejected = true;
        }
    }
    if !candidate.confidence.is_finite()
        || candidate.confidence <= 0.0
        || candidate.confidence > 1.0
    {
        reject(
            &mut issues,
            "invalid_confidence",
            "candidate confidence must be finite and within (0,1]",
        );
        rejected = true;
    }
    if candidate.source_snapshot_hash.trim().is_empty() {
        reject(
            &mut issues,
            "missing_source_snapshot_hash",
            "candidate must carry source_snapshot_hash",
        );
        rejected = true;
    }

    if rejected {
        return TruthCandidateValidationResult {
            epistemic_status: "rejected".to_string(),
            issues,
        };
    }

    if candidate.severity.trim().is_empty() || candidate.severity == "unknown" {
        needs_hitl(
            &mut issues,
            "unknown_severity",
            "candidate severity is unknown and needs deterministic review",
        );
    }
    if candidate.derivation_type.trim().is_empty() {
        needs_hitl(
            &mut issues,
            "missing_derivation_type",
            "candidate derivation_type is missing",
        );
    }
    if candidate.is_numeric && !has_numeric_params(&candidate.params) {
        needs_hitl(
            &mut issues,
            "missing_numeric_params",
            "numeric candidate is missing deterministic numeric params",
        );
    }
    if candidate.is_range && !has_range_params(&candidate.params) {
        needs_hitl(
            &mut issues,
            "missing_range_bounds",
            "range candidate is missing deterministic min/max style bounds",
        );
    }
    if candidate.is_incomplete {
        needs_hitl(
            &mut issues,
            "candidate_marked_incomplete",
            "candidate is explicitly marked incomplete",
        );
    }
    if candidate.uncertainty_flags.iter().any(|flag| {
        matches!(
            flag.as_str(),
            "freshness_ambiguous" | "temporal_ambiguous" | "stale_source"
        )
    }) {
        needs_hitl(
            &mut issues,
            "freshness_or_temporality_ambiguous",
            "candidate freshness or temporal scope is ambiguous",
        );
    }
    for issue in role_specific_completeness_checks(candidate) {
        issues.push(issue);
    }

    let epistemic_status = if issues.iter().any(|issue| issue.severity == "warning") {
        "needs_hitl"
    } else {
        "structured"
    };

    TruthCandidateValidationResult {
        epistemic_status: epistemic_status.to_string(),
        issues,
    }
}

pub fn adjudicate_truth_candidates(
    candidates: &[TruthStructuredCandidate],
) -> TruthAdjudicationResult {
    if candidates.is_empty() {
        return TruthAdjudicationResult {
            overall_status: "rejected".to_string(),
            rejected_count: 1,
            decisions: vec![TruthAdjudicationDecision {
                rule_candidate_id: String::new(),
                decision: "rejected".to_string(),
                publish_admissibility: "not_admissible".to_string(),
                verification_method: "truth_adjudication@1".to_string(),
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
                verification_method: "truth_adjudication@1".to_string(),
                adjudication_reason: "non_structured_input".to_string(),
            });
            continue;
        }
        if candidate.freshness_class != "fresh" || candidate.completeness_class != "complete" {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "needs_hitl".to_string(),
                publish_admissibility: "needs_hitl".to_string(),
                verification_method: "truth_adjudication@1".to_string(),
                adjudication_reason: "freshness_or_completeness_block".to_string(),
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

    let contradiction_count = contradiction_count(structured.as_slice());
    if contradiction_count > 0 {
        for candidate in structured {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "needs_hitl".to_string(),
                publish_admissibility: "needs_hitl".to_string(),
                verification_method: "truth_adjudication@1".to_string(),
                adjudication_reason: format!(
                    "contradictory_structured_candidates; source_tiers_evaluated_as_signals_only={}",
                    join_signal_tiers(&signal_tiers)
                ),
            });
        }
        let needs_hitl_count = decisions
            .iter()
            .filter(|d| d.decision == "needs_hitl")
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

    if structured.len() >= 2 {
        for candidate in structured {
            decisions.push(TruthAdjudicationDecision {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                decision: "verified".to_string(),
                publish_admissibility: "admissible".to_string(),
                verification_method: "cross_source_consensus@1".to_string(),
                adjudication_reason: format!(
                    "corroborated_structured_candidates; source_tiers_evaluated_as_signals_only={}",
                    join_signal_tiers(&signal_tiers)
                ),
            });
        }
        let verified_count = decisions
            .iter()
            .filter(|d| d.decision == "verified")
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

    for candidate in structured {
        decisions.push(TruthAdjudicationDecision {
            rule_candidate_id: candidate.rule_candidate_id.clone(),
            decision: "needs_hitl".to_string(),
            publish_admissibility: "needs_hitl".to_string(),
            verification_method: "truth_adjudication@1".to_string(),
            adjudication_reason: format!(
                "single_source_requires_corroboration; source_tiers_evaluated_as_signals_only={}",
                join_signal_tiers(&signal_tiers)
            ),
        });
    }

    let needs_hitl_count = decisions
        .iter()
        .filter(|d| d.decision == "needs_hitl")
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

fn is_allowed_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_uppercase().as_str(),
        "DOCUMENT_REQUIRED"
            | "ELIGIBILITY_RULE"
            | "FEE_ITEM"
            | "TIMELINE_ITEM"
            | "WHERE_TO_APPLY"
            | "APPOINTMENT_RULE"
            | "FORM_REQUIRED"
            | "STEP"
    )
}

fn role_specific_completeness_checks(
    candidate: &TruthCandidateRuntime,
) -> Vec<TruthCandidateIssue> {
    let mut issues = Vec::new();
    let params = candidate.params.as_object();
    let missing = |code: &str, message: &str| TruthCandidateIssue {
        code: code.to_string(),
        severity: "warning".to_string(),
        message: message.to_string(),
    };

    match candidate.role.as_str() {
        "FEE_ITEM" => {
            let has_amount = params
                .map(|p| {
                    p.contains_key("amount")
                        || p.contains_key("min_amount")
                        || p.contains_key("max_amount")
                })
                .unwrap_or(false);
            let has_currency = params.map(|p| p.contains_key("currency")).unwrap_or(false);
            if !has_amount || !has_currency {
                issues.push(missing(
                    "fee_item_incomplete",
                    "fee item must carry amount-style params and currency before it can be structured",
                ));
            }
        }
        "TIMELINE_ITEM" => {
            let has_timeline = params
                .map(|p| {
                    p.contains_key("days")
                        || p.contains_key("min_days")
                        || p.contains_key("max_days")
                        || p.contains_key("duration_days")
                })
                .unwrap_or(false);
            if !has_timeline {
                issues.push(missing(
                    "timeline_item_incomplete",
                    "timeline item must carry deterministic day bounds before it can be structured",
                ));
            }
        }
        "WHERE_TO_APPLY" => {
            let has_location = params
                .map(|p| {
                    p.contains_key("office_key")
                        || p.contains_key("location")
                        || p.contains_key("country_code")
                        || p.contains_key("url")
                })
                .unwrap_or(false);
            if !has_location {
                issues.push(missing(
                    "where_to_apply_incomplete",
                    "where-to-apply candidate must include office or location params",
                ));
            }
        }
        "APPOINTMENT_RULE" | "FORM_REQUIRED" | "ELIGIBILITY_RULE" => {
            let has_any_params = params.map(|p| !p.is_empty()).unwrap_or(false);
            if !has_any_params {
                issues.push(missing(
                    "role_specific_params_missing",
                    "this role requires deterministic params before it can be structured",
                ));
            }
        }
        _ => {}
    }

    issues
}

fn has_numeric_params(params: &TruthParamValue) -> bool {
    params
        .as_object()
        .map(|p| p.values().any(TruthParamValue::contains_numeric_token))
        .unwrap_or(false)
}

fn has_range_params(params: &TruthParamValue) -> bool {
    params
        .as_object()
        .map(|p| {
            (p.contains_key("min") && p.contains_key("max"))
                || (p.contains_key("min_amount") && p.contains_key("max_amount"))
                || (p.contains_key("min_days") && p.contains_key("max_days"))
                || (p.contains_key("from") && p.contains_key("to"))
        })
        .unwrap_or(false)
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
        candidate.params.stable_signature()
    )
}

fn join_signal_tiers(signal_tiers: &BTreeSet<String>) -> String {
    if signal_tiers.is_empty() {
        "none".to_string()
    } else {
        signal_tiers.iter().cloned().collect::<Vec<_>>().join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(entries: Vec<(&str, TruthParamValue)>) -> TruthParamValue {
        TruthParamValue::object(entries)
    }

    fn base_candidate() -> TruthCandidateRuntime {
        TruthCandidateRuntime {
            rule_candidate_id: "cand-1".to_string(),
            context_key: "ES|tourist||BY".to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            raw_mention: "Consular fee is 35 EUR".to_string(),
            params: obj(vec![
                ("amount", TruthParamValue::Integer(35)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ]),
            scope: obj(Vec::new()),
            severity: "mandatory".to_string(),
            derivation_type: "direct".to_string(),
            confidence: 0.91,
            evidence_section_id: 42,
            evidence_quote: "Consular fee is 35 EUR".to_string(),
            span_start: 0,
            span_end: 22,
            source_key: "https://example.gov/fee".to_string(),
            source_tier: "government".to_string(),
            source_snapshot_hash: "hash-1".to_string(),
            is_numeric: true,
            is_range: false,
            is_incomplete: false,
            uncertainty_flags: Vec::new(),
        }
    }

    #[test]
    fn valid_candidate_becomes_structured() {
        let raw_text = "Consular fee is 35 EUR";
        let result = validate_truth_candidate(&base_candidate(), raw_text);
        assert_eq!(result.epistemic_status, "structured");
        assert!(result.issues.is_empty());
    }

    #[test]
    fn malformed_candidate_becomes_rejected() {
        let mut candidate = base_candidate();
        candidate.evidence_quote = "Different text".to_string();
        let result = validate_truth_candidate(&candidate, "Consular fee is 35 EUR");
        assert_eq!(result.epistemic_status, "rejected");
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "evidence_quote_mismatch"));
    }

    #[test]
    fn incomplete_candidate_becomes_needs_hitl() {
        let mut candidate = base_candidate();
        candidate.params = obj(vec![("currency", TruthParamValue::Text("EUR".to_string()))]);
        let result = validate_truth_candidate(&candidate, "Consular fee is 35 EUR");
        assert_eq!(result.epistemic_status, "needs_hitl");
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "fee_item_incomplete"));
    }

    fn structured_candidate(
        id: &str,
        params: TruthParamValue,
        source_key: &str,
        source_tier: &str,
    ) -> TruthStructuredCandidate {
        TruthStructuredCandidate {
            rule_candidate_id: id.to_string(),
            context_key: "ES|tourist||BY".to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            params,
            source_key: source_key.to_string(),
            source_tier: source_tier.to_string(),
            confidence: 0.94,
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            evidence_quote: "Consular fee is 35 EUR".to_string(),
            epistemic_status: "structured".to_string(),
        }
    }

    #[test]
    fn corroborated_structured_candidates_become_verified() {
        let result = adjudicate_truth_candidates(&[
            structured_candidate(
                "cand-1",
                obj(vec![
                    ("amount", TruthParamValue::Integer(35)),
                    ("currency", TruthParamValue::Text("EUR".to_string())),
                ]),
                "a",
                "government",
            ),
            structured_candidate(
                "cand-2",
                obj(vec![
                    ("amount", TruthParamValue::Integer(35)),
                    ("currency", TruthParamValue::Text("EUR".to_string())),
                ]),
                "b",
                "editorial",
            ),
        ]);
        assert_eq!(result.overall_status, "verified");
        assert_eq!(result.verified_count, 2);
        assert!(result
            .decisions
            .iter()
            .all(|decision| decision.publish_admissibility == "admissible"));
    }

    #[test]
    fn contradictory_structured_candidates_need_hitl() {
        let result = adjudicate_truth_candidates(&[
            structured_candidate(
                "cand-1",
                obj(vec![
                    ("amount", TruthParamValue::Integer(35)),
                    ("currency", TruthParamValue::Text("EUR".to_string())),
                ]),
                "a",
                "government",
            ),
            structured_candidate(
                "cand-2",
                obj(vec![
                    ("amount", TruthParamValue::Integer(80)),
                    ("currency", TruthParamValue::Text("EUR".to_string())),
                ]),
                "b",
                "vfs",
            ),
        ]);
        assert_eq!(result.overall_status, "needs_hitl");
        assert!(result.contradiction_count > 0);
        assert!(result
            .decisions
            .iter()
            .all(|decision| decision.decision == "needs_hitl"));
    }

    #[test]
    fn non_structured_candidate_is_rejected_by_adjudication() {
        let mut candidate = structured_candidate(
            "cand-1",
            obj(vec![
                ("amount", TruthParamValue::Integer(35)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ]),
            "a",
            "government",
        );
        candidate.epistemic_status = "candidate".to_string();
        let result = adjudicate_truth_candidates(&[candidate]);
        assert_eq!(result.rejected_count, 1);
        assert_eq!(result.decisions[0].decision, "rejected");
    }
}
