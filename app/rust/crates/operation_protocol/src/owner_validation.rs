//! Typed, side-effect-free owner preflight for the V2 Temporal DAG parent.
//!
//! The coordinator validates the complete sealed plan before scheduling any
//! owner effect Child. Every owner then independently parses its own command
//! and any already available External payload into concrete Rust DTOs. Future
//! Produced payload bytes do not yet exist: only their pinned schema is
//! validated here, and the receiving effect owner must validate actual bytes
//! again after the producer completed and before its own external effect.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    operation_dag_parent_workflow_id_v2, owner_content_hash, AdmittedArtifactV1, ArtifactSchema,
    ArtifactBinding, ContractHash, OperationDagRunV6, OperationInstanceId, OperationOwnerId,
    OperationRef, PlanHash, PortId, RunScope, TemporalExecutionIdentityV4,
    OPERATION_DAG_RUN_SCHEMA_REVISION_V6, OWNER_OPERATION_CHILD_TYPE_V1,
};

pub const OWNER_VALIDATION_CHILD_TYPE_V1: &str = "AlegriaOwnerValidateV1Workflow";
pub const OWNER_VALIDATION_SCHEMA_REVISION_V1: u16 = 1;

/// One pure validation Child per DAG operation, pinned to the same owner
/// Task Queue as the eventual effect Child. There is no generic SQL/RPC
/// parser and no cross-provider command in this versioned wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerValidationRequestV1 {
    pub schema_revision: u16,
    pub parent: TemporalExecutionIdentityV4,
    pub plan_hash: PlanHash,
    pub scope: RunScope,
    pub instance: OperationInstanceId,
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub contract_hash: ContractHash,
    pub command_bytes: Vec<u8>,
    pub input_schemas: BTreeMap<PortId, ArtifactSchema>,
    /// Actual immutable, previously admitted bytes available before effects.
    pub external_inputs: BTreeMap<PortId, AdmittedArtifactV1>,
    /// The upstream bytes do not exist yet. Owners validate the exact static
    /// schema here and validate real Produced payloads before their effect.
    pub produced_input_ports: BTreeSet<PortId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerValidationResultV1 {
    pub schema_revision: u16,
    pub request_hash: String,
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub contract_hash: ContractHash,
    pub accepted: bool,
}

#[derive(Debug, Error)]
pub enum OwnerValidationError {
    #[error("invalid owner validation preflight request")]
    InvalidRequest,
    #[error("owner validation result does not attest the exact request")]
    InvalidResult,
    #[error("owner validation JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// A parent/run/plan-scoped Child ID, never aliasing an effect Child ID.
pub fn owner_validation_child_workflow_id_v1(
    parent: &TemporalExecutionIdentityV4,
    plan_hash: &PlanHash,
    instance: &OperationInstanceId,
) -> String {
    // The server-issued parent Run ID distinguishes a new execution of the
    // same sealed plan from a previous terminated/continued execution.
    let run_hash = blake3::hash(parent.run_id.as_bytes());
    let instance_hash = blake3::hash(instance.as_str().as_bytes());
    format!(
        "alegria-owner-validate-v1-{}-{}-{}",
        &run_hash.to_hex().as_str()[..16],
        plan_hash.as_str(),
        &instance_hash.to_hex().as_str()[..16]
    )
}

impl OwnerValidationRequestV1 {
    pub fn request_hash(&self) -> Result<String, serde_json::Error> {
        Ok(owner_content_hash(&serde_json::to_vec(self)?))
    }

    pub fn validate_shape(&self) -> Result<(), OwnerValidationError> {
        if self.schema_revision != OWNER_VALIDATION_SCHEMA_REVISION_V1
            || self.parent.workflow_id != operation_dag_parent_workflow_id_v2(&self.scope.run_id)
            || self.parent.run_id.trim().is_empty()
            || self.scope.run_id.trim().is_empty()
            || self.scope.tenant_id.trim().is_empty()
            || self.plan_hash.as_str().is_empty()
            || self.instance.as_str().is_empty()
            || self.operation.id.as_str().is_empty()
            || self.owner.as_str().is_empty()
            || self.contract_hash.as_str().is_empty()
            || self.command_bytes.is_empty()
            || self
                .external_inputs
                .keys()
                .any(|key| self.produced_input_ports.contains(key))
            || self.input_schemas.len()
                != self.external_inputs.len() + self.produced_input_ports.len()
            || self.external_inputs.iter().any(|(port, value)| {
                self.input_schemas.get(port) != Some(&value.schema)
                    || value.scope != self.scope
                    || value.provenance.trim().is_empty()
            })
            || self
                .produced_input_ports
                .iter()
                .any(|port| !self.input_schemas.contains_key(port))
        {
            return Err(OwnerValidationError::InvalidRequest);
        }
        Ok(())
    }

