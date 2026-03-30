use std::collections::HashMap;

use crate::fact_verifier::{
    verify_fact_typed, verify_numeric_rule_typed, FactCandidate, NumericRuleCandidate,
    SourceRegistryEntry,
};

pub fn verify_fact_json(fact_key: &str, candidates_json: &str, registry_json: &str) -> String {
    let candidates: Vec<FactCandidate> = serde_json::from_str(candidates_json).unwrap_or_default();
    let registry: HashMap<String, SourceRegistryEntry> =
        serde_json::from_str(registry_json).unwrap_or_default();
    serde_json::to_string(&verify_fact_typed(fact_key, &candidates, &registry))
        .unwrap_or_else(|_| {
            "{\"resolution\":\"hitl_required\",\"value\":null,\"diagnostics\":[]}".to_string()
        })
}

pub fn verify_numeric_rule_json(rule_key: &str, candidates_json: &str, field_name: &str) -> String {
    let candidates: Vec<NumericRuleCandidate> =
        serde_json::from_str(candidates_json).unwrap_or_default();
    serde_json::to_string(&verify_numeric_rule_typed(rule_key, &candidates, field_name))
        .unwrap_or_else(|_| {
            "{\"resolution\":\"hitl_required\",\"range\":null,\"diagnostics\":[]}".to_string()
        })
}
