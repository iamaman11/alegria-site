//! Adversarial owner-neutral ValidateOnly wire tests for V2 effect preflight.
//!
//! These tests do not claim business DTO validation: each owner must parse its
//! own concrete Input/Failure and independently attest the effectful History.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    operation_dag_parent_workflow_id, operation_dag_parent_workflow_id_v2,
    owner_validation::{
        owner_validation_child_workflow_id_v1, OwnerValidationRequestV1, OwnerValidationResultV1,
        OWNER_VALIDATION_SCHEMA_REVISION_V1,
    },
    AdmittedArtifactV1, ArtifactSchema, ArtifactTypeId, ContractHash, OperationId,
    OperationInstanceId, OperationOwnerId, OperationRef, PlanHash, PortId, RunScope,
    TemporalExecutionIdentityV4,
};

fn validated_request() -> OwnerValidationRequestV1 {
    let scope = RunScope {
        tenant_id: "tenant-a".into(),
        run_id: "business-run-a".into(),
    };
    let port = PortId::from("source");
    let schema = ArtifactSchema {
        id: ArtifactTypeId::from("validated.domain.document"),
        revision: 1,
    };
    OwnerValidationRequestV1 {
        schema_revision: OWNER_VALIDATION_SCHEMA_REVISION_V1,
        parent: TemporalExecutionIdentityV4 {
            workflow_id: operation_dag_parent_workflow_id_v2(&scope.run_id),
            run_id: "server-temporal-parent-run-id".into(),
        },
        plan_hash: PlanHash("immutable-sealed-plan-hash".into()),
        scope: scope.clone(),
        instance: OperationInstanceId::from("producer"),
        operation: OperationRef {
            id: OperationId("owner.validate.document".into()),
            revision: 1,
        },
        owner: OperationOwnerId("independent.owner".into()),
        contract_hash: ContractHash("fixed-owner-contract-fingerprint".into()),
        command_bytes: br#"{"schema_revision":1,"scope":"owner-only-input"}"#.to_vec(),
        input_schemas: BTreeMap::from([(port.clone(), schema.clone())]),
        external_inputs: BTreeMap::from([(
            port,
            AdmittedArtifactV1 {
                schema,
                scope,
                provenance: "independently-admitted-external-source".into(),
                payload: br#"{"document":"canonical"}"#.to_vec(),
            },
        )]),
        produced_input_ports: BTreeSet::new(),
    }
}

#[test]
fn validation_result_cannot_be_reused_with_different_input_or_owner() {
    let request = validated_request();
    let result = OwnerValidationResultV1::accepted(&request)
        .expect("shape-valid owner validation response for this test");
    result.verify_success(&request).unwrap();

    let mut replaced_bytes = request.clone();
    replaced_bytes.command_bytes = b"changed-wire-command".to_vec();
    assert!(result.verify_success(&replaced_bytes).is_err());

    let mut replaced_owner = request.clone();
    replaced_owner.owner = OperationOwnerId("foreign.owner".into());
    assert!(result.verify_success(&replaced_owner).is_err());

    let mut replaced_source = request.clone();
    replaced_source
        .external_inputs
        .get_mut(&PortId::from("source"))
        .unwrap()
        .payload = br#"{"document":"forged"}"#.to_vec();
    assert!(result.verify_success(&replaced_source).is_err());

    let mut replaced_contract = request;
    replaced_contract.contract_hash = ContractHash("foreign-contract".into());
    assert!(result.verify_success(&replaced_contract).is_err());
}

#[test]
fn validation_request_rejects_cross_version_parent_and_cross_tenant_artifact() {
    let mut request = validated_request();
    request.parent.workflow_id = operation_dag_parent_workflow_id(&request.scope.run_id);
    assert!(request.validate_shape().is_err(), "V1 parent cannot mint V2 ValidateOnly evidence");

    let mut request = validated_request();
    request
        .external_inputs
        .get_mut(&PortId::from("source"))
        .unwrap()
        .scope
        .tenant_id = "foreign-tenant".into();
    assert!(request.validate_shape().is_err());

    let mut request = validated_request();
    request.schema_revision += 1;
    assert!(request.validate_shape().is_err(), "future wire revision must be explicit");

    let mut request = validated_request();
    request.produced_input_ports.insert(PortId::from("source"));
    assert!(request.validate_shape().is_err(), "port cannot be both External and Produced");
}

#[test]
fn forged_negative_or_stale_validation_result_never_authorizes_effect() {
    let request = validated_request();
    let mut result = OwnerValidationResultV1::accepted(&request).unwrap();
    result.accepted = false;
    assert!(result.verify_success(&request).is_err());

    let mut result = OwnerValidationResultV1::accepted(&request).unwrap();
    result.request_hash = "self-asserted-hash-without-real-validation".into();
    assert!(result.verify_success(&request).is_err());

    let mut result = OwnerValidationResultV1::accepted(&request).unwrap();
    result.schema_revision += 1;
    assert!(result.verify_success(&request).is_err());
}

#[test]
fn validation_child_identity_is_scoped_to_actual_temporal_parent_run() {
    let first = validated_request();
    let mut restarted = first.clone();
    restarted.parent.run_id = "new-server-issued-temporal-parent-run".into();
    let first_id = owner_validation_child_workflow_id_v1(
        &first.parent, &first.plan_hash, &first.instance,
    );
    let restarted_id = owner_validation_child_workflow_id_v1(
        &restarted.parent, &restarted.plan_hash, &restarted.instance,
    );
    assert_ne!(first_id, restarted_id,
        "ValidateOnly child history must not be reused across parent Run IDs");

    let mut foreign_operation = first.clone();
    foreign_operation.instance = OperationInstanceId::from("another-operation");
    let foreign_id = owner_validation_child_workflow_id_v1(
        &foreign_operation.parent, &foreign_operation.plan_hash, &foreign_operation.instance,
    );
    assert_ne!(first_id, foreign_id,
        "distinct DAG nodes must have distinct immutable ValidateOnly child identities");
}