    /// Independently check a server-observed ValidateOnly request against the
    /// immutable DAG parent start input. This is a *structural* check shared
    /// by every owner operator, not approval of another owner's business DTO
    /// or authority to issue an effect admission.
    pub fn verify_against_run(&self, run: &OperationDagRunV6) -> Result<(), OwnerValidationError> {
        self.validate_shape()?;
        if run.schema_revision != OPERATION_DAG_RUN_SCHEMA_REVISION_V6
            || run.plan.canonical_hash().ok() != Some(run.plan.hash.clone())
            || self.scope != run.plan.scope
            || self.plan_hash != run.plan.hash
        {
            return Err(OwnerValidationError::InvalidRequest);
        }
        let node = run
            .plan
            .nodes
            .iter()
            .filter(|node| node.instance == self.instance)
            .collect::<Vec<_>>();
        if node.len() != 1 {
            return Err(OwnerValidationError::InvalidRequest);
        }
        let node = node[0];
        let command = run.owner_commands.get(&node.instance);
        let matching_contracts = run
            .plan
            .pinned_contracts
            .iter()
            .filter(|pinned| pinned.reference == node.operation)
            .collect::<Vec<_>>();
        let matching_routes = run
            .pinned_routes
            .iter()
            .filter(|route| route.operation == node.operation)
            .collect::<Vec<_>>();
        if self.operation != node.operation
            || command.is_none_or(|command| {
                command.operation != self.operation
                    || command.owner != self.owner
                    || command.contract_hash != self.contract_hash
                    || command.command_bytes != self.command_bytes
            })
            || matching_contracts.len() != 1
            || matching_contracts[0].fingerprint != self.contract_hash
            || matching_routes.len() != 1
            || matching_routes[0].owner != self.owner
            || matching_routes[0].contract_hash != self.contract_hash
            || matching_routes[0].workflow_type != OWNER_OPERATION_CHILD_TYPE_V1
            || matching_routes[0].task_queue.trim().is_empty()
            || matching_routes[0].build_id.trim().is_empty()
            || node.inputs.len() != self.input_schemas.len()
            || node.inputs.iter().any(|(port, source)| {
                if !self.input_schemas.contains_key(port) {
                    return true;
                }
                match source {
                    ArtifactBinding::External { artifact } => {
                        let admitted = run.admitted.get(artifact);
                        let declared = run.plan.external_artifacts.get(artifact);
                        self.external_inputs.get(port) != admitted
                            || admitted.is_none_or(|value| {
                                self.input_schemas.get(port) != Some(&value.schema)
                            })
                            || declared.is_none_or(|value| {
                                admitted.is_none_or(|actual| {
                                    actual.schema != value.schema
                                        || actual.scope != value.scope
                                        || actual.provenance != value.provenance
                                })
                            })
                            || self.produced_input_ports.contains(port)
                    }
                    ArtifactBinding::Produced { .. } => {
                        !self.produced_input_ports.contains(port)
                            || self.external_inputs.contains_key(port)
                    }
                }
            })
        {
            return Err(OwnerValidationError::InvalidRequest);
        }
        Ok(())
    }
}

impl OwnerValidationResultV1 {
    pub fn accepted(request: &OwnerValidationRequestV1) -> Result<Self, OwnerValidationError> {
        request.validate_shape()?;
        Ok(Self {
            schema_revision: OWNER_VALIDATION_SCHEMA_REVISION_V1,
            request_hash: request.request_hash()?,
            operation: request.operation.clone(),
            owner: request.owner.clone(),
            contract_hash: request.contract_hash.clone(),
            accepted: true,
        })
    }

    pub fn verify_success(
        &self,
        request: &OwnerValidationRequestV1,
    ) -> Result<(), OwnerValidationError> {
        request.validate_shape()?;
        if self.schema_revision != OWNER_VALIDATION_SCHEMA_REVISION_V1
            || self.request_hash != request.request_hash()?
            || self.operation != request.operation
            || self.owner != request.owner
            || self.contract_hash != request.contract_hash
            || !self.accepted
        {
            return Err(OwnerValidationError::InvalidResult);
        }
        Ok(())
    }
}
