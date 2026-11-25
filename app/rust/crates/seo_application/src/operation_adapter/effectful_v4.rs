//! Deterministic mixed-operation interpreter; owner execution happens in a
//! native Temporal child Workflow with separately attested PG admission.
//!
//! This module has no Temporal, SQL or provider dependency. Pure operations
//! are evaluated directly. Effects only produce exact versioned child-command
//! envelopes; the interpreter advances past an effect exclusively on a typed
//! receipt whose durable identity is verified by the owner bridge.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactBinding, ArtifactId, BoundEffectInputV4, EffectChildInvocationV4, OperationInstanceId,
    PlanHash, PortId, TemporalExecutionIdentityV4, EFFECT_CHILD_SCHEMA_REVISION,
};
use serde::{Deserialize, Serialize};
use truth_domain::review_effect_contract::{ReconcileTruthReviewsReceipt, OUTPUT_PORT};

use super::effectful_v1::{
    preflight_effect_dag_v1, require_canonical_owner_inputs_v4, EffectDagError, EffectDagRunV1,
    OwnerCommandV1, PreparedDagStepV1, PreparedDagV1,
};
use super::{
    enforce_json_budget, semantic_extraction_registry, ArtifactValue, OperationRegistry,
    OperationRunError, Payloads, PURE_DAG_MAX_RESULT_BYTES,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixedDagReceiptV4 {
    pub plan_hash: PlanHash,
    pub order: Vec<OperationInstanceId>,
    pub artifacts: BTreeMap<ArtifactId, ArtifactValue>,
    /// Durable typed PG owner receipts, never manifest-provided success flags.
    pub owner_receipts: BTreeMap<OperationInstanceId, ReconcileTruthReviewsReceipt>,
}

pub struct MixedDagMachineV4 {
    run: EffectDagRunV1,
    prepared: PreparedDagV1,
    registry: OperationRegistry,
    artifacts: BTreeMap<ArtifactId, ArtifactValue>,
    receipts: BTreeMap<OperationInstanceId, ReconcileTruthReviewsReceipt>,
    cursor: usize,
}

impl MixedDagMachineV4 {
    /// Complete whole-DAG preflight before the first native child Workflow.
    /// All unsupported owner routes, missing external bytes, stale contracts,
    /// malformed effect commands and source/command mismatches fail here.
    pub fn new(workflow_id: &str, run: EffectDagRunV1) -> Result<Self, EffectDagError> {
        let prepared = preflight_effect_dag_v1(workflow_id, &run)?;
        require_canonical_owner_inputs_v4(&run, &prepared)?;
        let registry = semantic_extraction_registry()?;
        // Before starting any child, dry-run every pure transformation in
        // scratch memory. A later pure input decode/output/budget failure
        // must not be discovered only AFTER an irreversible PG owner effect.
        // This is safe: these handlers have `IdempotencyPolicy::Pure`, and
        // their inputs may reference external/pure-produced artifacts only.
        // An effect-produced input that cannot be previewed fails closed.
        let mut preview = Self {
            artifacts: run.admitted.clone(),
            run: run.clone(),
            prepared: prepared.clone(),
            registry: semantic_extraction_registry()?,
            receipts: BTreeMap::new(),
            cursor: 0,
        };
        while let Some(step) = preview.next_step().cloned() {
            match step {
                PreparedDagStepV1::Pure { .. } => preview.advance_pure()?,
                PreparedDagStepV1::Owner { .. } => preview.cursor += 1,
            }
        }
        enforce_json_budget(
            &preview.artifacts,
            PURE_DAG_MAX_RESULT_BYTES,
            "preflight_pure_result_bytes",
        )?;
        Ok(Self {
            artifacts: run.admitted.clone(),
            run,
            prepared,
            registry,
            receipts: BTreeMap::new(),
            cursor: 0,
        })
    }

    pub fn next_step(&self) -> Option<&PreparedDagStepV1> {
        self.prepared.steps.get(self.cursor)
    }

    /// Execute the next registered pure transformation in-memory. Each result
    /// receives the same canonical schema/scope/provenance as the pure runner.
    pub fn advance_pure(&mut self) -> Result<(), EffectDagError> {
        let (instance, operation) = match self.next_step() {
            Some(PreparedDagStepV1::Pure {
                instance,
                operation,
            }) => (instance.clone(), operation.clone()),
            _ => return Err(EffectDagError::WrongStepKind),
        };
        let node = self
            .run
            .plan
            .nodes
            .iter()
            .find(|node| node.instance == instance)
            .expect("preflight validates each scheduled node");
        let handler = self
            .registry
            .handlers
            .get(&operation)
            .ok_or_else(|| OperationRunError::MissingExecutor(operation.clone()))?;
        let mut inputs = Payloads::new();
        for (port, binding) in &node.inputs {
            let artifact = match binding {
                ArtifactBinding::External { artifact }
                | ArtifactBinding::Produced { artifact, .. } => artifact,
            };
            let value = self
                .artifacts
                .get(artifact)
                .ok_or_else(|| OperationRunError::MissingArtifact(artifact.clone()))?;
            inputs.insert(port.clone(), (artifact.clone(), value.payload.clone()));
        }
        let mut outputs = handler(&inputs)?;
        let contract = self
            .registry
            .catalog
            .contract(&operation)
            .expect("preflight pins concrete operation contract");
        let ports: BTreeSet<_> = contract.outputs.keys().cloned().collect();
        if outputs.keys().cloned().collect::<BTreeSet<_>>() != ports {
            let wrong = outputs
                .keys()
                .find(|port| !ports.contains(*port))
                .cloned()
                .or_else(|| {
                    ports
                        .iter()
                        .find(|port| !outputs.contains_key(*port))
                        .cloned()
                })
                .expect("different output port sets have a difference");
            return Err(OperationRunError::InvalidOutputPort {
                operation,
                port: wrong,
            }
            .into());
        }
        for (port, artifact_id) in &node.outputs {
            let schema = contract
                .outputs
                .get(port)
                .expect("preflight validates producer port");
            let payload = outputs.remove(port).expect("output ports checked");
            self.artifacts.insert(
                artifact_id.clone(),
                ArtifactValue {
                    schema: schema.clone(),
                    scope: self.run.plan.scope.clone(),
                    provenance: format!(
                        "operation:{}@{}/{}#{}",
                        operation.id.as_str(),
                        operation.revision,
                        instance.as_str(),
                        self.run.plan.hash.as_str(),
                    ),
                    payload,
                },
            );
        }
        self.cursor += 1;
        Ok(())
    }

    /// Build exactly the bytes the child Workflow must record in its Started
    /// event. The independently verified operator uses both Temporal Histories
    /// and this fingerprint before issuing admission. This is not a permit.
    pub fn next_owner_child_invocation(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<EffectChildInvocationV4, EffectDagError> {
        let (bound_input, command, checked) = match self.next_step() {
            Some(PreparedDagStepV1::Owner {
                bound_input,
                command,
                checked,
                ..
            }) => (bound_input, command, checked),
            _ => return Err(EffectDagError::WrongStepKind),
        };
        let value = self
            .artifacts
            .get(bound_input)
            .ok_or_else(|| OperationRunError::MissingArtifact(bound_input.clone()))?;
        let bound = BoundEffectInputV4 {
            artifact_id: bound_input.clone(),
            schema: value.schema.clone(),
            scope: value.scope.clone(),
            provenance: value.provenance.clone(),
            payload: value.payload.clone(),
        };
        let OwnerCommandV1::PgTruthReviewReconcileV1(input) = command;
        let owner_command_bytes = serde_json::to_vec(input)
            .map_err(|error| OperationRunError::EnvelopeSerialization(error.to_string()))?;
        let invocation = EffectChildInvocationV4 {
            schema_revision: EFFECT_CHILD_SCHEMA_REVISION,
            parent,
            checked_dispatch: checked.clone(),
            bound_input: bound,
            owner_command_bytes,
        };
        invocation
            .verify_integrity()
            .map_err(|error| EffectDagError::InvalidInvocation(error.to_string()))?;
        Ok(invocation)
    }

    /// Only the typed child outcome received from Temporal may advance the
    /// graph. The PG owner independently issued and atomically recorded this
    /// receipt; this parent checks its exact expected identity and outputs.
    pub fn advance_owner_receipt(
        &mut self,
        receipt: ReconcileTruthReviewsReceipt,
    ) -> Result<(), EffectDagError> {
        let (instance, command, checked) = match self.next_step() {
            Some(PreparedDagStepV1::Owner {
                instance,
                command,
                checked,
                ..
            }) => (instance.clone(), command.clone(), checked.clone()),
            _ => return Err(EffectDagError::WrongStepKind),
        };
        let OwnerCommandV1::PgTruthReviewReconcileV1(input) = command;
        if receipt.execution_key != checked.execution_key
            || receipt.admission_id != input.admission_id
            || receipt.run_id != input.run_id
            || receipt.context_key != input.context_key
            || receipt.receipt_id.is_nil()
            || receipt.candidate_set_hash.trim().is_empty()
            || receipt.created_review_tasks < 0
            || receipt.open_review_tasks < 0
        {
            return Err(EffectDagError::InvalidOwnerReceipt(instance));
        }
        let node = self
            .run
            .plan
            .nodes
            .iter()
            .find(|node| node.instance == instance)
            .expect("validated operation instance");
        let output_id = node
            .outputs
            .get(&PortId::from(OUTPUT_PORT))
            .expect("contract-pinned output port");
        let payload = serde_json::to_vec(&receipt)
            .map_err(|error| OperationRunError::EnvelopeSerialization(error.to_string()))?;
        self.artifacts.insert(
            output_id.clone(),
            ArtifactValue {
                schema: truth_domain::review_effect_contract::operation_contract().outputs
                    [&PortId::from(OUTPUT_PORT)]
                    .clone(),
                scope: self.run.plan.scope.clone(),
                provenance: format!(
                    "owner-receipt:{}:{}:{}",
                    checked.owner.as_str(),
                    instance.as_str(),
                    checked.execution_key,
                ),
                payload,
            },
        );
        self.receipts.insert(instance, receipt);
        self.cursor += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<MixedDagReceiptV4, EffectDagError> {
        if self.cursor != self.prepared.steps.len() {
            return Err(EffectDagError::IncompleteDag);
        }
        let receipt = MixedDagReceiptV4 {
            plan_hash: self.run.plan.hash,
            order: self
                .prepared
                .steps
                .iter()
                .map(|step| match step {
                    PreparedDagStepV1::Pure { instance, .. }
                    | PreparedDagStepV1::Owner { instance, .. } => instance.clone(),
                })
                .collect(),
            artifacts: self.artifacts,
            owner_receipts: self.receipts,
        };
        enforce_json_budget(&receipt, PURE_DAG_MAX_RESULT_BYTES, "result_bytes")?;
        Ok(receipt)
    }
}
