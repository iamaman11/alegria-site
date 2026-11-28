//! A provider-free, concretely typed Qdrant adapter for the shared Temporal
//! OwnerInvocationV1 wire. Only EXTERNAL command artifacts are admitted here:
//! a Produced edge requires independent source-Workflow/receipt attestation.

use std::collections::BTreeMap;

use operation_protocol::{
    owner_dispatch::{
        OwnerInvocationV1, OwnerResultV1, PinnedOwnerRouteV1, TrustedRouteCatalog,
        OWNER_OPERATION_CHILD_TYPE_V1,
    },
    ArtifactBinding, Operation, PortId,
};
use thiserror::Error;

use crate::{
    verify_produced_qdrant_owner_invocation, QdrantChildOutcomeV1, QdrantKeywordChildResultV2,
    QdrantKeywordProjectionCommandV1, QdrantKeywordProjectionContractV2,
    QdrantProducedVectorProjectionContractV1, QDRANT_CHILD_SCHEMA_REVISION_V2,
    QDRANT_EFFECT_INPUT_PORT_V2, QDRANT_EFFECT_OUTPUT_PORT_V2,
};

pub const QDRANT_GENERIC_OWNER_QUEUE_V1: &str = "alegria-qdrant-owner-v1";
pub const QDRANT_GENERIC_ACTIVITY_QUEUE_V1: &str = "alegria-qdrant-owner-activities-v1";
pub const QDRANT_GENERIC_ADMISSION_SIGNAL_V1: &str = "operator_admission_ready_qdrant_generic_v1";

#[derive(Debug, Error)]
pub enum GenericQdrantContractError {
    #[error("Qdrant owner invocation contains a foreign operation or inconsistent dispatch")]
    ForeignOrInvalidDispatch,
    #[error(
        "Qdrant Produced artifact requires independently proven prior owner History and receipt"
    )]
    UnprovenProducedArtifact,
    #[error("Qdrant input bytes or type mismatch")]
    InvalidInput,
    #[error("Qdrant command encoding failed: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// Validate a concrete Qdrant input before any provider Activity is scheduled.
/// The operator independently verifies the matching real Temporal Histories,
/// trusted deployed build ID and immutable PostgreSQL admission.
pub fn verify_external_qdrant_owner_invocation(
    invocation: &OwnerInvocationV1,
) -> Result<QdrantKeywordProjectionCommandV1, GenericQdrantContractError> {
    let contract = QdrantKeywordProjectionContractV2::contract();
    let mut catalog = TrustedRouteCatalog::default();
    let route = PinnedOwnerRouteV1 {
        operation: contract.reference.clone(),
        owner: contract.owner.clone(),
        contract_hash: contract
            .fingerprint()
            .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?,
        workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
        task_queue: QDRANT_GENERIC_OWNER_QUEUE_V1.into(),
        // The real deployed build ID is independently pinned by the operator
        // against the trusted configuration and original parent History.
        build_id: "qdrant-generic-owner-v1-local-type-check".into(),
    };
    catalog
        .register(contract, route)
        .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?;
    invocation
        .verify_integrity(&catalog)
        .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?;
    let bound = invocation
        .bound_inputs
        .get(&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2))
        .ok_or(GenericQdrantContractError::InvalidInput)?;
    if !matches!(&bound.source, ArtifactBinding::External { .. }) {
        return Err(GenericQdrantContractError::UnprovenProducedArtifact);
    }
    if bound.produced_evidence.is_some() || bound.artifact.payload != invocation.command_bytes {
        return Err(GenericQdrantContractError::InvalidInput);
    }
    let command: QdrantKeywordProjectionCommandV1 =
        serde_json::from_slice(&invocation.command_bytes)?;
    command
        .validate()
        .map_err(|_| GenericQdrantContractError::InvalidInput)?;
    if command.tenant_id != invocation.checked_dispatch.scope.tenant_id {
        return Err(GenericQdrantContractError::InvalidInput);
    }
    Ok(command)
}

/// Provider-owned dispatch covers the historical External command and the
/// new typed Produced artifact. Both are one local Qdrant owner implementation;
/// neither can invoke or import the upstream owner.
pub fn verify_qdrant_owner_invocation(
    invocation: &OwnerInvocationV1,
) -> Result<QdrantKeywordProjectionCommandV1, GenericQdrantContractError> {
    if invocation.checked_dispatch.operation
        == QdrantKeywordProjectionContractV2::contract().reference
    {
        verify_external_qdrant_owner_invocation(invocation)
    } else if invocation.checked_dispatch.operation
        == QdrantProducedVectorProjectionContractV1::contract().reference
    {
        verify_produced_qdrant_owner_invocation(invocation)
    } else {
        Err(GenericQdrantContractError::ForeignOrInvalidDispatch)
    }
}

