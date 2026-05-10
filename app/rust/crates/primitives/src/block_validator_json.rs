use std::collections::HashSet;

use serde_json::Value;

use crate::block_validator::{
    validate_block_typed, BlockValidationInput, RequiredLink, TypedValidationEnvelope,
};

fn parse_string_vec(v: &Value) -> Vec<String> {
    match v {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_numbers_from_json(v: &Value) -> HashSet<String> {
    let mut out = HashSet::new();
    match v {
        Value::Number(n) => {
            out.insert(n.to_string());
        }
        Value::String(s) => {
            for token in s.split(|c: char| !c.is_ascii_digit() && c != '.' && c != ',') {
                let t = token.trim();
                if !t.is_empty() && t.chars().any(|c| c.is_ascii_digit()) {
                    out.insert(t.replace(',', "."));
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                out.extend(extract_numbers_from_json(item));
            }
        }
        Value::Object(map) => {
            for val in map.values() {
                out.extend(extract_numbers_from_json(val));
            }
        }
        _ => {}
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn validate_block_boundary_typed(
    html: &str,
    rules_json: &str,
    facts_json: &str,
    required_links_json: &str,
    required_keys_json: &str,
    used_rule_keys_json: &str,
    used_fact_keys_json: &str,
    block_key: &str,
    url_norm: &str,
) -> TypedValidationEnvelope {
    let rules = serde_json::from_str::<Value>(rules_json).unwrap_or(Value::Array(Vec::new()));
    let facts = serde_json::from_str::<Value>(facts_json).unwrap_or(Value::Array(Vec::new()));
    let req_links =
        serde_json::from_str::<Vec<RequiredLink>>(required_links_json).unwrap_or_default();
    let required_keys =
        serde_json::from_str::<Value>(required_keys_json).unwrap_or(Value::Array(Vec::new()));
    let used_rule_keys =
        serde_json::from_str::<Value>(used_rule_keys_json).unwrap_or(Value::Array(Vec::new()));
    let used_fact_keys =
        serde_json::from_str::<Value>(used_fact_keys_json).unwrap_or(Value::Array(Vec::new()));

    let mut allowed_numbers = extract_numbers_from_json(&rules);
    allowed_numbers.extend(extract_numbers_from_json(&facts));
    let input = BlockValidationInput {
        allowed_numbers,
        required_links: req_links,
        required_keys: parse_string_vec(&required_keys),
        used_rule_keys: parse_string_vec(&used_rule_keys),
        used_fact_keys: parse_string_vec(&used_fact_keys),
        block_key: block_key.to_string(),
        url_norm: url_norm.to_string(),
    };
    validate_block_typed(html, &input)
}

#[allow(clippy::too_many_arguments)]
pub fn validate_block_json(
    html: &str,
    rules_json: &str,
    facts_json: &str,
    required_links_json: &str,
    required_keys_json: &str,
    used_rule_keys_json: &str,
    used_fact_keys_json: &str,
    block_key: &str,
    url_norm: &str,
) -> String {
    let envelope = validate_block_boundary_typed(
        html,
        rules_json,
        facts_json,
        required_links_json,
        required_keys_json,
        used_rule_keys_json,
        used_fact_keys_json,
        block_key,
        url_norm,
    );
    serde_json::to_string(&envelope).unwrap_or_else(|_| "{\"diagnostics\":[]}".to_string())
}
