//! Provider-neutral, immutable boundary between a checked operation DAG and
//! independently deployed Temporal effect owners.
//!
//! No provider-specific input, SDK, SQL statement, URL, Task Queue or enum
//! lives in this module. The trusted runner registers owner routes; the
//! untrusted plan can only *select* those registered operation contracts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    checked_execution_key, ArtifactBinding, ArtifactId, ArtifactSchema, CheckedDispatch,
    ContractCatalog, ContractHash, ExecutionPlan, OperationContract, OperationInstanceId,
    OperationOwnerId, OperationRef, PlanError, PortId, RunScope, TemporalExecutionIdentityV4,
};

pub const OPERATION_DAG_RUN_SCHEMA_REVISION_V6: u16 = 6;
pub const OPERATION_DAG_PARENT_TYPE: &str = "AlegriaOperationDagV1Workflow";
pub const OPERATION_DAG_PARENT_QUEUE: &str = "alegria-operation-dag-v1";
/// The domain/business RunScope is NOT the Temporal parent Workflow ID or
/// the server-generated Temporal Workflow Run ID. In particular it must not
/// collide with a still-active `seo_site_build` Workflow for the same Truth
/// run. This deterministic namespace applies only to the new generic parent.
pub const OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX: &str = "alegria-operation-dag-v1-";

pub fn operation_dag_parent_workflow_id(business_run_id: &str) -> String {
    format!("{OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX}{business_run_id}")
}
/// All owners implement the same typed Temporal child ABI on separate queues.
/// A client can never select an arbitrary Workflow type from plan data.
pub const OWNER_OPERATION_CHILD_TYPE_V1: &str = "AlegriaOwnerOperationV1Workflow";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedArtifactV1 {
    pub schema: ArtifactSchema,
    pub scope: RunScope,
    pub provenance: String,
    pub payload: Vec<u8>,
}

/// Command bytes are decoded as a concrete Rust Input ONLY by their owner.
/// The generic runner verifies the operation/owner/contract and binds bytes
/// to the exact declared artifact ports before scheduling external effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerCommandWireV1 {
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub contract_hash: ContractHash,
    pub command_bytes: Vec<u8>,
}

/// Retains the exact producer-to-consumer edge as well as the byte payload.
/// The owner verifies its admitted parent History plan before any effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundArtifactV1 {
    pub artifact_id: ArtifactId,
    pub source: ArtifactBinding,
    pub artifact: AdmittedArtifactV1,
    /// External artifacts must have no producer proof. Produced artifacts
    /// must bind the exact earlier owner child result and receipt bytes.
    /// This deterministic claim is NOT itself independent History attestation:
    /// the receiving owner operator checks server parent/child Histories.
    pub produced_evidence: Option<ProducedArtifactEvidenceV1>,
}

/// Deterministic digest of a *completed* upstream typed Temporal child result.
/// No arbitrary plan can manufacture this in the generic DAG machine. A
/// receiving owner must corroborate the hashes against the parent's actual
/// ChildWorkflowExecutionCompleted event and the owner's durable receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducedArtifactEvidenceV1 {
    pub producer: OperationInstanceId,
    pub port: PortId,
    pub artifact_id: ArtifactId,
    pub dispatch_execution_key: String,
    pub child_workflow_id: String,
    /// Blake3 hash of canonical `serde_json::to_vec(OwnerResultV1)` bytes.
    pub result_hash: String,
    pub receipt_hash: String,
    pub payload_hash: String,
}

/// Exact child Workflow ID used by the generic parent and independently
/// checked by owner operators against server-observed Temporal History.
pub fn owner_child_workflow_id(dispatch_execution_key: &str) -> String {
    format!("alegria-owner-operation-v1-{dispatch_execution_key}")
}

fn is_canonical_blake3_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Snapshot from a trusted route catalog, recorded in Temporal start History
/// so replay cannot silently dispatch old effects to a different deployment.
/// The caller-supplied copy has NO authority unless it equals that catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedOwnerRouteV1 {
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub contract_hash: ContractHash,
    pub workflow_type: String,
    pub task_queue: String,
    pub build_id: String,
}

/// Versioned owner-neutral DAG start input. The user-provided plan contains
/// no arbitrary SQL, URL, provider SDK or Dynamic Workflow route.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDagRunV6 {
    pub schema_revision: u16,
    pub plan: ExecutionPlan,
    pub admitted: BTreeMap<ArtifactId, AdmittedArtifactV1>,
    pub owner_commands: BTreeMap<OperationInstanceId, OwnerCommandWireV1>,
    pub pinned_routes: Vec<PinnedOwnerRouteV1>,
}

pub const OWNER_INVOCATION_SCHEMA_REVISION: u16 = 1;

/// A stable Temporal child input independent of any provider's command type.
/// The effect owner decodes command_bytes as its concrete Rust Input type and
/// independently validates server-issued parent/child History and admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerInvocationV1 {
    pub schema_revision: u16,
    pub parent: TemporalExecutionIdentityV4,
    pub checked_dispatch: CheckedDispatch,
    pub command_bytes: Vec<u8>,
    pub bound_inputs: BTreeMap<PortId, BoundArtifactV1>,
}

