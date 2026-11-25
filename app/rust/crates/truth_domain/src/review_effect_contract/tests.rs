use super::*;

#[test]
fn contract_retain_exact_v3_owner_effect_and_truth_invalidation() {
    let contract = ReconcileTruthReviewsContractV1::contract();
    assert_eq!(contract.reference.id.as_str(), OPERATION_ID);
    assert_eq!(contract.reference.revision, OPERATION_REVISION);
    assert_eq!(contract.owner.as_str(), OWNER_ID);
    assert_eq!(
        contract.effects,
        BTreeSet::from([EffectId::from(EFFECT_ID)])
    );
    assert_eq!(contract.idempotency, IdempotencyPolicy::Idempotent);
    assert!(contract
        .invalidates
        .contains(&ArtifactTypeId::from("truth.review_queue_snapshot")));
}

#[test]
fn owner_command_rejects_extra_fields_including_claimed_admission() {
    let mut bytes = serde_json::json!({
        "admission_id": Uuid::from_u128(1),
        "run_id": Uuid::from_u128(2),
        "scope": { "run_id": Uuid::from_u128(2).to_string(), "tenant_id": "t" },
        "context_key": "ctx",
        "operation_instance": "review",
        "plan_hash": "h",
    });
    assert!(serde_json::from_value::<ReconcileTruthReviewsInput>(bytes.clone()).is_ok());
    bytes["operator_override"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ReconcileTruthReviewsInput>(bytes).is_err());
}
