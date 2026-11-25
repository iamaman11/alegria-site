//! Typed test payloads for the real Truth extraction HTTP boundary.
//! Shared between E2E owners; never fake or bypass PostgreSQL Truth storage.

use std::collections::BTreeMap;

pub fn local_truth_response(payload: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-truth-test",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": payload.to_string()
            }
        }]
    })
}

pub fn local_truth_candidate_payload(
    raw_text: &str,
    role: &str,
    concept_canonical_key: &str,
    params: serde_json::Value,
    evidence_section_id: i64,
) -> serde_json::Value {
    serde_json::json!({
        "candidates": [{
            "role": role,
            "concept_canonical_key": concept_canonical_key,
            "raw_mention": raw_text,
            "params": params,
            "scope": {},
            "severity": "mandatory",
            "applies_to_profiles": [],
            "exceptions_raw": "",
            "conditions_raw": "",
            "alternatives": [],
            "modality_raw": "",
            "derivation_type": "direct",
            "is_numeric": true,
            "is_range": false,
            "is_incomplete": false,
            "confidence": 0.94,
            "evidence_section_id": evidence_section_id,
            "evidence_quote": raw_text,
            "span_start": 0,
            "span_end": raw_text.len(),
            "uncertainty_flags": []
        }]
    })
}

pub fn local_truth_env(endpoint: &str) -> BTreeMap<String, String> {
    crate::env_overrides::btree_env([
        (
            "SEO_TRUTH_LLM_LOCAL_ENDPOINT".to_string(),
            endpoint.to_string(),
        ),
        (
            "SEO_TRUTH_LLM_LOCAL_MODEL".to_string(),
            "stub-truth-extraction-model".to_string(),
        ),
    ])
}
