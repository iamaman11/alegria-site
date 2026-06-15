pub fn decode_extracted_payload(payload: Option<&ExecutionRunBlob>) -> ExtractedPayload {
    let Some(payload) = payload else {
        return ExtractedPayload::default();
    };
    if payload.payload_type != <ExtractedPayload as RuntimeProtoPayload>::payload_type() {
        return ExtractedPayload::default();
    }
    ExtractedPayload::decode_payload_bytes(&payload.payload_bytes).unwrap_or_default()
}

pub fn decode_verify_report(payload: Option<&ExecutionRunBlob>) -> Option<VerifyReport> {
    let payload = payload?;
    if payload.payload_type != <VerifyReport as RuntimeProtoPayload>::payload_type() {
        return None;
    }
    VerifyReport::decode_payload_bytes(&payload.payload_bytes).ok()
}

pub fn extracted_payload_rules_json(payload: &ExtractedPayload) -> String {
    let rules = payload
        .rule_instances
        .iter()
        .map(|r| {
            serde_json::json!({
                "rule_type_key": r.rule_type_key,
                "concept_key": r.concept_key,
                "role_type": r.role_type.as_str(),
                "params": r.params.as_json_value(),
                "status": r.status,
                "source_key": r.source_key,
                "condition_expr": r.condition_expr,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&rules).unwrap_or_else(|_| "[]".to_string())
}

pub fn extracted_payload_facts_json(payload: &ExtractedPayload) -> String {
    let facts = payload
        .facts
        .iter()
        .map(FactCandidateValue::as_json_value)
        .collect::<Vec<_>>();
    serde_json::to_string(&facts).unwrap_or_else(|_| "[]".to_string())
}

pub fn extract_sections_json(input_payload: Option<&ExecutionRunBlob>) -> String {
    let Some(payload) = input_payload else {
        return "[]".to_string();
    };
    if payload.payload_type != "alegria.temporal.v1.FactExtractionInputPayload" {
        return "[]".to_string();
    }
    decode_prost::<FactExtractionInputPayload>(&payload.payload_bytes, "FactExtractionInputPayload")
        .map(|v| v.sections_json)
        .unwrap_or_else(|_| "[]".to_string())
}

pub fn decode_validation_input(input_payload: Option<&ExecutionRunBlob>) -> ValidationInputRecord {
    let Some(payload) = input_payload else {
        return ValidationInputRecord::default();
    };
    if payload.payload_type != <ValidationInputPayload as RuntimeProtoPayload>::payload_type() {
        return ValidationInputRecord::default();
    }
    let payload = match ValidationInputPayload::decode_payload_bytes(&payload.payload_bytes) {
        Ok(v) => v,
        Err(_) => return ValidationInputRecord::default(),
    };
    ValidationInputRecord {
        required_links_json: String::from_utf8(payload.required_links_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        required_keys_json: String::from_utf8(payload.required_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        used_rule_keys_json: String::from_utf8(payload.used_rule_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        used_fact_keys_json: String::from_utf8(payload.used_fact_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        url_norm: payload.url_norm,
    }
}

pub fn json_string(v: Option<&Value>) -> String {
    serde_json::to_string(v.unwrap_or(&Value::Null)).unwrap_or_else(|_| "null".to_string())
}

pub fn decode_generation_result(
    generation_result: Option<&ExecutionRunBlob>,
) -> BTreeMap<String, String> {
    let Some(payload) = generation_result else {
        return BTreeMap::new();
    };
    if payload.payload_type != <BTreeMap<String, String> as RuntimeProtoPayload>::payload_type() {
        return BTreeMap::new();
    }
    <BTreeMap<String, String> as RuntimeProtoPayload>::decode_payload_bytes(&payload.payload_bytes)
        .unwrap_or_default()
}