/// One effect result can materialize zero or more versioned output ports.
/// Each payload is checked against the declared contract before becoming an
/// input of another owner. This result alone is NOT an authorization: the
/// effect owner must independently attest its Temporal History and receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerResultV1 {
    pub dispatch_execution_key: String,
    pub outputs: BTreeMap<PortId, AdmittedArtifactV1>,
    pub receipt_bytes: Vec<u8>,
}

/// Stable canonical digest input shared by the generic parent and independent
/// owner. Ports are sorted by BTreeMap, and the complete artifact envelope
/// (including actual payload) contributes to checked_dispatch.input_hash.
pub fn canonical_owner_dispatch_input_bytes(
    bound_inputs: &BTreeMap<PortId, BoundArtifactV1>,
    command_bytes: &[u8],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&(
        OWNER_INVOCATION_SCHEMA_REVISION,
        bound_inputs,
        command_bytes,
    ))
}

/// An independent owner can recompute this from the previous child's
/// server-observed completed result; a provenance string is not proof.
pub fn canonical_owner_result_hash(result: &OwnerResultV1) -> Result<String, serde_json::Error> {
    Ok(owner_content_hash(&serde_json::to_vec(result)?))
}

/// Provider-neutral byte digest used for receipt and artifact evidence.
pub fn owner_content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[derive(Debug, Error)]
pub enum RouteCatalogError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error("route is not a complete, immutable owner contract")]
    InvalidRoute,
    #[error("owner route already registered for {0:?}")]
    Duplicate(OperationRef),
    #[error("unregistered owner route for {0:?}")]
    Unregistered(OperationRef),
    #[error("owner route does not match checked dispatch for {0:?}")]
    MismatchedDispatch(OperationRef),
    #[error("untrusted route snapshot differs from configured route catalog")]
    UntrustedRouteSnapshot,
    #[error("run schema revision {0} is unsupported")]
    UnsupportedRunSchema(u16),
    #[error("owner invocation has mismatched exact operation, scope or bytes")]
    InvalidInvocation,
    #[error("owner result has invalid dispatch, schema, scope or provenance")]
    InvalidOwnerResult,
    #[error("owner invocation or output JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl OwnerInvocationV1 {
    pub fn verify_integrity(&self, catalog: &TrustedRouteCatalog) -> Result<(), RouteCatalogError> {
        if self.schema_revision != OWNER_INVOCATION_SCHEMA_REVISION
            || self.parent.workflow_id.trim().is_empty()
            || self.parent.run_id.trim().is_empty()
            || self.parent.workflow_id
                != operation_dag_parent_workflow_id(&self.checked_dispatch.scope.run_id)
            || self.command_bytes.is_empty()
        {
            return Err(RouteCatalogError::InvalidInvocation);
        }
        let contract = catalog
            .catalog()
            .contract(&self.checked_dispatch.operation)
            .ok_or_else(|| {
                RouteCatalogError::Unregistered(self.checked_dispatch.operation.clone())
            })?;
        catalog.route_for_dispatch(&self.checked_dispatch)?;
        if self.bound_inputs.len() != contract.inputs.len()
            || self.bound_inputs.keys().collect::<BTreeSet<_>>()
                != contract.inputs.keys().collect::<BTreeSet<_>>()
        {
            return Err(RouteCatalogError::InvalidInvocation);
        }
        for (port, bound) in &self.bound_inputs {
            let source_artifact = match (&bound.source, &bound.produced_evidence) {
                (ArtifactBinding::External { artifact }, None) => artifact,
                (
                    ArtifactBinding::Produced {
                        producer,
                        port: producer_port,
                        artifact,
                    },
                    Some(proof),
                ) if proof.producer == *producer
                    && proof.port == *producer_port
                    && proof.artifact_id == *artifact
                    && is_canonical_blake3_hash(&proof.dispatch_execution_key)
                    && proof.child_workflow_id
                        == owner_child_workflow_id(&proof.dispatch_execution_key)
                    && is_canonical_blake3_hash(&proof.result_hash)
                    && is_canonical_blake3_hash(&proof.receipt_hash)
                    && proof.payload_hash
                        == blake3::hash(&bound.artifact.payload).to_hex().to_string() =>
                {
                    artifact
                }
                _ => return Err(RouteCatalogError::InvalidInvocation),
            };
            if source_artifact != &bound.artifact_id
                || contract.inputs.get(port) != Some(&bound.artifact.schema)
                || bound.artifact.scope != self.checked_dispatch.scope
                || bound.artifact.provenance.trim().is_empty()
            {
                return Err(RouteCatalogError::InvalidInvocation);
            }
        }
        let bytes = canonical_owner_dispatch_input_bytes(&self.bound_inputs, &self.command_bytes)?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        if digest != self.checked_dispatch.input_hash
            || checked_execution_key(&self.checked_dispatch, &digest)
                .map_err(|_| RouteCatalogError::InvalidInvocation)?
                != self.checked_dispatch.execution_key
        {
            return Err(RouteCatalogError::InvalidInvocation);
        }
        Ok(())
    }
}

