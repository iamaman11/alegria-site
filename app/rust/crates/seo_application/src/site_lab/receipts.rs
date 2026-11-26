//! Offline consistency checks for an already completed generic Temporal DAG.
//!
//! The plan and receipt are untrusted files. This validates them against the
//! installed contract catalog and each other; it NEVER verifies server-issued
//! Temporal History, owner receipts, source authorization or provider state.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    canonical_owner_result_hash, enforce_json_budget, owner_child_workflow_id_v2,
    owner_content_hash, ArtifactId, ArtifactSchema, ContractCatalog, ExecutionPlan,
    OperationDagReceiptV1, OperationInstanceId, OperationRef, PlanError, PlanHash, PortId,
    DAG_MAX_INPUT_BYTES, DAG_MAX_RESULT_BYTES,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InspectionError {
    #[error("saved plan is invalid against the installed trusted catalog")]
    Plan(#[source] PlanError),
    #[error("saved plan or receipt exceeds the existing DAG size budget")]
    Budget,
    #[error("receipt is inconsistent at {0}")]
    Inconsistent(&'static str),
    #[error("both reports refer to the same business run")]
    SameRun,
    #[error("runs do not share one exact tenant, external source metadata and source bytes")]
    DifferentSources,
}

#[derive(Debug, Serialize)]
pub struct InspectionProblem {
    pub status: &'static str,
    pub code: &'static str,
    pub message: String,
    pub next_action: &'static str,
}

impl InspectionError {
    pub fn problem(&self) -> InspectionProblem {
        let (code, next_action) = match self {
            Self::Plan(_) => (
                "saved_plan_invalid",
                "Use the original sealed plan and install its exact trusted operation revisions.",
            ),
            Self::Budget => (
                "saved_result_too_large",
                "Use the existing bounded Temporal DAG envelopes; do not truncate receipts.",
            ),
            Self::Inconsistent(_) => (
                "receipt_inconsistent",
                "Inspect the named receipt boundary; obtain the original completed Temporal result.",
            ),
            Self::SameRun => (
                "same_run",
                "Choose two distinct completed business runs for comparison.",
            ),
            Self::DifferentSources => (
                "different_source_snapshot",
                "Repeat both hypotheses with identical authenticated source snapshots and tenant.",
            ),
        };
        InspectionProblem {
            status: "invalid",
            code,
            message: self.to_string(),
            next_action,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactSummary {
    pub id: ArtifactId,
    pub schema: ArtifactSchema,
    pub origin: &'static str,
    pub payload_hash: String,
    pub payload_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct StepSummary {
    pub instance: OperationInstanceId,
    pub operation: OperationRef,
    pub owner: String,
    pub result_hash: String,
    pub receipt_hash: String,
    pub output_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ExecutionInspection {
    pub status: &'static str,
    pub tenant_id: String,
    pub run_id: String,
    pub plan_hash: PlanHash,
    pub steps: Vec<StepSummary>,
    pub artifacts: Vec<ArtifactSummary>,
    pub temporal_history_attestation: &'static str,
    pub owner_receipt_attestation: &'static str,
    pub source_authentication: &'static str,
}

/// Check a V2 parent receipt locally; the result is NOT evidence that Temporal
/// issued these bytes or the providers performed the described effects.
pub fn inspect_execution(
    plan: &ExecutionPlan,
    receipt: &OperationDagReceiptV1,
    catalog: &ContractCatalog,
) -> Result<ExecutionInspection, InspectionError> {
    enforce_json_budget(plan, DAG_MAX_INPUT_BYTES, "lab_saved_plan")
        .map_err(|_| InspectionError::Budget)?;
    enforce_json_budget(receipt, DAG_MAX_RESULT_BYTES, "lab_saved_receipt")
        .map_err(|_| InspectionError::Budget)?;
    let validated = plan.validate(catalog).map_err(InspectionError::Plan)?;
    if receipt.plan_hash != plan.hash || receipt.order.as_slice() != validated.execution_order() {
        return Err(InspectionError::Inconsistent("plan_hash_or_execution_order"));
    }

    let nodes: BTreeMap<_, _> = plan
        .nodes
        .iter()
        .map(|node| (node.instance.clone(), node))
        .collect();
    let expected_steps: BTreeSet<_> = nodes.keys().cloned().collect();
    if receipt.owner_results.keys().cloned().collect::<BTreeSet<_>>() != expected_steps {
        return Err(InspectionError::Inconsistent("owner_result_set"));
    }

    let mut expected_artifacts: BTreeMap<ArtifactId, bool> = plan
        .external_artifacts
        .keys()
        .cloned()
        .map(|id| (id, false))
        .collect();
    for node in &plan.nodes {
        for id in node.outputs.values() {
            if expected_artifacts.insert(id.clone(), true).is_some() {
                return Err(InspectionError::Inconsistent("duplicate_output_artifact"));
            }
        }
    }
    if receipt.artifacts.keys().cloned().collect::<BTreeSet<_>>()
        != expected_artifacts.keys().cloned().collect::<BTreeSet<_>>()
    {
        return Err(InspectionError::Inconsistent("artifact_set"));
    }
    let expected_produced: BTreeSet<_> = expected_artifacts
        .iter()
        .filter_map(|(id, produced)| produced.then_some(id.clone()))
        .collect();
    if receipt
        .produced_evidence
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected_produced
    {
        return Err(InspectionError::Inconsistent("produced_evidence_set"));
    }

    for (id, declared) in &plan.external_artifacts {
        let artifact = receipt
            .artifacts
            .get(id)
            .ok_or(InspectionError::Inconsistent("external_artifact_missing"))?;
        if artifact.schema != declared.schema
            || artifact.scope != declared.scope
            || artifact.provenance != declared.provenance
        {
            return Err(InspectionError::Inconsistent("external_artifact_metadata"));
        }
    }

    let mut steps = Vec::with_capacity(nodes.len());
    let mut dispatch_keys = BTreeSet::new();
    for instance in validated.execution_order() {
        let node = nodes[instance];
        let contract = catalog
            .contract(&node.operation)
            .ok_or(InspectionError::Inconsistent("unregistered_owner"))?;
        let result = &receipt.owner_results[instance];
        if !is_canonical_hash(&result.dispatch_execution_key)
            || !dispatch_keys.insert(result.dispatch_execution_key.clone())
            || result.receipt_bytes.is_empty()
            || result.outputs.keys().collect::<BTreeSet<_>>()
                != contract.outputs.keys().collect::<BTreeSet<_>>()
        {
            return Err(InspectionError::Inconsistent("owner_result_shape"));
        }
        let result_hash = canonical_owner_result_hash(result)
            .map_err(|_| InspectionError::Inconsistent("result_encoding"))?;
        let receipt_hash = owner_content_hash(&result.receipt_bytes);
        for (port, artifact_id) in &node.outputs {
            let actual = receipt
                .artifacts
                .get(artifact_id)
                .ok_or(InspectionError::Inconsistent("produced_artifact_missing"))?;
            let output = result
                .outputs
                .get(port)
                .ok_or(InspectionError::Inconsistent("result_output_missing"))?;
            let proof = receipt
                .produced_evidence
                .get(artifact_id)
                .ok_or(InspectionError::Inconsistent("produced_proof_missing"))?;
            if output != actual
                || contract.outputs.get(port) != Some(&actual.schema)
                || actual.scope != plan.scope
                || actual.provenance.trim().is_empty()
                || proof.producer != *instance
                || proof.port != *port
                || proof.artifact_id != *artifact_id
                || proof.dispatch_execution_key != result.dispatch_execution_key
                || proof.child_workflow_id
                    != owner_child_workflow_id_v2(&result.dispatch_execution_key)
                || proof.result_hash != result_hash
                || proof.receipt_hash != receipt_hash
                || proof.payload_hash != owner_content_hash(&actual.payload)
            {
                return Err(InspectionError::Inconsistent("produced_artifact_proof"));
            }
        }
        steps.push(StepSummary {
            instance: instance.clone(),
            operation: node.operation.clone(),
            owner: contract.owner.as_str().to_owned(),
            result_hash,
            receipt_hash,
            output_count: result.outputs.len(),
        });
    }

    let artifacts = receipt
        .artifacts
        .iter()
        .map(|(id, value)| ArtifactSummary {
            id: id.clone(),
            schema: value.schema.clone(),
            origin: if expected_artifacts[id] {
                "produced"
            } else {
                "external"
            },
            payload_hash: owner_content_hash(&value.payload),
            payload_bytes: value.payload.len(),
        })
        .collect();
    Ok(ExecutionInspection {
        status: "self_consistent_not_attested",
        tenant_id: plan.scope.tenant_id.clone(),
        run_id: plan.scope.run_id.clone(),
        plan_hash: plan.hash.clone(),
        steps,
        artifacts,
        temporal_history_attestation: "not_checked",
        owner_receipt_attestation: "not_checked",
        source_authentication: "not_checked",
    })
}

fn is_canonical_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Serialize)]
pub struct ArtifactChange {
    pub id: ArtifactId,
    pub change: &'static str,
    pub first_hash: Option<String>,
    pub second_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecutionComparison {
    pub status: &'static str,
    pub first_run_id: String,
    pub second_run_id: String,
    pub common_output_count: usize,
    pub unchanged_output_count: usize,
    pub changes: Vec<ArtifactChange>,
    pub seo_measurement: &'static str,
    pub attestation: &'static str,
}

/// Comparable means same *actual external bytes*, not merely equal recipe
/// declarations. This comparison never infers rankings, demand or quality.
pub fn compare_executions(
    first_plan: &ExecutionPlan,
    first_receipt: &OperationDagReceiptV1,
    second_plan: &ExecutionPlan,
    second_receipt: &OperationDagReceiptV1,
    catalog: &ContractCatalog,
) -> Result<ExecutionComparison, InspectionError> {
    let first = inspect_execution(first_plan, first_receipt, catalog)?;
    let second = inspect_execution(second_plan, second_receipt, catalog)?;
    if first.run_id == second.run_id && first.tenant_id == second.tenant_id {
        return Err(InspectionError::SameRun);
    }
    if first.tenant_id != second.tenant_id
        || first_plan.external_artifacts.is_empty()
        || first_plan.external_artifacts.len() != second_plan.external_artifacts.len()
        || first_plan.external_artifacts.iter().any(|(id, expected)| {
            let Some(other) = second_plan.external_artifacts.get(id) else {
                return true;
            };
            let Some(first_bytes) = first_receipt.artifacts.get(id) else {
                return true;
            };
            let Some(second_bytes) = second_receipt.artifacts.get(id) else {
                return true;
            };
            expected.schema != other.schema
                || expected.provenance != other.provenance
                || first_bytes.payload != second_bytes.payload
        })
    {
        return Err(InspectionError::DifferentSources);
    }
    let first_outputs: BTreeMap<_, _> = first
        .artifacts
        .iter()
        .filter(|artifact| artifact.origin == "produced")
        .map(|artifact| (artifact.id.clone(), artifact))
        .collect();
    let second_outputs: BTreeMap<_, _> = second
        .artifacts
        .iter()
        .filter(|artifact| artifact.origin == "produced")
        .map(|artifact| (artifact.id.clone(), artifact))
        .collect();
    let ids: BTreeSet<_> = first_outputs
        .keys()
        .chain(second_outputs.keys())
        .cloned()
        .collect();
    let mut common_output_count = 0;
    let mut unchanged_output_count = 0;
    let mut changes = Vec::new();
    for id in ids {
        let left = first_outputs.get(&id);
        let right = second_outputs.get(&id);
        let change = match (left, right) {
            (Some(a), Some(b)) if a.schema == b.schema => {
                common_output_count += 1;
                if a.payload_hash == b.payload_hash {
                    unchanged_output_count += 1;
                    continue;
                }
                "modified"
            }
            (Some(_), Some(_)) => "schema_changed",
            (Some(_), None) => "removed",
            (None, Some(_)) => "added",
            (None, None) => unreachable!(),
        };
        changes.push(ArtifactChange {
            id,
            change,
            first_hash: left.map(|artifact| artifact.payload_hash.clone()),
            second_hash: right.map(|artifact| artifact.payload_hash.clone()),
        });
    }
    Ok(ExecutionComparison {
        status: "same_source_bytes_structural_result_comparison",
        first_run_id: first.run_id,
        second_run_id: second.run_id,
        common_output_count,
        unchanged_output_count,
        changes,
        seo_measurement: "not_performed",
        attestation: "not_checked_against_temporal_or_provider",
    })
}