/// A committed Qdrant receipt is returned as one declared typed output port.
pub fn confirmed_qdrant_owner_result(
    invocation: &OwnerInvocationV1,
    command: &QdrantKeywordProjectionCommandV1,
    outcome: QdrantChildOutcomeV1,
) -> Result<OwnerResultV1, GenericQdrantContractError> {
    let receipt = QdrantKeywordChildResultV2 {
        schema_revision: QDRANT_CHILD_SCHEMA_REVISION_V2,
        admission_id: command.admission_id,
        dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
        outcome,
    };
    let receipt_bytes = serde_json::to_vec(&receipt)?;
    let contract = if invocation.checked_dispatch.operation
        == QdrantProducedVectorProjectionContractV1::contract().reference
    {
        QdrantProducedVectorProjectionContractV1::contract()
    } else {
        QdrantKeywordProjectionContractV2::contract()
    };
    let port = PortId::from(QDRANT_EFFECT_OUTPUT_PORT_V2);
    let schema = contract.outputs[&port].clone();
    Ok(OwnerResultV1 {
        dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
        outputs: BTreeMap::from([(
            port,
            operation_protocol::owner_dispatch::AdmittedArtifactV1 {
                schema,
                scope: invocation.checked_dispatch.scope.clone(),
                provenance: format!(
                    "qdrant:confirmed:{}:{}",
                    command.admission_id, invocation.checked_dispatch.execution_key
                ),
                payload: receipt_bytes.clone(),
            },
        )]),
        receipt_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        checked_execution_key, operation_dag_parent_workflow_id,
        owner_dispatch::{
            canonical_owner_dispatch_input_bytes, AdmittedArtifactV1, BoundArtifactV1,
            OWNER_INVOCATION_SCHEMA_REVISION,
        },
        ArtifactId, CheckedDispatch, OperationInstanceId, PlanHash, RunScope,
        TemporalExecutionIdentityV4,
    };
    use uuid::Uuid;

    fn valid_external_invocation() -> OwnerInvocationV1 {
        let contract = QdrantKeywordProjectionContractV2::contract();
        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: Uuid::new_v4().to_string(),
        };
        let parent_workflow_id = operation_dag_parent_workflow_id(&scope.run_id);
        let command = QdrantKeywordProjectionCommandV1 {
            schema_revision: 1,
            admission_id: Uuid::new_v4(),
            tenant_id: scope.tenant_id.clone(),
            context_key: "ES:tourist:BY".into(),
            keyword_key: "consular-fee".into(),
            vector: vec![0.25; 1024],
        };
        let bytes = serde_json::to_vec(&command).unwrap();
        let port = PortId::from(QDRANT_EFFECT_INPUT_PORT_V2);
        let input_id = ArtifactId::from("qdrant-command");
        let bound = BTreeMap::from([(
            port.clone(),
            BoundArtifactV1 {
                artifact_id: input_id.clone(),
                source: ArtifactBinding::External { artifact: input_id },
                artifact: AdmittedArtifactV1 {
                    schema: contract.inputs[&port].clone(),
                    scope: scope.clone(),
                    provenance: "operator-admitted-external".into(),
                    payload: bytes.clone(),
                },
                produced_evidence: None,
            },
        )]);
        let mut checked = CheckedDispatch {
            plan_hash: PlanHash("a".repeat(64)),
            scope,
            instance: OperationInstanceId::from("keyword-projection"),
            operation: contract.reference,
            owner: contract.owner,
            effects: contract.effects,
            idempotency: contract.idempotency,
            input_hash: String::new(),
            execution_key: String::new(),
        };
        checked.input_hash =
            blake3::hash(&canonical_owner_dispatch_input_bytes(&bound, &bytes).unwrap())
                .to_hex()
                .to_string();
        checked.execution_key = checked_execution_key(&checked, &checked.input_hash).unwrap();
        OwnerInvocationV1 {
            schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: parent_workflow_id,
                run_id: Uuid::new_v4().to_string(),
            },
            checked_dispatch: checked,
            command_bytes: bytes,
            bound_inputs: bound,
        }
    }

    fn rehash(invocation: &mut OwnerInvocationV1) {
        let bytes = canonical_owner_dispatch_input_bytes(
            &invocation.bound_inputs,
            &invocation.command_bytes,
        )
        .unwrap();
        invocation.checked_dispatch.input_hash = blake3::hash(&bytes).to_hex().to_string();
        invocation.checked_dispatch.execution_key = checked_execution_key(
            &invocation.checked_dispatch,
            &invocation.checked_dispatch.input_hash,
        )
        .unwrap();
    }

    #[test]
    fn external_owner_command_decodes_concrete_type_and_emits_typed_receipt() {
        let invocation = valid_external_invocation();
        let command = verify_external_qdrant_owner_invocation(&invocation).unwrap();
        let result =
            confirmed_qdrant_owner_result(&invocation, &command, QdrantChildOutcomeV1::Applied)
                .unwrap();
        assert_eq!(
            result.dispatch_execution_key,
            invocation.checked_dispatch.execution_key
        );
        let receipt: QdrantKeywordChildResultV2 =
            serde_json::from_slice(&result.receipt_bytes).unwrap();
        assert_eq!(receipt.admission_id, command.admission_id);
        assert_eq!(result.outputs.len(), 1);
    }

    #[test]
    fn produced_input_is_rejected_even_when_checked_hash_is_self_consistent() {
        let mut invocation = valid_external_invocation();
        let input = invocation
            .bound_inputs
            .get_mut(&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2))
            .unwrap();
        input.source = ArtifactBinding::Produced {
            artifact: input.artifact_id.clone(),
            producer: OperationInstanceId::from("postgres-producer"),
            port: PortId::from("receipt"),
        };
        rehash(&mut invocation);
        assert!(verify_external_qdrant_owner_invocation(&invocation).is_err());
    }

    #[test]
    fn foreign_owner_or_changed_payload_cannot_pass_checked_dispatch() {
        let mut invocation = valid_external_invocation();
        invocation.checked_dispatch.owner = "foreign-owner".into();
        assert!(verify_external_qdrant_owner_invocation(&invocation).is_err());
        let mut invocation = valid_external_invocation();
        invocation
            .bound_inputs
            .get_mut(&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2))
            .unwrap()
            .artifact
            .payload
            .push(0);
        assert!(verify_external_qdrant_owner_invocation(&invocation).is_err());
    }
}
