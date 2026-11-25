//! A Qdrant-owned operation consuming a provider-neutral vector projection
//! produced by an independent Temporal owner. It never reads PostgreSQL
//! application data or imports the producer's Rust types.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    owner_dispatch::{OwnerInvocationV1, PinnedOwnerRouteV1, TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1},
    ArtifactBinding, ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation,
    OperationContract, OperationId, OperationOwnerId, OperationRef, PortId,
};
use projection_contracts::VectorProjectionV1;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    qdrant_owner_generic_v1::{GenericQdrantContractError, QDRANT_GENERIC_OWNER_QUEUE_V1},
    QdrantKeywordChildResultV2, QdrantKeywordProjectionCommandV1,
    QDRANT_EFFECT_OPERATION_ID_V2, QDRANT_EFFECT_OUTPUT_PORT_V2, QDRANT_EFFECT_OWNER_ID_V2,
};

pub const QDRANT_PRODUCED_VECTOR_OPERATION_ID_V1: &str = "qdrant.vector_projection.upsert_produced";
pub const QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1: &str = "vector_projection";

pub struct QdrantProducedVectorProjectionContractV1;

impl Operation for QdrantProducedVectorProjectionContractV1 {
    type Input = VectorProjectionV1;
    type Output = QdrantKeywordChildResultV2;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(QDRANT_PRODUCED_VECTOR_OPERATION_ID_V1.to_owned()),
                revision: 1,
            },
            owner: OperationOwnerId(QDRANT_EFFECT_OWNER_ID_V2.to_owned()),
            inputs: BTreeMap::from([(
                PortId(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId("projection.vector_projection".to_owned()),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId(QDRANT_EFFECT_OUTPUT_PORT_V2.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId("qdrant.keyword_projection_receipt".to_owned()),
                    revision: 2,
                },
            )]),
            invalidates: BTreeSet::from([ArtifactTypeId(
                "qdrant.keyword_projection_snapshot".to_owned(),
            )]),
            // Same external effect as the historical Qdrant V2 contract: a
            // checked DAG cannot schedule these two writes in parallel.
            effects: BTreeSet::from([EffectId(QDRANT_EFFECT_OPERATION_ID_V2.to_owned())]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

/// Immutable Qdrant-local admission identity. Crucially this contains NO
/// producer vector, tenant or projection keys: those bytes can ONLY arrive
/// in the typed Produced output of an earlier Temporal Child Workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantProducedVectorCommandV1 {
    pub schema_revision: u16,
    pub admission_id: Uuid,
}

pub fn verify_produced_qdrant_owner_invocation(
    invocation: &OwnerInvocationV1,
) -> Result<QdrantKeywordProjectionCommandV1, GenericQdrantContractError> {
    let contract = QdrantProducedVectorProjectionContractV1::contract();
    let mut catalog = TrustedRouteCatalog::default();
    catalog
        .register(
            contract.clone(),
            PinnedOwnerRouteV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract
                    .fingerprint()
                    .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?,
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: QDRANT_GENERIC_OWNER_QUEUE_V1.into(),
                build_id: "qdrant-generic-owner-v1-local-type-check".into(),
            },
        )
        .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?;
    invocation
        .verify_integrity(&catalog)
        .map_err(|_| GenericQdrantContractError::ForeignOrInvalidDispatch)?;

    let wire: QdrantProducedVectorCommandV1 =
        serde_json::from_slice(&invocation.command_bytes)?;
    if wire.schema_revision != 1 || wire.admission_id.is_nil() {
        return Err(GenericQdrantContractError::InvalidInput);
    }
    let input_port = PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1);
    let bound = invocation
        .bound_inputs
        .get(&input_port)
        .ok_or(GenericQdrantContractError::InvalidInput)?;
    if !matches!(&bound.source, ArtifactBinding::Produced { .. })
        || bound.produced_evidence.is_none()
    {
        return Err(GenericQdrantContractError::UnprovenProducedArtifact);
    }
    let projection: VectorProjectionV1 =
        serde_json::from_slice(&bound.artifact.payload)?;
    // A receipt cannot be reinterpreted as a vector. The consumer checks a
    // real, bounded, finite vector and exact admitted tenant before any I/O.
    if projection.tenant_id != invocation.checked_dispatch.scope.tenant_id
        || projection.tenant_id.trim().is_empty()
        || projection.context_key.trim().is_empty()
        || projection.keyword_key.trim().is_empty()
        || projection.vector.len() != 1024
        || !projection.vector.iter().all(|component| component.is_finite())
    {
        return Err(GenericQdrantContractError::InvalidInput);
    }
    let command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: wire.admission_id,
        tenant_id: projection.tenant_id,
        context_key: projection.context_key,
        keyword_key: projection.keyword_key,
        vector: projection.vector,
    };
    command
        .validate()
        .map_err(|_| GenericQdrantContractError::InvalidInput)?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        checked_execution_key, operation_dag_parent_workflow_id,
        owner_dispatch::{
            canonical_owner_dispatch_input_bytes, AdmittedArtifactV1, BoundArtifactV1,
            ProducedArtifactEvidenceV1, OWNER_INVOCATION_SCHEMA_REVISION,
        },
        ArtifactId, CheckedDispatch, OperationInstanceId, PlanHash, RunScope,
        TemporalExecutionIdentityV4,
    };

    fn sample_invocation() -> OwnerInvocationV1 {
        let contract = QdrantProducedVectorProjectionContractV1::contract();
        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: Uuid::new_v4().to_string(),
        };
        let projection = VectorProjectionV1 {
            tenant_id: scope.tenant_id.clone(),
            context_key: "ES:tourist:BY".into(),
            keyword_key: "fee".into(),
            vector: vec![0.25; 1024],
        };
        let artifact_id = ArtifactId::from("actual-pg-vector");
        let producer_key = "a".repeat(64);
        let binding = BTreeMap::from([(
            PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1),
            BoundArtifactV1 {
                artifact_id: artifact_id.clone(),
                source: ArtifactBinding::Produced {
                    artifact: artifact_id.clone(),
                    producer: OperationInstanceId::from("pg-vector-reader"),
                    port: PortId::from("vector_projection"),
                },
                artifact: AdmittedArtifactV1 {
                    schema: contract.inputs[&PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1)].clone(),
                    scope: scope.clone(),
                    provenance: "postgres:durable-vector-source".into(),
                    payload: serde_json::to_vec(&projection).unwrap(),
                },
                produced_evidence: Some(ProducedArtifactEvidenceV1 {
                    producer: OperationInstanceId::from("pg-vector-reader"),
                    port: PortId::from("vector_projection"),
                    artifact_id: artifact_id.clone(),
                    dispatch_execution_key: producer_key.clone(),
                    child_workflow_id: operation_protocol::owner_dispatch::owner_child_workflow_id(&producer_key),
                    result_hash: "b".repeat(64),
                    receipt_hash: "c".repeat(64),
                    payload_hash: blake3::hash(&serde_json::to_vec(&projection).unwrap()).to_hex().to_string(),
                }),
            },
        )]);
        let wire = QdrantProducedVectorCommandV1 {
            schema_revision: 1,
            admission_id: Uuid::new_v4(),
        };
        let bytes = serde_json::to_vec(&wire).unwrap();
        let mut dispatch = CheckedDispatch {
            plan_hash: PlanHash("d".repeat(64)),
            scope: scope.clone(),
            instance: OperationInstanceId::from("qdrant-vector-writer"),
            operation: contract.reference,
            owner: contract.owner,
            effects: contract.effects,
            idempotency: contract.idempotency,
            input_hash: String::new(),
            execution_key: String::new(),
        };
        let hash = blake3::hash(&canonical_owner_dispatch_input_bytes(&binding, &bytes).unwrap())
            .to_hex().to_string();
        dispatch.input_hash = hash.clone();
        dispatch.execution_key = checked_execution_key(&dispatch, &hash).unwrap();
        OwnerInvocationV1 {
            schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: operation_dag_parent_workflow_id(&scope.run_id),
                run_id: Uuid::new_v4().to_string(),
            },
            checked_dispatch: dispatch,
            command_bytes: bytes,
            bound_inputs: binding,
        }
    }

    #[test]
    fn produced_qdrant_contract_cannot_cast_receipt_into_vector() {
        let contract = QdrantProducedVectorProjectionContractV1::contract();
        assert_eq!(contract.inputs.len(), 1);
        assert_eq!(contract.inputs[&PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1)].id.0,
                   "projection.vector_projection");
        assert_ne!(contract.reference.id.0, QDRANT_EFFECT_OPERATION_ID_V2);
        assert_eq!(contract.effects.iter().next().unwrap().0, QDRANT_EFFECT_OPERATION_ID_V2);
        assert!(serde_json::from_slice::<VectorProjectionV1>(b"{\"admission_id\":\"00000000-0000-0000-0000-000000000000\"}").is_err());
    }

    #[test]
    fn qdrant_derives_vector_only_from_produced_owner_result() {
        let invocation = sample_invocation();
        let command = verify_produced_qdrant_owner_invocation(&invocation).unwrap();
        assert_eq!(command.vector, vec![0.25; 1024]);
        let wire: QdrantProducedVectorCommandV1 =
            serde_json::from_slice(&invocation.command_bytes).unwrap();
        assert_eq!(wire.admission_id, command.admission_id);
        assert!(!invocation.command_bytes.windows(4).any(|chunk| chunk == b"0.25"));
    }

    #[test]
    fn qdrant_rejects_external_or_receipt_as_produced_vector() {
        let mut external = sample_invocation();
        let input = external.bound_inputs.values_mut().next().unwrap();
        input.source = ArtifactBinding::External { artifact: input.artifact_id.clone() };
        input.produced_evidence = None;
        assert!(verify_produced_qdrant_owner_invocation(&external).is_err());

        let mut receipt_cast = sample_invocation();
        let input = receipt_cast.bound_inputs.values_mut().next().unwrap();
        input.artifact.payload = b"{\"receipt_id\":\"not-a-vector\"}".to_vec();
        assert!(verify_produced_qdrant_owner_invocation(&receipt_cast).is_err());
    }
}
