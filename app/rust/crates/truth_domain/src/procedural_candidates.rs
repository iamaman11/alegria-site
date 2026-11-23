//! Mapping legacy procedural detections into typed Truth candidates.
//! No dependency on SEO step structs or on Temporal workflow payloads.

use crate::truth_candidates::TruthParamValue;

/// When manually grounded Truth was chosen, loose extraction proposals must
/// not compete as a second independent owner of factual candidates.
pub fn active_heuristic_truth_rules<T>(rules: &[T], manual_selected: bool) -> &[T] {
    if manual_selected {
        &[]
    } else {
        rules
    }
}

pub fn role_and_concept_for_rule(rule_key: &str) -> (&'static str, String) {
    match rule_key {
        "consular_fee" => ("FEE_ITEM", "consular_fee".to_string()),
        "processing_time" => ("TIMELINE_ITEM", "processing_time".to_string()),
        "passport_required" => ("DOCUMENT_REQUIRED", "passport".to_string()),
        "insurance_required" => ("DOCUMENT_REQUIRED", "medical_insurance".to_string()),
        _ => ("DOCUMENT_REQUIRED", rule_key.to_string()),
    }
}

fn extract_first_number(token: &str) -> Option<f64> {
    let normalized = token.replace(',', ".");
    let digits = normalized
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    digits.parse::<f64>().ok()
}

fn extract_first_days(token: &str) -> Option<i64> {
    let digits = token
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<i64>().ok()
}

pub fn params_for_rule(rule_key: &str, numeric_tokens: &[String]) -> TruthParamValue {
    match rule_key {
        "consular_fee" => {
            let amount = numeric_tokens
                .first()
                .and_then(|token| extract_first_number(token))
                .unwrap_or_default();
            TruthParamValue::object([
                ("amount", TruthParamValue::Decimal(amount)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ])
        }
        "processing_time" => {
            let days = numeric_tokens
                .first()
                .and_then(|token| extract_first_days(token))
                .unwrap_or_default();
            TruthParamValue::object([("days", TruthParamValue::Integer(days))])
        }
        "passport_required" => TruthParamValue::object([
            ("subtype", TruthParamValue::Text("passport".to_string())),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        "insurance_required" => TruthParamValue::object([
            (
                "subtype",
                TruthParamValue::Text("medical_insurance".to_string()),
            ),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        _ => TruthParamValue::object([("subtype", TruthParamValue::Text(rule_key.to_string()))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_truth_does_not_mix_with_loose_proposals() {
        let proposed = ["passport", "insurance", "fee"];
        assert!(active_heuristic_truth_rules(&proposed, true).is_empty());
        assert_eq!(
            active_heuristic_truth_rules(&proposed, false),
            proposed.as_slice()
        );
    }

    #[test]
    fn fee_mapping_preserves_decimal_comma_and_first_token() {
        let params = params_for_rule("consular_fee", &["35,50 EUR".to_string(), "80".to_string()]);
        assert_eq!(
            params.as_object().unwrap().get("amount"),
            Some(&TruthParamValue::Decimal(35.5))
        );
        assert_eq!(
            role_and_concept_for_rule("consular_fee"),
            ("FEE_ITEM", "consular_fee".to_string())
        );
    }

    #[test]
    fn processing_days_map_to_typed_integer() {
        let params = params_for_rule("processing_time", &["15 days".to_string()]);
        assert_eq!(
            params.as_object().unwrap().get("days"),
            Some(&TruthParamValue::Integer(15))
        );
        assert_eq!(
            role_and_concept_for_rule("insurance_required"),
            ("DOCUMENT_REQUIRED", "medical_insurance".to_string())
        );
    }
}
