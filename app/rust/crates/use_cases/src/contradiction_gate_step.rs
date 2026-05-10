use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateInput {
    pub run_id: String,
    pub facts: Vec<FactAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactAssertion {
    pub subject_key: String,
    pub predicate_key: String,
    pub value_normalized: String,
    pub source_key: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictCase {
    pub subject_key: String,
    pub predicate_key: String,
    pub value_left: String,
    pub value_right: String,
    pub source_left: Option<String>,
    pub source_right: Option<String>,
    pub confidence_left: f32,
    pub confidence_right: f32,
    pub severity: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateOutput {
    pub is_blocked: bool,
    pub conflict_count: usize,
    pub conflicts: Vec<ConflictCase>,
    pub needs_hitl: bool,
    pub hitl_reason: Option<String>,
    pub decision: String,
}

pub fn execute(input: &ContradictionGateInput) -> ContradictionGateOutput {
    let mut first_seen: BTreeMap<(String, String), &FactAssertion> = BTreeMap::new();
    let mut conflicts: Vec<ConflictCase> = Vec::new();

    for fact in &input.facts {
        let key = (fact.subject_key.clone(), fact.predicate_key.clone());
        if let Some(prev) = first_seen.get(&key) {
            if prev.value_normalized != fact.value_normalized {
                let max_conf = prev.confidence.max(fact.confidence);
                let severity = if max_conf >= 0.90 {
                    "critical"
                } else if max_conf >= 0.75 {
                    "high"
                } else {
                    "medium"
                };
                conflicts.push(ConflictCase {
                    subject_key: fact.subject_key.clone(),
                    predicate_key: fact.predicate_key.clone(),
                    value_left: prev.value_normalized.clone(),
                    value_right: fact.value_normalized.clone(),
                    source_left: prev.source_key.clone(),
                    source_right: fact.source_key.clone(),
                    confidence_left: prev.confidence,
                    confidence_right: fact.confidence,
                    severity: severity.to_string(),
                    reason: "same (subject,predicate) has multiple normalized values".to_string(),
                });
            }
        } else {
            first_seen.insert(key, fact);
        }
    }

    let has_critical = conflicts.iter().any(|c| c.severity == "critical");
    let is_blocked = has_critical;
    let needs_hitl = !conflicts.is_empty();
    let decision = if is_blocked {
        "block_publish"
    } else if needs_hitl {
        "hitl_review"
    } else {
        "pass"
    };

    ContradictionGateOutput {
        is_blocked,
        conflict_count: conflicts.len(),
        conflicts,
        needs_hitl,
        hitl_reason: if needs_hitl {
            Some("contradictions_detected".to_string())
        } else {
            None
        },
        decision: decision.to_string(),
    }
}
