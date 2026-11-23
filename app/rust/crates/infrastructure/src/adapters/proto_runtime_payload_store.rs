include!("proto_runtime_payload_store/preamble.rs");
include!("proto_runtime_payload_store/rule_fact_mappers.rs");
include!("proto_runtime_payload_store/core_runtime_payload_impls.rs");
include!("proto_runtime_payload_store/registered_payload_impls.rs");
include!("proto_runtime_payload_store/decode_helpers.rs");

#[cfg(test)]
mod truth_review_payload_tests {
    use super::RuntimeProtoPayload;
    use truth_domain::{completeness, hitl_decision};

    #[test]
    fn completeness_payload_keeps_temporal_identifier_and_json_fields() {
        type Input = completeness::CompletenessJudgeInput;
        type Output = completeness::CompletenessJudgeOutput;
        assert_eq!(
            Input::payload_type(),
            "alegria.runtime.json.CompletenessJudgeInput"
        );
        assert_eq!(
            Output::payload_type(),
            "alegria.runtime.json.CompletenessJudgeOutput"
        );

        let input = Input {
            section_id: "s".to_string(),
            raw_text: "Fee: 80 EUR".to_string(),
            source_numeric_tokens: vec!["80".to_string()],
            extracted_numeric_tokens: Vec::new(),
            extracted_rule_keys: Vec::new(),
        };
        let encoded = input.encode_payload_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "section_id": "s",
                "raw_text": "Fee: 80 EUR",
                "source_numeric_tokens": ["80"],
                "extracted_numeric_tokens": [],
                "extracted_rule_keys": []
            })
        );

        let decoded = Input::decode_payload_bytes(&encoded).unwrap();
        let output = completeness::execute(&decoded);
        let result = output.encode_payload_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(json["needs_hitl"], true);
        assert_eq!(json["hitl_reason"], "completeness_gaps_detected");
        assert_eq!(json["missing_elements"][0]["action"], "add_numeric_fact");
        assert!(json.get("completeness_score").is_some());
    }

    #[test]
    fn hitl_payload_keeps_temporal_identifier_and_json_fields() {
        type Input = hitl_decision::HitlDecisionInput;
        type Output = hitl_decision::HitlDecisionOutput;
        assert_eq!(
            Input::payload_type(),
            "alegria.runtime.json.HitlDecisionInput"
        );
        assert_eq!(
            Output::payload_type(),
            "alegria.runtime.json.HitlDecisionOutput"
        );

        let input_json = serde_json::json!({
            "run_id": "r",
            "step_name": "s",
            "layer_confidence": 1.0,
            "completeness_score": 1.0,
            "unresolved_mappings": 0,
            "contradiction_blocked": false,
            "conflict_count": 0,
            "loss_count": 0
        });
        let json_bytes = serde_json::to_vec(&input_json).unwrap();
        let input = Input::decode_payload_bytes(&json_bytes).unwrap();
        let encoded = input.encode_payload_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(json, input_json);

        let output = hitl_decision::execute(&input);
        let result = output.encode_payload_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "requires_hitl": false,
                "route": "auto_approve",
                "task_type": null,
                "priority": 0,
                "reason": null
            })
        );
    }
}
