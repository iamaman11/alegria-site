//! PostgreSQL-owned native Temporal producer for precomputed vector artifacts.
//!
//! This owner knows nothing about Qdrant. It loads a previously materialized
//! semantic vector from a separately authorized PostgreSQL source row and
//! commits one immutable source/Temporal/receipt certificate before returning
//! the typed data through its owner Workflow. The generic DAG and downstream
//! owners never query the PostgreSQL source table.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    owner_dispatch::{
        AdmittedArtifactV1, OwnerInvocationV1, OwnerResultV1, PinnedOwnerRouteV1,
        TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1,
    },
    ArtifactBinding, EffectId, IdempotencyPolicy, Operation, OperationContract, OperationId,
    OperationOwnerId, OperationRef, PortId, RunScope,
};
use projection_contracts::{
    vector_lookup_schema, vector_projection_schema, VectorProjectionLookupV1,
    VectorProjectionSourceReceiptV1, VectorProjectionV1, VECTOR_PROJECTION_PORT_V1,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::truth_review_effect_v3::{assert_unprivileged_pg_effect_role, PG_OWNER_TASK_QUEUE_V1};

pub const OPERATION_ID: &str = "pg.vector_projection.read";
pub const OPERATION_REVISION: u32 = 1;
pub const OWNER_ID: &str = "postgres_store";
pub const EFFECT_ID: &str = "pg.vector_projection.certify_read";
pub const INPUT_PORT: &str = "projection_lookup";
pub const OUTPUT_PORT: &str = VECTOR_PROJECTION_PORT_V1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PgVectorProjectionCommandV1 {
    pub admission_id: Uuid,
    pub run_id: Uuid,
    pub scope: RunScope,
    pub context_key: String,
    pub keyword_key: String,
    pub operation_instance: String,
    pub plan_hash: String,
}

pub type PgVectorProjectionReceiptV1 = VectorProjectionSourceReceiptV1;

pub struct PgReadVectorProjectionV1;

#[derive(Debug, Error)]
pub enum PgVectorProjectionError {
    #[error("invalid typed PostgreSQL projection command: {0}")]
    InvalidCommand(&'static str),
    #[error("PostgreSQL projection owner cannot authenticate its authority")]
    Unauthorized,
    #[error("PostgreSQL projection SQL failed: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("PostgreSQL projection JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("PostgreSQL owner role has unexpected privileges: {0}")]
    WorkerRole(String),
}

impl Operation for PgReadVectorProjectionV1 {
    type Input = VectorProjectionLookupV1;
    type Output = VectorProjectionV1;
    type Failure = PgVectorProjectionError;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(OPERATION_ID.into()),
                revision: OPERATION_REVISION,
            },
            owner: OperationOwnerId(OWNER_ID.into()),
            inputs: BTreeMap::from([(PortId(INPUT_PORT.into()), vector_lookup_schema())]),
            outputs: BTreeMap::from([(PortId(OUTPUT_PORT.into()), vector_projection_schema())]),
            invalidates: BTreeSet::new(),
            // Certifying the authoritative read writes exactly one durable
            // receipt under an operator admission, even though source data
            // itself is read-only.
            effects: BTreeSet::from([EffectId(EFFECT_ID.into())]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

pub fn validate_pg_vector_projection_invocation_v1(
    invocation: &OwnerInvocationV1,
) -> Result<PgVectorProjectionCommandV1, PgVectorProjectionError> {
    let contract = PgReadVectorProjectionV1::contract();
    let mut catalog = TrustedRouteCatalog::default();
    catalog
        .register(
            contract.clone(),
            PinnedOwnerRouteV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract
                    .fingerprint()
                    .map_err(|_| PgVectorProjectionError::InvalidCommand("contract"))?,
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: PG_OWNER_TASK_QUEUE_V1.into(),
                build_id: "pg-vector-projection-v1-owner-code".into(),
            },
        )
        .map_err(|_| PgVectorProjectionError::InvalidCommand("route_catalog"))?;
    invocation
        .verify_integrity(&catalog)
        .map_err(|_| PgVectorProjectionError::InvalidCommand("checked_dispatch"))?;
    let input: PgVectorProjectionCommandV1 = serde_json::from_slice(&invocation.command_bytes)?;
    if serde_json::to_vec(&input)? != invocation.command_bytes
        || input.admission_id.is_nil()
        || input.run_id.to_string() != input.scope.run_id
        || input.run_id.to_string() != invocation.checked_dispatch.scope.run_id
        || input.scope != invocation.checked_dispatch.scope
        || input.operation_instance != invocation.checked_dispatch.instance.as_str()
        || input.plan_hash != invocation.checked_dispatch.plan_hash.as_str()
        || input.scope.tenant_id.trim().is_empty()
        || input.scope.tenant_id != input.scope.tenant_id.trim()
        || input.plan_hash.len() != 64
    {
        return Err(PgVectorProjectionError::InvalidCommand("command_binding"));
    }
    let bound = invocation
        .bound_inputs
        .get(&PortId(INPUT_PORT.into()))
        .ok_or(PgVectorProjectionError::InvalidCommand("missing_lookup"))?;
    if !matches!(&bound.source, ArtifactBinding::External { artifact }
        if artifact == &bound.artifact_id)
        || bound.produced_evidence.is_some()
    {
        return Err(PgVectorProjectionError::InvalidCommand("lookup_source"));
    }
    let lookup: VectorProjectionLookupV1 = serde_json::from_slice(&bound.artifact.payload)?;
    lookup
        .validate()
        .map_err(|_| PgVectorProjectionError::InvalidCommand("lookup_identity"))?;
    if lookup.context_key != input.context_key
        || lookup.keyword_key != input.keyword_key
        || serde_json::to_vec(&lookup)? != bound.artifact.payload
    {
        return Err(PgVectorProjectionError::InvalidCommand("lookup_binding"));
    }
    Ok(input)
}

/// SQL obtains the vector from an authoritative database row, not from any
/// payload, command or downstream service. Its immutable receipt is bound to
/// operator-issued real Temporal parent/child Histories and plan/tenant keys.
pub async fn execute_pg_vector_projection_owner_v1(
    pool: &PgPool,
    actual_child_workflow_id: &str,
    actual_child_workflow_run_id: &str,
    invocation: &OwnerInvocationV1,
) -> Result<(VectorProjectionV1, PgVectorProjectionReceiptV1), PgVectorProjectionError> {
    let command = validate_pg_vector_projection_invocation_v1(invocation)?;
    if actual_child_workflow_id.trim().is_empty()
        || Uuid::parse_str(actual_child_workflow_run_id).is_err()
    {
        return Err(PgVectorProjectionError::InvalidCommand(
            "actual_temporal_child",
        ));
    }
    assert_unprivileged_pg_effect_role(pool)
        .await
        .map_err(|e| PgVectorProjectionError::WorkerRole(e.to_string()))?;
    let invocation_bytes = serde_json::to_vec(invocation)?;
    let fingerprint = PgReadVectorProjectionV1::contract()
        .fingerprint()
        .map_err(|_| PgVectorProjectionError::InvalidCommand("fingerprint"))?;
    let row = sqlx::query(
        "SELECT * FROM pipeline.consume_pg_vector_projection_owner_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(command.admission_id)
    .bind(command.run_id)
    .bind(&command.scope.tenant_id)
    .bind(&command.context_key)
    .bind(&command.keyword_key)
    .bind(actual_child_workflow_id)
    .bind(actual_child_workflow_run_id)
    .bind(&command.operation_instance)
    .bind(&command.plan_hash)
    .bind(&fingerprint)
    .bind(&invocation.checked_dispatch.execution_key)
    .bind(&invocation.parent.workflow_id)
    .bind(&invocation.parent.run_id)
    .bind(invocation_bytes)
    .fetch_one(pool)
    .await?;
    let vector: Vec<f32> = serde_json::from_value(row.try_get("vector")?)?;
    let projection = VectorProjectionV1 {
        tenant_id: command.scope.tenant_id.clone(),
        context_key: command.context_key.clone(),
        keyword_key: command.keyword_key.clone(),
        vector,
    };
    projection
        .validate()
        .map_err(|_| PgVectorProjectionError::InvalidCommand("authoritative_vector"))?;
    let receipt = PgVectorProjectionReceiptV1 {
        receipt_id: row.try_get("receipt_id")?,
        admission_id: command.admission_id,
        execution_key: invocation.checked_dispatch.execution_key.clone(),
        source_hash: row.try_get("source_hash")?,
        child_workflow_run_id: actual_child_workflow_run_id.to_string(),
    };
    Ok((projection, receipt))
}

pub fn confirmed_pg_vector_projection_owner_result(
    invocation: &OwnerInvocationV1,
    projection: VectorProjectionV1,
    receipt: PgVectorProjectionReceiptV1,
) -> Result<OwnerResultV1, PgVectorProjectionError> {
    projection
        .validate()
        .map_err(|_| PgVectorProjectionError::InvalidCommand("authoritative_vector"))?;
    if receipt.execution_key != invocation.checked_dispatch.execution_key
        || receipt.admission_id.is_nil()
    {
        return Err(PgVectorProjectionError::InvalidCommand("receipt_identity"));
    }
    let payload = serde_json::to_vec(&projection)?;
    let receipt_bytes = serde_json::to_vec(&receipt)?;
    Ok(OwnerResultV1 {
        dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
        outputs: BTreeMap::from([(
            PortId(OUTPUT_PORT.into()),
            AdmittedArtifactV1 {
                schema: vector_projection_schema(),
                scope: invocation.checked_dispatch.scope.clone(),
                provenance: format!(
                    "pg:vector-source-sha256:{}:receipt:{}",
                    receipt.source_hash, receipt.receipt_id
                ),
                payload,
            },
        )]),
        receipt_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use projection_contracts::VECTOR_DIMENSIONS_V1;

    #[test]
    fn pg_contract_publishes_provider_neutral_projection_not_qdrant_command() {
        let contract = PgReadVectorProjectionV1::contract();
        assert_eq!(contract.reference.id.as_str(), OPERATION_ID);
        assert_eq!(contract.owner.as_str(), OWNER_ID);
        assert_eq!(
            contract.outputs[&PortId(OUTPUT_PORT.into())],
            vector_projection_schema()
        );
        assert_eq!(
            contract.inputs[&PortId(INPUT_PORT.into())],
            vector_lookup_schema()
        );
        let projection = VectorProjectionV1 {
            tenant_id: "tenant-a".into(),
            context_key: "ES:tourist:BY".into(),
            keyword_key: "visa-fee".into(),
            vector: vec![0.25; VECTOR_DIMENSIONS_V1],
        };
        projection.validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<VectorProjectionV1>(&serde_json::to_vec(&projection).unwrap())
                .unwrap(),
            projection
        );
    }
}
