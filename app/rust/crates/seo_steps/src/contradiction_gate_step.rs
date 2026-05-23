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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_conflict_blocks_publish() {
        let output = execute(&ContradictionGateInput {
            run_id: "run".to_string(),
            facts: vec![
                FactAssertion {
                    subject_key: "fee".to_string(),
                    predicate_key: "amount".to_string(),
                    value_normalized: "80 EUR".to_string(),
                    source_key: Some("source:official".to_string()),
                    confidence: 0.95,
                },
                FactAssertion {
                    subject_key: "fee".to_string(),
                    predicate_key: "amount".to_string(),
                    value_normalized: "120 EUR".to_string(),
                    source_key: Some("source:competitor".to_string()),
                    confidence: 0.92,
                },
            ],
        });

        assert!(output.is_blocked);
        assert!(output.needs_hitl);
        assert_eq!(output.decision, "block_publish");
        assert_eq!(output.conflict_count, 1);
        assert_eq!(output.conflicts[0].severity, "critical");
    }

    #[test]
    fn lower_confidence_conflict_goes_to_hitl() {
        let output = execute(&ContradictionGateInput {
            run_id: "run".to_string(),
            facts: vec![
                FactAssertion {
                    subject_key: "timeline".to_string(),
                    predicate_key: "days".to_string(),
                    value_normalized: "15".to_string(),
                    source_key: Some("source:official".to_string()),
                    confidence: 0.80,
                },
                FactAssertion {
                    subject_key: "timeline".to_string(),
                    predicate_key: "days".to_string(),
                    value_normalized: "20".to_string(),
                    source_key: Some("source:blog".to_string()),
                    confidence: 0.70,
                },
            ],
        });

        assert!(!output.is_blocked);
        assert!(output.needs_hitl);
        assert_eq!(output.decision, "hitl_review");
        assert_eq!(output.conflicts[0].severity, "high");
    }

    #[test]
    fn consistent_facts_pass() {
        let output = execute(&ContradictionGateInput {
            run_id: "run".to_string(),
            facts: vec![
                FactAssertion {
                    subject_key: "doc".to_string(),
                    predicate_key: "required".to_string(),
                    value_normalized: "passport".to_string(),
                    source_key: Some("source:official".to_string()),
                    confidence: 0.99,
                },
                FactAssertion {
                    subject_key: "doc".to_string(),
                    predicate_key: "required".to_string(),
                    value_normalized: "passport".to_string(),
                    source_key: Some("source:official-2".to_string()),
                    confidence: 0.96,
                },
            ],
        });

        assert!(!output.is_blocked);
        assert!(!output.needs_hitl);
        assert_eq!(output.decision, "pass");
        assert!(output.conflicts.is_empty());
    }

    #[test]
    fn contradiction_verdict_is_order_invariant() {
        let facts = vec![
            FactAssertion {
                subject_key: "fee".to_string(),
                predicate_key: "amount".to_string(),
                value_normalized: "80 EUR".to_string(),
                source_key: Some("source:official".to_string()),
                confidence: 0.95,
            },
            FactAssertion {
                subject_key: "fee".to_string(),
                predicate_key: "amount".to_string(),
                value_normalized: "120 EUR".to_string(),
                source_key: Some("source:official-2".to_string()),
                confidence: 0.96,
            },
        ];
        let forward = execute(&ContradictionGateInput {
            run_id: "run-1".to_string(),
            facts: facts.clone(),
        });
        let reverse = execute(&ContradictionGateInput {
            run_id: "run-1".to_string(),
            facts: facts.into_iter().rev().collect(),
        });
        assert_eq!(forward.decision, reverse.decision);
        assert_eq!(forward.conflict_count, reverse.conflict_count);
        assert_eq!(forward.conflicts[0].severity, reverse.conflicts[0].severity);
    }
}
