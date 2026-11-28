//! One source of truth for what an *installed contract catalog* can prove.
//!
//! A user recipe and even a valid parameter DTO cannot authenticate a user,
//! mint a budget-bound request, prove that an owner Worker is live, or attest
//! Temporal History. Keep those facts explicit for each individual tool.

use operation_protocol::{IdempotencyPolicy, OperationContract};
use serde::Serialize;

use super::{ParameterDescription, ToolDescription};

pub const OPERATION_EXECUTION_READINESS_SCHEMA_REVISION_V1: u16 = 1;

/// A typed statement about a prerequisite. Runtime-specific, independently
/// authenticated evidence would be needed to promote `NotVerified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationGateV1 {
    /// A versioned contract/schema is installed; no execution is implied.
    DefinitionInstalled,
    /// The contract is absent; callers cannot substitute an authored schema.
    DefinitionMissing,
    /// Owner ValidateOnly must check settings beyond local JSON object shape.
    IndependentOwnerValidationRequired,
    /// Runtime user, source, owner, or Temporal evidence is not available.
    NotVerified,
    /// This prerequisite is not needed for the operation's declared shape.
    NotApplicable,
}

/// Derived ONLY from trusted locally installed contracts. An authoring caller
/// cannot set either executable flag or inject its own runtime attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationExecutionReadinessV1 {
    pub schema_revision: u16,
    pub status: &'static str,
    pub parameter_contract: OperationGateV1,
    pub typed_output_contract: OperationGateV1,
    pub typed_failure_wire: OperationGateV1,
    pub input_binding_and_source_provenance: OperationGateV1,
    pub installed_runtime_owner_route: OperationGateV1,
    pub owner_validate_only_history: OperationGateV1,
    pub authenticated_principal_tenant_acl: OperationGateV1,
    pub bounded_one_use_request_grant: OperationGateV1,
    pub temporal_start_and_server_history: OperationGateV1,
    pub provider_usage_and_budget_receipt: OperationGateV1,
    pub user_start_verified: bool,
    pub group_execution_verified: bool,
}

/// Exact installed-catalog counts, not a hard-coded copy of the 52 provisional
/// product candidates. Never sum unrelated historical test evidence into an
/// executable acceptance count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationReadinessSummaryV1 {
    pub schema_revision: u16,
    pub status: &'static str,
    pub installed_versioned_operations: usize,
    pub with_installed_parameter_definition: usize,
    pub with_missing_parameter_definition: usize,
    pub requiring_independent_owner_parameter_validation: usize,
    pub user_start_verified: usize,
    pub group_execution_verified: usize,
}

pub fn summarize_local_execution_readiness(
    tools: &[ToolDescription],
) -> OperationReadinessSummaryV1 {
    OperationReadinessSummaryV1 {
        schema_revision: OPERATION_EXECUTION_READINESS_SCHEMA_REVISION_V1,
        status: "installed_catalog_only",
        installed_versioned_operations: tools.len(),
        with_installed_parameter_definition: tools
            .iter()
            .filter(|tool| {
                tool.execution_readiness.parameter_contract == OperationGateV1::DefinitionInstalled
            })
            .count(),
        with_missing_parameter_definition: tools
            .iter()
            .filter(|tool| {
                tool.execution_readiness.parameter_contract == OperationGateV1::DefinitionMissing
            })
            .count(),
        requiring_independent_owner_parameter_validation: tools
            .iter()
            .filter(|tool| {
                tool.execution_readiness.parameter_contract
                    == OperationGateV1::IndependentOwnerValidationRequired
            })
            .count(),
        user_start_verified: tools
            .iter()
            .filter(|tool| tool.execution_readiness.user_start_verified)
            .count(),
        group_execution_verified: tools
            .iter()
            .filter(|tool| tool.execution_readiness.group_execution_verified)
            .count(),
    }
}

pub(crate) fn local_catalog_execution_readiness(
    contract: &OperationContract,
    parameters: Option<&ParameterDescription>,
) -> OperationExecutionReadinessV1 {
    let parameter_contract = match parameters {
        None => OperationGateV1::DefinitionMissing,
        Some(description) if description.verification == "typed_local" => {
            OperationGateV1::DefinitionInstalled
        }
        Some(_) => OperationGateV1::IndependentOwnerValidationRequired,
    };
    let effectful = !contract.effects.is_empty() || contract.idempotency != IdempotencyPolicy::Pure;
    OperationExecutionReadinessV1 {
        schema_revision: OPERATION_EXECUTION_READINESS_SCHEMA_REVISION_V1,
        status: "catalog_preview_only",
        parameter_contract,
        typed_output_contract: OperationGateV1::DefinitionInstalled,
        typed_failure_wire: OperationGateV1::NotVerified,
        input_binding_and_source_provenance: if contract.inputs.is_empty() {
            OperationGateV1::NotApplicable
        } else {
            OperationGateV1::NotVerified
        },
        installed_runtime_owner_route: OperationGateV1::NotVerified,
        owner_validate_only_history: OperationGateV1::NotVerified,
        authenticated_principal_tenant_acl: OperationGateV1::NotVerified,
        bounded_one_use_request_grant: OperationGateV1::NotVerified,
        temporal_start_and_server_history: OperationGateV1::NotVerified,
        provider_usage_and_budget_receipt: if effectful {
            OperationGateV1::NotVerified
        } else {
            OperationGateV1::NotApplicable
        },
        user_start_verified: false,
        group_execution_verified: false,
    }
}
