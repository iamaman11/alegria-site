include!("proto_runtime_payload_store/preamble.rs");
include!("proto_runtime_payload_store/rule_fact_mappers.rs");
include!("proto_runtime_payload_store/core_runtime_payload_impls.rs");
include!("proto_runtime_payload_store/registered_payload_impls.rs");
include!("proto_runtime_payload_store/decode_helpers.rs");

#[cfg(test)]
mod semantic_extraction_payload_tests {
    use super::RuntimeProtoPayload;
    use seo_domain::extraction::{
        editorial_topics, entity_spans, layer_routing, operational_entities,
    };

    #[test]
    fn all_eight_registered_identifiers_survive_the_domain_move() {
        macro_rules! assert_id {
            ($ty:ty, $id:literal) => {
                assert_eq!(<$ty as RuntimeProtoPayload>::payload_type(), $id);
            };
        }

        assert_id!(
            layer_routing::LayerRouterInput,
            "alegria.runtime.json.LayerRouterInput"
        );
        assert_id!(
            layer_routing::LayerRouterOutput,
            "alegria.runtime.json.LayerRouterOutput"
        );
        assert_id!(
            entity_spans::EntitySpanInput,
            "alegria.runtime.json.EntitySpanInput"
        );
        assert_id!(
            entity_spans::EntitySpanOutput,
            "alegria.runtime.json.EntitySpanOutput"
        );
        assert_id!(
            operational_entities::OperationalExtractionInput,
            "alegria.runtime.json.OperationalExtractionInput"
        );
        assert_id!(
            operational_entities::OperationalExtractionOutput,
            "alegria.runtime.json.OperationalExtractionOutput"
        );
        assert_id!(
            editorial_topics::EditorialExtractionInput,
            "alegria.runtime.json.EditorialExtractionInput"
        );
        assert_id!(
            editorial_topics::EditorialExtractionOutput,
            "alegria.runtime.json.EditorialExtractionOutput"
        );
    }

    #[test]
    fn router_and_mentions_keep_existing_json_shapes_and_decode_roundtrip() {
        let router = layer_routing::LayerRouterInput {
            section_id: "s1".into(),
            heading_text: "Title".into(),
            raw_text: "80 EUR".into(),
            source_tier: "official".into(),
            block_type: "document".into(),
        };
        let bytes = router.encode_payload_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "section_id": "s1",
                "heading_text": "Title",
                "raw_text": "80 EUR",
                "source_tier": "official",
                "block_type": "document"
            })
        );
        let decoded = layer_routing::LayerRouterInput::decode_payload_bytes(&bytes).unwrap();
        assert_eq!(decoded.block_type, "document");

        let routed = layer_routing::execute(&decoded);
        let json: serde_json::Value =
            serde_json::from_slice(&routed.encode_payload_bytes().unwrap()).unwrap();
        assert!(json["confidence"].is_number());
        assert!(json["needs_hitl"].is_boolean());
        assert!(json["layer_scores"]["procedural"].is_number());
        assert_eq!(json.as_object().unwrap().len(), 6);

        let input = entity_spans::EntitySpanInput {
            section_id: "s1".into(),
            raw_text: "80 EUR".into(),
        };
        let bytes = input.encode_payload_bytes().unwrap();
        let decoded = entity_spans::EntitySpanInput::decode_payload_bytes(&bytes).unwrap();
        let mentions = entity_spans::execute(&decoded);
        let json: serde_json::Value =
            serde_json::from_slice(&mentions.encode_payload_bytes().unwrap()).unwrap();
        let first = &json["mentions"][0];
        assert_eq!(first["entity_type"], "fee");
        assert_eq!(first["has_numeric"], true);
        assert_eq!(first["is_central"], true);
        assert!(first["confidence"].as_f64().unwrap() > 0.9);
        assert_eq!(first.as_object().unwrap().len(), 5);
    }

    #[test]
    fn editorial_and_operational_payloads_keep_typed_wire_shape() {
        let operational = operational_entities::OperationalExtractionInput {
            section_id: "s1".into(),
            raw_text: "График работы и запись".into(),
        };
        let decoded = operational_entities::OperationalExtractionInput::decode_payload_bytes(
            &operational.encode_payload_bytes().unwrap(),
        )
        .unwrap();
        let output = operational_entities::execute(&decoded);
        let json: serde_json::Value =
            serde_json::from_slice(&output.encode_payload_bytes().unwrap()).unwrap();
        assert_eq!(json["entities"].as_array().unwrap().len(), 2);
        assert_eq!(json["entities"][0]["entity_kind"], "office_schedule");
        assert!(json["entities"][0]["confidence"].is_number());

        let editorial = editorial_topics::EditorialExtractionInput {
            section_id: "s1".into(),
            raw_text: "Почему отказ?".into(),
        };
        let decoded = editorial_topics::EditorialExtractionInput::decode_payload_bytes(
            &editorial.encode_payload_bytes().unwrap(),
        )
        .unwrap();
        let output = editorial_topics::execute(&decoded);
        let json: serde_json::Value =
            serde_json::from_slice(&output.encode_payload_bytes().unwrap()).unwrap();
        assert_eq!(json["topics"].as_array().unwrap().len(), 2);
        assert_eq!(json["topics"][0]["topic_type"], "faq_topic");
        assert!(json["topics"][0]["topic_key_candidate"].is_string());
    }
}

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
