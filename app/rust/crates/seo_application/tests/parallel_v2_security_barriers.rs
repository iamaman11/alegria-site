//! Cross-version server-History identity and Produced-provenance regression.
//!
//! The V1 parent has shipped Temporal Histories; a new parallel V2 parent
//! MUST NOT silently inherit V1's child Workflow-ID namespace. These tests
//! exercise only the stable provider-neutral typed boundary. No provider
//! service or external side effect is required.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    canonical_owner_dispatch_input_bytes, canonical_owner_result_hash, checked_execution_key,
    operation_dag_parent_workflow_id, operation_dag_parent_workflow_id_v2, owner_child_workflow_id,
    owner_child_workflow_id_v2, owner_content_hash, AdmittedArtifactV1, ArtifactBinding,
    ArtifactId, ArtifactSchema, ArtifactTypeId, BoundArtifactV1, CheckedDispatch,
    IdempotencyPolicy, OperationId, OperationInstanceId, OperationOwnerId, OperationRef,
    OwnerInvocationV1, OwnerResultV1, PlanHash, PortId, ProducedArtifactEvidenceV1, RunScope,
    TemporalExecutionIdentityV4, OWNER_INVOCATION_SCHEMA_REVISION,
};

fn scope() -> RunScope {
    RunScope {
        tenant_id: "tenant-security".into(),
        run_id: "business-run-security".into(),
    }
}

fn dispatch(name: &str) -> CheckedDispatch {
    CheckedDispatch {
        plan_hash: PlanHash("immutable-security-plan".into()),
        scope: scope(),
        instance: OperationInstanceId::from(name),
        operation: OperationRef {
            id: OperationId(format!("test.{name}")),
            revision: 1,
        },
        owner: OperationOwnerId(format!("test.{name}.owner")),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
        input_hash: String::new(),
        execution_key: String::new(),
    }
}

fn seal(invocation: &mut OwnerInvocationV1) {
    let bytes = canonical_owner_dispatch_input_bytes(
        &invocation.bound_inputs,
        &invocation.command_bytes,
    )
    .expect("canonical checked owner input");
    let hash = owner_content_hash(&bytes);
    invocation.checked_dispatch.input_hash = hash.clone();
    invocation.checked_dispatch.execution_key =
        checked_execution_key(&invocation.checked_dispatch, &hash).expect("checked execution key");
}

fn producer_and_consumer(v2: bool) -> (OwnerInvocationV1, OwnerResultV1, OwnerInvocationV1) {
    let parent_workflow_id = if v2 {
        operation_dag_parent_workflow_id_v2(&scope().run_id)
    } else {
        operation_dag_parent_workflow_id(&scope().run_id)
    };
    let parent = TemporalExecutionIdentityV4 {
        workflow_id: parent_workflow_id,
        run_id: "actual-server-issued-parent-run-id".into(),
    };
    let mut producer = OwnerInvocationV1 {
        schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
        parent: parent.clone(),
        checked_dispatch: dispatch("producer"),
        command_bytes: b"owner-typed-producer-command".to_vec(),
        bound_inputs: BTreeMap::new(),
    };
    seal(&mut producer);
    let value = AdmittedArtifactV1 {
        schema: ArtifactSchema {
            id: ArtifactTypeId::from("projection.vector_projection"),
            revision: 1,
        },
        scope: scope(),
        provenance: "independent-source-history-and-durable-receipt".into(),
        payload: b"typed-vector-projection-from-upstream-child".to_vec(),
    };
    let output_port = PortId::from("vector_projection");
    let result = OwnerResultV1 {
        dispatch_execution_key: producer.checked_dispatch.execution_key.clone(),
        outputs: BTreeMap::from([(output_port.clone(), value.clone())]),
        receipt_bytes: b"immutable-upstream-owner-receipt".to_vec(),
    };
    let source_child_id = if v2 {
        owner_child_workflow_id_v2(&producer.checked_dispatch.execution_key)
    } else {
        owner_child_workflow_id(&producer.checked_dispatch.execution_key)
    };
    let artifact_id = ArtifactId::from("producer-artifact");
    let evidence = ProducedArtifactEvidenceV1 {
        producer: producer.checked_dispatch.instance.clone(),
        port: output_port.clone(),
        artifact_id: artifact_id.clone(),
        dispatch_execution_key: producer.checked_dispatch.execution_key.clone(),
        child_workflow_id: source_child_id,
        result_hash: canonical_owner_result_hash(&result).expect("typed result hash"),
        receipt_hash: owner_content_hash(&result.receipt_bytes),
        payload_hash: owner_content_hash(&value.payload),
    };
    let mut consumer = OwnerInvocationV1 {
        schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
        parent,
        checked_dispatch: dispatch("consumer"),
        command_bytes: b"owner-typed-consumer-command".to_vec(),
        bound_inputs: BTreeMap::from([(
            output_port,
            BoundArtifactV1 {
                artifact_id: artifact_id.clone(),
                source: ArtifactBinding::Produced {
                    producer: producer.checked_dispatch.instance.clone(),
                    port: PortId::from("vector_projection"),
                    artifact: artifact_id,
                },
                artifact: value,
                produced_evidence: Some(evidence),
            },
        )]),
    };
    seal(&mut consumer);
    (producer, result, consumer)
}

#[test]
fn v2_child_identity_must_not_reuse_frozen_v1_history_namespace() {
    let key = "same-checked-dispatch-key-on-the-same-business-run";
    assert_ne!(
        operation_dag_parent_workflow_id("business-run"),
        operation_dag_parent_workflow_id_v2("business-run"),
    );
    assert_ne!(
        owner_child_workflow_id(key),
        owner_child_workflow_id_v2(key),
        "V1 and V2 parent Histories must not race for the same Child Workflow ID",
    );
}

#[test]
fn observed_produced_child_completion_is_version_scoped() {
    let (v1_source, v1_result, v1_consumer) = producer_and_consumer(false);
    v1_consumer
        .verify_produced_input_from_observed_completion(
            &PortId::from("vector_projection"),
            &v1_source,
            &v1_result,
        )
        .expect("frozen V1 provenance remains valid under V1 History identity");

    let (v2_source, v2_result, v2_consumer) = producer_and_consumer(true);
    v2_consumer
        .verify_produced_input_from_observed_completion(
            &PortId::from("vector_projection"),
            &v2_source,
            &v2_result,
        )
        .expect("new V2 provenance matches only the new V2 child History identity");

    let mut forged = v2_consumer.clone();
    forged
        .bound_inputs
        .get_mut(&PortId::from("vector_projection"))
        .unwrap()
        .produced_evidence
        .as_mut()
        .unwrap()
        .child_workflow_id = owner_child_workflow_id(&v2_source.checked_dispatch.execution_key);
    assert!(
        forged
            .verify_produced_input_from_observed_completion(
                &PortId::from("vector_projection"),
                &v2_source,
                &v2_result,
            )
            .is_err(),
        "a forged V1 child History must never authorize a V2 Produced consumer",
    );

    let mut foreign_parent = v2_consumer;
    foreign_parent.parent.workflow_id = operation_dag_parent_workflow_id(&scope().run_id);
    assert!(foreign_parent
        .verify_produced_input_from_observed_completion(
            &PortId::from("vector_projection"),
            &v2_source,
            &v2_result,
        )
        .is_err());
}
