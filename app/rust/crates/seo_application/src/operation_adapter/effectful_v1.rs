//! Checked, provider-free preparation of an effectful operation DAG.
//!
//! Unlike `execute_pure_dag`, this records which concrete owner command an
//! operation would need. The manifest is NOT a PG admission. Until a
//! per-operation-instance parent/child Workflow Run ID mapping and durable
//! receipt protocol exist, no Temporal Activity/child Workflow may be started.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactBinding, ArtifactId, CheckedDispatch, DispatchAttempt, ExecutionPlan,
    IdempotencyPolicy, Operation, OperationInstanceId, OperationRef, PlanError, PlanHash, PortId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use truth_domain::review_effect_contract::{
    ReconcileTruthReviewsContractV1, ReconcileTruthReviewsInput, TruthReviewScopeArtifactV1,
    INPUT_PORT,
};

use super::{
    enforce_json_budget, enforce_pure_dag_input_budget, execute_pure_dag,
    semantic_extraction_registry, ArtifactValue, OperationRunError, PureDagReceipt, PureDagRun,
    PURE_DAG_MAX_INPUT_BYTES, PURE_DAG_MAX_NODES,
};

/// Immutable Temporal History input, never a durable operator admission.
/// Unknown owner commands fail deserialization rather than falling back to a
/// generic SQL/HTTP facade; additional commands require a new schema version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDagRunV1 {
    pub plan: ExecutionPlan,
    pub admitted: BTreeMap<ArtifactId, ArtifactValue>,
    pub owner_commands: BTreeMap<OperationInstanceId, OwnerCommandV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OwnerCommandV1 {
    PgTruthReviewReconcileV1(ReconcileTruthReviewsInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedDagStepV1 {
    Pure {
        instance: OperationInstanceId,
        operation: OperationRef,
    },
    Owner {
        instance: OperationInstanceId,
        bound_input: ArtifactId,
        command: OwnerCommandV1,
        checked: CheckedDispatch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDagV1 {
    pub plan_hash: PlanHash,
    pub steps: Vec<PreparedDagStepV1>,
}

impl PreparedDagV1 {
    pub fn effect_instances(&self) -> Vec<OperationInstanceId> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                PreparedDagStepV1::Owner { instance, .. } => Some(instance.clone()),
                PreparedDagStepV1::Pure { .. } => None,
            })
            .collect()
    }
}

/// Only a pure plan may return a completed receipt in this version. Explicitly
/// leaving out an "effect success" variant prevents a future caller from
/// mistaking locally checked bytes for the PG owner's durable outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "receipt", rename_all = "snake_case")]
pub enum CheckedDagOutcomeV1 {
    Pure(PureDagReceipt),
}

#[derive(Debug, Error)]
pub enum EffectDagError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Run(#[from] OperationRunError),
    #[error("effect DAG has no executable operation nodes")]
    EmptyPlan,
    #[error("effect DAG Temporal workflow ID differs from immutable plan run ID")]
    TemporalRunIdMismatch,
    #[error("effect DAG has an unregistered or unsupported owner operation {0:?}")]
    UnsupportedOwner(OperationRef),
    #[error("effect DAG lacks typed owner command for {0:?}")]
    MissingOwnerCommand(OperationInstanceId),
    #[error("effect DAG includes unrequested owner command for {0:?}")]
    UnexpectedOwnerCommand(OperationInstanceId),
    #[error("owner command is invalid at {instance:?}: {reason}")]
    InvalidOwnerCommand {
        instance: OperationInstanceId,
        reason: &'static str,
    },
    #[error("owner effect input must be a declared external artifact at {0:?}")]
    OwnerNeedsExternalInput(OperationInstanceId),
    #[error("owner effect input {artifact:?} is not a typed Truth review scope: {reason}")]
    InvalidReviewScope {
        artifact: ArtifactId,
        reason: String,
    },
    #[error("owner execution is unavailable: per-instance parent/child Run ID admission and PG receipt binding have not been implemented; blocked {0:?}")]
    NoDurableOwnerBridge(Vec<OperationInstanceId>),
}

fn invalid_command(instance: &OperationInstanceId, reason: &'static str) -> EffectDagError {
    EffectDagError::InvalidOwnerCommand {
        instance: instance.clone(),
        reason,
    }
}

/// Resolve the entire manifest, every owner route and every admitted envelope
/// *before* permitting the first Activity. The only registered effect is the
/// authentic, concrete PostgreSQL Truth-review reconciliation contract.
pub fn preflight_effect_dag_v1(
    temporal_workflow_id: &str,
    run: &EffectDagRunV1,
) -> Result<PreparedDagV1, EffectDagError> {
    if temporal_workflow_id != run.plan.scope.run_id {
        return Err(EffectDagError::TemporalRunIdMismatch);
    }
    if run.plan.nodes.is_empty() {
        return Err(EffectDagError::EmptyPlan);
    }
    if run.plan.nodes.len() > PURE_DAG_MAX_NODES {
        return Err(OperationRunError::ResourceBudgetExceeded {
            resource: "operation_nodes",
            limit: PURE_DAG_MAX_NODES,
        }
        .into());
    }
    enforce_pure_dag_input_budget(&PureDagRun {
        plan: run.plan.clone(),
        admitted: run.admitted.clone(),
    })?;
    enforce_json_budget(run, PURE_DAG_MAX_INPUT_BYTES, "effect_dag_input_bytes")?;

    let mut registry = semantic_extraction_registry()?;
    registry.catalog.register::<ReconcileTruthReviewsContractV1>()?;
    // Reject changed contract fingerprints, undeclared effects, stale
    // artifacts, missing dependencies, cycles and unauthorized scopes.
    // Never call seal() on a resumed/untrusted plan.
    let checked_plan = run.plan.validate(&registry.catalog)?;
    for (id, expected) in &run.plan.external_artifacts {
        let value = run
            .admitted
            .get(id)
            .ok_or_else(|| OperationRunError::MissingArtifact(id.clone()))?;
        if value.schema != expected.schema
            || value.scope != expected.scope
            || value.provenance != expected.provenance
        {
            return Err(OperationRunError::InvalidArtifactEnvelope(id.clone()).into());
        }
    }
    for id in run.admitted.keys() {
        if !run.plan.external_artifacts.contains_key(id) {
            return Err(OperationRunError::UnexpectedArtifact(id.clone()).into());
        }
    }

    let nodes: BTreeMap<_, _> = run
        .plan
        .nodes
        .iter()
        .map(|node| (node.instance.clone(), node))
        .collect();
    let effect_reference = ReconcileTruthReviewsContractV1::contract().reference;
    let mut used_commands = BTreeSet::new();
    let mut steps = Vec::with_capacity(nodes.len());
    for instance in checked_plan.execution_order() {
        let node = nodes.get(instance).expect("validated instance in schedule");
        let contract = registry
            .catalog
            .contract(&node.operation)
            .expect("validated contract exists");
        if contract.idempotency == IdempotencyPolicy::Pure {
            // This extra executor check is whole-DAG preflight: an unknown
            // pure implementation must not be discovered after a PG effect.
            if !registry.handlers.contains_key(&node.operation) {
                return Err(EffectDagError::UnsupportedOwner(node.operation.clone()));
            }
            steps.push(PreparedDagStepV1::Pure {
                instance: instance.clone(),
                operation: node.operation.clone(),
            });
            continue;
        }
        if node.operation != effect_reference {
            return Err(EffectDagError::UnsupportedOwner(node.operation.clone()));
        }
        let command = run
            .owner_commands
            .get(instance)
            .ok_or_else(|| EffectDagError::MissingOwnerCommand(instance.clone()))?;
        let OwnerCommandV1::PgTruthReviewReconcileV1(input) = command;
        if input.admission_id.is_nil() || input.run_id.is_nil() {
            return Err(invalid_command(
                instance,
                "nil admission or canonical run UUID",
            ));
        }
        if input.scope != run.plan.scope || input.scope.run_id != input.run_id.to_string() {
            return Err(invalid_command(
                instance,
                "cross-run or cross-tenant command scope",
            ));
        }
        if input.operation_instance != instance.as_str()
            || input.plan_hash != run.plan.hash.as_str()
        {
            return Err(invalid_command(
                instance,
                "instance or plan fingerprint mismatch",
            ));
        }
        if input.context_key.trim().is_empty() || input.context_key != input.context_key.trim() {
            return Err(invalid_command(instance, "invalid canonical context key"));
        }
        let bound_input = match node.inputs.get(&PortId::from(INPUT_PORT)) {
            Some(ArtifactBinding::External { artifact }) => artifact,
            _ => return Err(EffectDagError::OwnerNeedsExternalInput(instance.clone())),
        };
        let admitted = run
            .admitted
            .get(bound_input)
            .ok_or_else(|| OperationRunError::MissingArtifact(bound_input.clone()))?;
        let review_scope: TruthReviewScopeArtifactV1 =
            serde_json::from_slice(&admitted.payload).map_err(|error| {
                EffectDagError::InvalidReviewScope {
                    artifact: bound_input.clone(),
                    reason: error.to_string(),
                }
            })?;
        if review_scope.run_id != input.run_id || review_scope.context_key != input.context_key {
            return Err(invalid_command(
                instance,
                "owner input artifact and command disagree",
            ));
        }
        // The hash covers actual bytes and the concrete command, never an
        // operator-supplied precomputed digest. It does NOT authenticate the
        // artifact's producer or grant PG authority; the PG owner must do so.
        let actual_input_bytes = serde_json::to_vec(&(bound_input, &admitted.payload, command))
            .map_err(|error| OperationRunError::EnvelopeSerialization(error.to_string()))?;
        let dispatch = checked_plan.checked_dispatch(
            instance,
            &actual_input_bytes,
            DispatchAttempt::First,
        )?;
        used_commands.insert(instance.clone());
        steps.push(PreparedDagStepV1::Owner {
            instance: instance.clone(),
            bound_input: bound_input.clone(),
            command: command.clone(),
            checked: dispatch,
        });
    }
    for instance in run.owner_commands.keys() {
        if !used_commands.contains(instance) {
            return Err(EffectDagError::UnexpectedOwnerCommand(instance.clone()));
        }
    }
    Ok(PreparedDagV1 {
        plan_hash: run.plan.hash.clone(),
        steps,
    })
}

/// The only permitted live outcome until an independent PG-admitted
/// parent/child-Run-ID owner bridge is implemented and tested. An effectful
/// request deterministically fails before even the first pure operation, let
/// alone the first Temporal Activity; there is no fake effect receipt.
pub fn execute_checked_dag_v1(
    temporal_workflow_id: &str,
    run: &EffectDagRunV1,
) -> Result<CheckedDagOutcomeV1, EffectDagError> {
    let prepared = preflight_effect_dag_v1(temporal_workflow_id, run)?;
    let effects = prepared.effect_instances();
    if !effects.is_empty() {
        return Err(EffectDagError::NoDurableOwnerBridge(effects));
    }
    let receipt = execute_pure_dag(
        temporal_workflow_id,
        &PureDagRun {
            plan: run.plan.clone(),
            admitted: run.admitted.clone(),
        },
    )?;
    Ok(CheckedDagOutcomeV1::Pure(receipt))
}

#[cfg(test)]
mod tests;
