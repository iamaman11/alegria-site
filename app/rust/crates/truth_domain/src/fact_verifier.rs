use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerificationDiagnostic {
    pub severity: String,
    pub gate: String,
    pub message: String,
    #[serde(default)]
    pub context: VerificationContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationContext {
    None,
    CandidateSourceKeys(Vec<String>),
}

impl Default for VerificationContext {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FactCandidate {
    #[serde(default)]
    pub source_key: String,
    #[serde(default)]
    pub fact_value: TypedFactValue,
    #[serde(default)]
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypedFactValue {
    Null,
    Integer(i64),
    Decimal(f64),
    Text(String),
    Boolean(bool),
}

impl Default for TypedFactValue {
    fn default() -> Self {
        Self::Null
    }
}

impl TypedFactValue {
    pub fn stable_text(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Integer(v) => v.to_string(),
            Self::Decimal(v) => v.to_string(),
            Self::Text(v) => v.clone(),
            Self::Boolean(v) => v.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceRegistryEntry {
    #[serde(default)]
    pub source_type: String,
    #[serde(default)]
    pub trust_level: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FactVerificationResult {
    pub resolution: String,
    #[serde(default)]
    pub value: TypedFactValue,
    #[serde(default)]
    pub diagnostics: Vec<VerificationDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NumericRange {
    pub min: f64,
    pub max: f64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NumericRuleCandidate {
    #[serde(default)]
    pub params: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NumericVerificationResult {
    pub resolution: String,
    #[serde(default)]
    pub range: Option<NumericRange>,
    #[serde(default)]
    pub diagnostics: Vec<VerificationDiagnostic>,
}

pub fn verify_fact_typed(
    fact_key: &str,
    candidates: &[FactCandidate],
    registry: &HashMap<String, SourceRegistryEntry>,
) -> FactVerificationResult {
    if candidates.is_empty() {
        return FactVerificationResult {
            resolution: "hitl_required".to_string(),
            value: TypedFactValue::Null,
            diagnostics: Vec::new(),
        };
    }

    let get_trust = |c: &FactCandidate| -> i64 {
        registry
            .get(&c.source_key)
            .map(|s| s.trust_level)
            .unwrap_or(0)
    };

    let get_source_type = |c: &FactCandidate| -> &str {
        registry
            .get(&c.source_key)
            .map(|s| s.source_type.as_str())
            .unwrap_or("")
    };

    let auth_sources: Vec<&FactCandidate> =
        candidates.iter().filter(|c| get_trust(c) >= 4).collect();
    if !auth_sources.is_empty() {
        let best = auth_sources
            .iter()
            .max_by(|a, b| {
                a.extraction_confidence
                    .partial_cmp(&b.extraction_confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("auth_sources non-empty");
        return FactVerificationResult {
            resolution: "gov_wins".to_string(),
            value: best.fact_value.clone(),
            diagnostics: Vec::new(),
        };
    }

    let agency_sources: Vec<&FactCandidate> = candidates
        .iter()
        .filter(|c| matches!(get_source_type(c), "niche_agency" | "editorial"))
        .collect();

    if !agency_sources.is_empty() {
        let values_str: Vec<String> = agency_sources
            .iter()
            .map(|c| c.fact_value.stable_text())
            .collect();

        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &values_str {
            *counts.entry(s.as_str()).or_insert(0) += 1;
        }
        if let Some((most_common, count)) = counts.into_iter().max_by_key(|(_, c)| *c) {
            if count * 10 >= values_str.len() * 6 {
                let result_val = agency_sources
                    .iter()
                    .find(|c| c.fact_value.stable_text() == most_common)
                    .map(|c| c.fact_value.clone())
                    .unwrap_or(TypedFactValue::Null);
                return FactVerificationResult {
                    resolution: "consensus".to_string(),
                    value: result_val,
                    diagnostics: Vec::new(),
                };
            }
        }
    }

    let candidate_keys: Vec<String> = candidates.iter().map(|c| c.source_key.clone()).collect();
    FactVerificationResult {
        resolution: "hitl_required".to_string(),
        value: TypedFactValue::Null,
        diagnostics: vec![VerificationDiagnostic {
            severity: "warning".to_string(),
            gate: "fact_verifier".to_string(),
            message: format!("Conflict detected for {}. Sources do not agree.", fact_key),
            context: VerificationContext::CandidateSourceKeys(candidate_keys),
        }],
    }
}

pub fn verify_numeric_rule_typed(
    _rule_key: &str,
    candidates: &[NumericRuleCandidate],
    field_name: &str,
) -> NumericVerificationResult {
    let values: Vec<f64> = candidates
        .iter()
        .filter_map(|c| c.params.get(field_name).copied())
        .collect();

    if values.is_empty() {
        return NumericVerificationResult {
            resolution: "hitl_required".to_string(),
            range: None,
            diagnostics: Vec::new(),
        };
    }

    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    NumericVerificationResult {
        resolution: "range_merged".to_string(),
        range: Some(NumericRange {
            min,
            max,
            count: values.len(),
        }),
        diagnostics: Vec::new(),
    }
}

// Preserved from the Truth boundary stack (#45/#46) when #44 established
// the canonical owner. These are behavioral checks, not source-string gates.
#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(source: &str, value: &str) -> FactCandidate {
        FactCandidate {
            source_key: source.to_string(),
            fact_value: TypedFactValue::Text(value.to_string()),
            extraction_confidence: 0.95,
        }
    }

    #[test]
    fn missing_evidence_requires_human_review() {
        let result = verify_fact_typed("visa_fee", &[], &HashMap::new());
        assert_eq!(result.resolution, "hitl_required");
        assert!(matches!(result.value, TypedFactValue::Null));
    }

    #[test]
    fn authoritative_source_wins_over_conflicting_editorial_candidate() {
        let sources = HashMap::from([
            (
                "official".to_string(),
                SourceRegistryEntry {
                    source_type: "government".to_string(),
                    trust_level: 4,
                },
            ),
            (
                "blog".to_string(),
                SourceRegistryEntry {
                    source_type: "editorial".to_string(),
                    trust_level: 1,
                },
            ),
        ]);
        let result = verify_fact_typed(
            "visa_fee",
            &[candidate("blog", "90 EUR"), candidate("official", "80 EUR")],
            &sources,
        );
        assert_eq!(result.resolution, "gov_wins");
        assert!(matches!(result.value, TypedFactValue::Text(ref v) if v == "80 EUR"));
    }

    #[test]
    fn conflicting_untrusted_sources_preserve_human_review_and_evidence_keys() {
        let result = verify_fact_typed(
            "visa_fee",
            &[
                candidate("source-a", "80 EUR"),
                candidate("source-b", "90 EUR"),
            ],
            &HashMap::new(),
        );
        assert_eq!(result.resolution, "hitl_required");
        assert_eq!(result.diagnostics.len(), 1);
        match &result.diagnostics[0].context {
            VerificationContext::CandidateSourceKeys(keys) => {
                assert_eq!(keys, &["source-a".to_string(), "source-b".to_string()])
            }
            VerificationContext::None => panic!("candidate source keys must be retained"),
        }
    }

    #[test]
    fn numeric_rule_merges_only_present_values() {
        let candidates = [
            NumericRuleCandidate {
                params: HashMap::from([("days".to_string(), 15.0)]),
            },
            NumericRuleCandidate {
                params: HashMap::from([("days".to_string(), 30.0)]),
            },
            NumericRuleCandidate {
                params: HashMap::new(),
            },
        ];
        let result = verify_numeric_rule_typed("timeline", &candidates, "days");
        assert_eq!(result.resolution, "range_merged");
        let range = result.range.expect("two numeric values");
        assert_eq!((range.min, range.max, range.count), (15.0, 30.0, 2));
    }
}
