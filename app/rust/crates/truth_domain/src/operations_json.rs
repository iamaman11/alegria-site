//! JSON is examined only in this owner-local test boundary. Production Truth
//! operations stay typed and provider-free (no generic Value dispatch).

use super::*;
use operation_protocol::{ContractCatalog, IdempotencyPolicy};
use schemars::schema_for;
use serde_json::{json, Value};

use crate::operation_owner_schema_json::truth_pure_operation_owner_schemas;

#[test]
fn truth_operations_have_distinct_pure_versioned_contracts() {
    let mut registry = ContractCatalog::default();
    registry.register::<JudgeCompletenessV1>().unwrap();
    registry.register::<DetectContradictionsV1>().unwrap();
    for contract in [
        JudgeCompletenessV1::contract(),
        DetectContradictionsV1::contract(),
    ] {
        assert_eq!(contract.reference.revision, 1);
        assert_eq!(contract.owner.0, TRUTH_PURE_OWNER);
        assert_eq!(contract.inputs.len(), 1);
        assert_eq!(contract.outputs.len(), 1);
        assert!(contract.invalidates.is_empty());
        assert!(contract.effects.is_empty());
        assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
        assert!(!contract.fingerprint().unwrap().0.is_empty());
    }
    assert_ne!(
        JudgeCompletenessV1::contract().reference,
        DetectContradictionsV1::contract().reference
    );
}

#[test]
fn completeness_wire_rejects_unknown_and_ill_typed_fields() {
    let input = json!({
        "section_id":"s1",
        "raw_text":"Срок 15 дней, кроме студентов",
        "source_numeric_tokens":["15"],
        "extracted_numeric_tokens":[],
        "extracted_rule_keys":[]
    });
    let valid: CompletenessInputV1 = serde_json::from_value(input.clone()).unwrap();
    let assessed = JudgeCompletenessV1::evaluate(&valid);
    assert!(assessed.needs_hitl);
    assert!(assessed
        .missing_elements
        .iter()
        .any(|x| x.loss_type == "number"));

    let mut unexpected = input.clone();
    unexpected["parameter_not_defined_by_owner"] = json!(true);
    assert!(serde_json::from_value::<CompletenessInputV1>(unexpected).is_err());

    let mut wrong = input;
    wrong["source_numeric_tokens"] = json!([15]);
    assert!(serde_json::from_value::<CompletenessInputV1>(wrong).is_err());

    let encoded = serde_json::to_value(assessed).unwrap();
    let mut forged = encoded;
    forged["missing_elements"][0]["provider_receipt"] = json!("forged");
    assert!(serde_json::from_value::<CompletenessOutputV1>(forged).is_err());
}

#[test]
fn contradiction_wire_rejects_nested_unknown_fields() {
    let input = json!({
        "run_id":"r1",
        "facts":[{
            "subject_key":"fee",
            "predicate_key":"amount",
            "value_normalized":"80 EUR",
            "source_key":"official",
            "confidence":0.97
        }]
    });
    let valid: ContradictionInputV1 = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(DetectContradictionsV1::evaluate(&valid).decision, "pass");

    let mut unknown = input.clone();
    unknown["facts"][0]["untrusted_command"] = json!("mutate_truth");
    assert!(serde_json::from_value::<ContradictionInputV1>(unknown).is_err());
    let mut wrong = input;
    wrong["facts"][0]["confidence"] = json!("high");
    assert!(serde_json::from_value::<ContradictionInputV1>(wrong).is_err());
}

#[test]
fn truth_owner_dto_schemas_match_concrete_strict_structures() {
    for schema in [
        serde_json::to_value(schema_for!(CompletenessInputV1)).unwrap(),
        serde_json::to_value(schema_for!(ContradictionInputV1)).unwrap(),
    ] {
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], Value::Bool(false));
    }
    let contradiction = serde_json::to_value(schema_for!(ContradictionInputV1)).unwrap();
    let nested = &contradiction["$defs"]["FactAssertionV1"];
    assert_eq!(nested["additionalProperties"], Value::Bool(false));
    assert_eq!(nested["properties"]["confidence"]["type"], "number");
    let output = serde_json::to_value(schema_for!(ContradictionOutputV1)).unwrap();
    assert_eq!(
        output["$defs"]["ConflictCaseV1"]["additionalProperties"],
        false
    );
}

#[test]
fn pinned_owner_schema_is_generated_and_rejects_tampered_nested_field_type() {
    let trusted = truth_pure_operation_owner_schemas().unwrap();
    assert_eq!(trusted.len(), 2);
    for operation in [
        JudgeCompletenessV1::contract(),
        DetectContradictionsV1::contract(),
    ] {
        let pin = trusted.get(&operation.reference).unwrap();
        pin.validate(&operation).unwrap();
        assert_eq!(pin.schema.inputs.len(), 1);
        assert_eq!(pin.schema.outputs.len(), 1);
        assert_eq!(pin.schema.failure.descriptor_bytes.as_slice(), b"false");
        assert_eq!(
            pin.schema.v2_contract_fingerprint,
            operation.fingerprint().unwrap()
        );
        let mut mutated = pin.clone();
        mutated
            .schema
            .inputs
            .get_mut(&PortId::from("input"))
            .unwrap()
            .descriptor_bytes = b"true".to_vec();
        assert!(mutated.validate(&operation).is_err());
    }
}

#[test]
fn contradiction_owner_adapter_preserves_deterministic_verdict() {
    let facts = vec![
        FactAssertionV1 {
            subject_key: "fee".into(),
            predicate_key: "amount".into(),
            value_normalized: "80 EUR".into(),
            source_key: Some("official".into()),
            confidence: 0.95,
        },
        FactAssertionV1 {
            subject_key: "fee".into(),
            predicate_key: "amount".into(),
            value_normalized: "120 EUR".into(),
            source_key: Some("other".into()),
            confidence: 0.91,
        },
    ];
    let first = DetectContradictionsV1::evaluate(&ContradictionInputV1 {
        run_id: "r1".into(),
        facts: facts.clone(),
    });
    let second = DetectContradictionsV1::evaluate(&ContradictionInputV1 {
        run_id: "r1".into(),
        facts: facts.into_iter().rev().collect(),
    });
    assert!(first.is_blocked);
    assert!(first.needs_hitl);
    assert_eq!(first.conflict_count, 1);
    assert_eq!(first.conflicts[0].severity, "critical");
    assert_eq!(first.decision, second.decision);
    assert_eq!(
        first.conflicts[0].value_left,
        second.conflicts[0].value_left
    );
}
