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
    // One representative per distinct normalized value, not the first record
    // encountered. Otherwise a low-confidence duplicate can hide a critical
    // conflict, and replaying the same facts in another order changes the
    // publish decision.
    let mut by_identity: BTreeMap<(String, String), BTreeMap<String, &FactAssertion>> =
        BTreeMap::new();
    let mut conflicts: Vec<ConflictCase> = Vec::new();

    for fact in &input.facts {
        let distinct_values = by_identity
            .entry((fact.subject_key.clone(), fact.predicate_key.clone()))
            .or_default();
        match distinct_values.get(&fact.value_normalized) {
            Some(previous)
                if previous.confidence.total_cmp(&fact.confidence).is_gt()
                    || (previous.confidence.total_cmp(&fact.confidence).is_eq()
                        && previous.source_key >= fact.source_key) => {}
            _ => {
                distinct_values.insert(fact.value_normalized.clone(), fact);
            }
        }
    }

    // BTreeMap order gives deterministic representative pairs and output.
    // A conflict against the canonical first distinct value is sufficient to
    // report every alternative without duplicating the same contradiction.
    for ((subject_key, predicate_key), distinct_values) in by_identity {
        let mut facts = distinct_values.values();
        let Some(reference) = facts.next() else {
            continue;
        };
        for alternative in facts {
            let max_conf = reference.confidence.max(alternative.confidence);
            let severity = if max_conf >= 0.90 {
                "critical"
            } else if max_conf >= 0.75 {
                "high"
            } else {
                "medium"
            };
            conflicts.push(ConflictCase {
                subject_key: subject_key.clone(),
                predicate_key: predicate_key.clone(),
                value_left: reference.value_normalized.clone(),
                value_right: alternative.value_normalized.clone(),
                source_left: reference.source_key.clone(),
                source_right: alternative.source_key.clone(),
                confidence_left: reference.confidence,
                confidence_right: alternative.confidence,
                severity: severity.to_string(),
                reason: "same (subject,predicate) has multiple normalized values".to_string(),
            });
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

    #[test]
    fn higher_confidence_duplicate_cannot_be_hidden_by_input_order() {
        let facts = vec![
            FactAssertion {
                subject_key: "fee".into(),
                predicate_key: "amount".into(),
                value_normalized: "80 EUR".into(),
                source_key: Some("source:old".into()),
                confidence: 0.50,
            },
            FactAssertion {
                subject_key: "fee".into(),
                predicate_key: "amount".into(),
                value_normalized: "80 EUR".into(),
                source_key: Some("source:official".into()),
                confidence: 0.99,
            },
            FactAssertion {
                subject_key: "fee".into(),
                predicate_key: "amount".into(),
                value_normalized: "120 EUR".into(),
                source_key: Some("source:other".into()),
                confidence: 0.60,
            },
        ];
        for permutation in [
            facts.clone(),
            vec![facts[2].clone(), facts[0].clone(), facts[1].clone()],
            facts.iter().rev().cloned().collect(),
        ] {
            let output = execute(&ContradictionGateInput {
                run_id: "run-replay".into(),
                facts: permutation,
            });
            assert!(output.is_blocked);
            assert_eq!(output.decision, "block_publish");
            assert_eq!(output.conflict_count, 1);
            let conflict = &output.conflicts[0];
            assert_eq!(conflict.severity, "critical");
            // Value order is stable lexical order, not "preferred evidence
            // always goes on the left". Verify the winning source is tied
            // to the *80 EUR value* regardless of pair orientation.
            let (source, confidence) = if conflict.value_left == "80 EUR" {
                (conflict.source_left.as_deref(), conflict.confidence_left)
            } else {
                assert_eq!(conflict.value_right, "80 EUR");
                (conflict.source_right.as_deref(), conflict.confidence_right)
            };
            assert_eq!(confidence, 0.99);
            assert_eq!(source, Some("source:official"));
        }
    }

    #[test]
    fn repeated_alternate_evidence_does_not_duplicate_conflicts() {
        let assertion = |value: &str, confidence: f32| FactAssertion {
            subject_key: "fee".into(),
            predicate_key: "amount".into(),
            value_normalized: value.into(),
            source_key: Some(format!("source:{confidence}")),
            confidence,
        };
        let output = execute(&ContradictionGateInput {
            run_id: "run-duplicates".into(),
            facts: vec![
                assertion("80 EUR", 0.80),
                assertion("120 EUR", 0.65),
                assertion("120 EUR", 0.95),
                assertion("150 EUR", 0.60),
            ],
        });
        assert_eq!(output.conflict_count, 2);
        assert!(output.is_blocked);
        // Canonical representative is lexicographically first, so an
        // alternative can be on either side. Every conflict against the
        // repeated 120 EUR value must use its strongest 0.95 evidence.
        let alternatives = output
            .conflicts
            .iter()
            .map(|conflict| {
                assert_eq!(conflict.severity, "critical");
                if conflict.value_left == "120 EUR" {
                    assert_eq!(conflict.confidence_left, 0.95);
                    conflict.value_right.as_str()
                } else {
                    assert_eq!(conflict.value_right, "120 EUR");
                    assert_eq!(conflict.confidence_right, 0.95);
                    conflict.value_left.as_str()
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            alternatives,
            std::collections::BTreeSet::from(["80 EUR", "150 EUR"])
        );
    }
}