impl OwnerResultV1 {
    /// Actual owner state is confirmed by its own server History/receipt.
    /// This checks the *shape* before materializing outputs into the DAG.
    pub fn verify_integrity(
        &self,
        dispatch: &CheckedDispatch,
        catalog: &TrustedRouteCatalog,
    ) -> Result<(), RouteCatalogError> {
        let contract = catalog
            .catalog()
            .contract(&dispatch.operation)
            .ok_or_else(|| RouteCatalogError::Unregistered(dispatch.operation.clone()))?;
        catalog.route_for_dispatch(dispatch)?;
        if self.dispatch_execution_key != dispatch.execution_key
            || self.receipt_bytes.is_empty()
            || self.outputs.keys().collect::<BTreeSet<_>>()
                != contract.outputs.keys().collect::<BTreeSet<_>>()
        {
            return Err(RouteCatalogError::InvalidOwnerResult);
        }
        for (port, artifact) in &self.outputs {
            if contract.outputs.get(port) != Some(&artifact.schema)
                || artifact.scope != dispatch.scope
                || artifact.provenance.trim().is_empty()
            {
                return Err(RouteCatalogError::InvalidOwnerResult);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrustedRouteCatalog {
    contracts: ContractCatalog,
    routes: BTreeMap<OperationRef, PinnedOwnerRouteV1>,
}

impl TrustedRouteCatalog {
    /// Trust must originate in runner configuration/deployed build, NEVER in
    /// a caller-controlled DAG manifest or its pinned_routes field.
    pub fn register(
        &mut self,
        contract: OperationContract,
        route: PinnedOwnerRouteV1,
    ) -> Result<(), RouteCatalogError> {
        if route.operation != contract.reference
            || route.owner != contract.owner
            || route.contract_hash != contract.fingerprint()?
            || route.workflow_type != OWNER_OPERATION_CHILD_TYPE_V1
            || route.task_queue.trim().is_empty()
            || route.build_id.trim().is_empty()
        {
            return Err(RouteCatalogError::InvalidRoute);
        }
        if self.routes.contains_key(&route.operation) {
            return Err(RouteCatalogError::Duplicate(route.operation));
        }
        self.contracts.register_contract(contract)?;
        self.routes.insert(route.operation.clone(), route);
        Ok(())
    }

    pub fn catalog(&self) -> &ContractCatalog {
        &self.contracts
    }

    pub fn route_for_dispatch(
        &self,
        dispatch: &CheckedDispatch,
    ) -> Result<&PinnedOwnerRouteV1, RouteCatalogError> {
        let route = self
            .routes
            .get(&dispatch.operation)
            .ok_or_else(|| RouteCatalogError::Unregistered(dispatch.operation.clone()))?;
        let contract = self
            .contracts
            .contract(&dispatch.operation)
            .ok_or_else(|| RouteCatalogError::Unregistered(dispatch.operation.clone()))?;
        if route.owner != dispatch.owner
            || contract.owner != dispatch.owner
            || contract.effects != dispatch.effects
            || contract.idempotency != dispatch.idempotency
            || route.contract_hash != contract.fingerprint()?
        {
            return Err(RouteCatalogError::MismatchedDispatch(
                dispatch.operation.clone(),
            ));
        }
        Ok(route)
    }

    /// Require exact set equality, not merely a matching subset: a forged
    /// extra route could otherwise be recorded and reinterpreted on replay.
    pub fn validate_run_routes(&self, run: &OperationDagRunV6) -> Result<(), RouteCatalogError> {
        if run.schema_revision != OPERATION_DAG_RUN_SCHEMA_REVISION_V6 {
            return Err(RouteCatalogError::UnsupportedRunSchema(run.schema_revision));
        }
        let used: BTreeSet<_> = run
            .owner_commands
            .values()
            .map(|command| command.operation.clone())
            .collect();
        let expected: Vec<_> = self
            .routes
            .iter()
            .filter(|(operation, _)| used.contains(*operation))
            .map(|(_, route)| route.clone())
            .collect();
        if run.pinned_routes != expected {
            return Err(RouteCatalogError::UntrustedRouteSnapshot);
        }
        Ok(())
    }

    pub fn trusted_snapshot_for<'a>(
        &self,
        operations: impl IntoIterator<Item = &'a OperationRef>,
    ) -> Result<Vec<PinnedOwnerRouteV1>, RouteCatalogError> {
        let used: BTreeSet<_> = operations.into_iter().cloned().collect();
        used.iter()
            .map(|operation| {
                self.routes
                    .get(operation)
                    .cloned()
                    .ok_or_else(|| RouteCatalogError::Unregistered(operation.clone()))
            })
            .collect()
    }
}
