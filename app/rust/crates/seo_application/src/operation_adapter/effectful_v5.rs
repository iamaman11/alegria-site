//! V5-only deterministic, dependency-aware sequencing of independently
//! attested Temporal owner children. The old V1/V4 Histories are frozen.
//!
//! This is a *typed owner route*, not an arbitrary RPC registry. A new effect
//! owner must supply a real Workflow/Activity, permission and receipt before
//! another route may be registered. At present the only route with an actual
//! externally committed business receipt is PostgreSQL Truth reconciliation.

use std::collections::BTreeSet;

use operation_protocol::{
    ArtifactBinding, CheckedDispatch, EffectChildInvocationV4, IdempotencyPolicy, Operation,
    OperationInstanceId, OperationNode, OperationOwnerId, TemporalExecutionIdentityV4,
};
use serde::{Deserialize, Serialize};
use truth_domain::review_effect_contract::ReconcileTruthReviewsContractV1;

use super::effectful_v1::{EffectDagError, OwnerCommandV1, PreparedDagStepV1};
use super::effectful_v4::{MixedDagMachineV4, MixedDagReceiptV4};

pub const ADMITTED_DAG_RECEIPT_SCHEMA_V5: u16 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedDagReceiptV5 {
    pub schema_revision: u16,
    pub result: MixedDagReceiptV4,
}

/// Closed, compile-time verified routing table. It deliberately has no
/// generic `owner_url`, SQL statement or caller-supplied Workflow type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEffectRouteV5 {
    PgTruthReviewV4,
}

impl NativeEffectRouteV5 {
    pub fn for_dispatch(dispatch: &CheckedDispatch) -> Result<Self, EffectDagError> {
        let contract = ReconcileTruthReviewsContractV1::contract();
        if dispatch.operation != contract.reference
            || dispatch.owner != OperationOwnerId::from("postgres_store")
            || dispatch.effects != contract.effects
            || dispatch.idempotency != IdempotencyPolicy::Idempotent
        {
            return Err(EffectDagError::UnsupportedOwner(dispatch.operation.clone()));
        }
        Ok(Self::PgTruthReviewV4)
    }
}

#[derive(Debug, Clone)]
pub struct IndependentOwnerChildV5 {
    pub checked: CheckedDispatch,
    pub route: NativeEffectRouteV5,
    pub invocation: EffectChildInvocationV4,
}

/// The V5 interpreter intentionally reuses the frozen V4 typed runtime for
/// dataflow, effect input fingerprints, PG receipt insertion and output
/// production. Only V5 chooses *which already-ready independent children*
/// may be started before waiting for their individual durable results.
pub struct MixedDagMachineV5 {
    inner: MixedDagMachineV4,
}

/// Transitive topological fence: both explicit sequencing-only edges and
/// data-producing edges establish ordering. Validation of the original plan
/// has already rejected missing producers/cycles before this walk.
fn depends_on(
    nodes: &[OperationNode],
    consumer: &OperationInstanceId,
    ancestor: &OperationInstanceId,
) -> bool {
    let mut pending = vec![consumer.clone()];
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        let node = nodes
            .iter()
            .find(|node| node.instance == current)
            .expect("validated DAG has every producer node");
        for previous in node
            .after
            .iter()
            .chain(node.inputs.values().filter_map(|input| match input {
                ArtifactBinding::External { .. } => None,
                ArtifactBinding::Produced { producer, .. } => Some(producer),
            }))
        {
            if previous == ancestor {
                return true;
            }
            pending.push(previous.clone());
        }
    }
    false
}

impl MixedDagMachineV5 {
    pub fn new(workflow_id: &str, run: super::EffectDagRunV1) -> Result<Self, EffectDagError> {
        let inner = MixedDagMachineV4::new(workflow_id, run)?;
        let run = &inner.run;
        // Check every admission ID before starting the *first* child. A
        // repeated ID across distinct operation instances would otherwise
        // allow an earlier irreversible effect before failing the second.
        let mut admissions = BTreeSet::new();
        let mut canonical_context = None::<&str>;
        for (instance, command) in &run.owner_commands {
            let OwnerCommandV1::PgTruthReviewReconcileV1(value) = command;
            if !admissions.insert(value.admission_id) {
                return Err(EffectDagError::DuplicateOwnerAdmission(instance.clone()));
            }
            if let Some(expected) = canonical_context {
                if expected != value.context_key {
                    // pipeline.execution_runs has one authoritative context
                    // per canonical run_id. A different context in a later
                    // child would fail only *after* an earlier PG effect.
                    return Err(EffectDagError::MixedOwnerContext(instance.clone()));
                }
            } else {
                canonical_context = Some(&value.context_key);
            }
        }
        let commands: Vec<_> = run.owner_commands.iter().collect();
        for (index, &(left_id, left_command)) in commands.iter().enumerate() {
            let OwnerCommandV1::PgTruthReviewReconcileV1(left) = left_command;
            for &(right_id, right_command) in commands.iter().skip(index + 1) {
                let OwnerCommandV1::PgTruthReviewReconcileV1(right) = right_command;
                if left.run_id == right.run_id
                    && left.context_key == right.context_key
                    && !depends_on(&run.plan.nodes, right_id, left_id)
                    && !depends_on(&run.plan.nodes, left_id, right_id)
                {
                    // The same PostgreSQL source/Truth effect cannot be
                    // declared independent by forgetting an `after` edge.
                    // Reject the whole DAG before starting either child.
                    return Err(EffectDagError::ConflictingOwnerWriteScope {
                        first: left_id.clone(),
                        second: right_id.clone(),
                    });
                }
            }
        }
        Ok(Self { inner })
    }

    pub fn next_step(&self) -> Option<&PreparedDagStepV1> {
        self.inner.next_step()
    }

    pub fn advance_pure(&mut self) -> Result<(), EffectDagError> {
        self.inner.advance_pure()
    }

    /// Return the maximal *contiguous* independently executable owner group.
    /// A barrier is inserted at every pure step, explicit `after` edge,
    /// produced-input edge, or overlapping (run, Truth context) write scope.
    /// Operators must authorize each distinct child from independent server
    /// Histories. A batch itself is never a bearer admission. With the single
    /// currently supported canonical Truth context and PG effect route, the
    /// accepted group length is always one. This API is a future-safe typed
    /// coordination boundary, NOT a claim of production multi-owner fanout.
    pub fn next_independent_children(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<Vec<IndependentOwnerChildV5>, EffectDagError> {
        let prepared = &self.inner.prepared;
        let run = &self.inner.run;
        let mut batch = Vec::new();
        let mut scheduled = BTreeSet::<OperationInstanceId>::new();
        let mut write_scopes = BTreeSet::<(String, String)>::new();
        for step in prepared.steps.iter().skip(self.inner.cursor) {
            let PreparedDagStepV1::Owner { instance, command, checked, .. } = step else {
                break;
            };
            let node = run
                .plan
                .nodes
                .iter()
                .find(|node| &node.instance == instance)
                .expect("validated owner instance");
            let has_batch_dependency = !node.after.is_disjoint(&scheduled)
                || node
                    .inputs
                    .values()
                    .any(|binding| match binding {
                        ArtifactBinding::External { .. } => false,
                        ArtifactBinding::Produced { producer, .. } => scheduled.contains(producer),
                    });
            let OwnerCommandV1::PgTruthReviewReconcileV1(input) = command;
            let write_scope = (input.run_id.to_string(), input.context_key.clone());
            // Same-source effects need a deterministic sequential barrier even
            // if the manifest forgot to declare an explicit ordering edge.
            if has_batch_dependency || write_scopes.contains(&write_scope) {
                break;
            }
            let invocation = self.inner.owner_invocation_at_v5(step, parent.clone())?;
            let route = NativeEffectRouteV5::for_dispatch(checked)?;
            batch.push(IndependentOwnerChildV5 {
                checked: checked.clone(),
                route,
                invocation,
            });
            scheduled.insert(instance.clone());
            write_scopes.insert(write_scope);
        }
        if batch.is_empty() {
            return Err(EffectDagError::WrongStepKind);
        }
        // A future second owner route must explicitly revise the operator
        // authorization/compensation policy before enabling parallel writes.
        // Never silently turn a new contract into irreversible fanout.
        if batch.len() != 1 {
            return Err(EffectDagError::ParallelOwnerBatchNotSupported);
        }
        Ok(batch)
    }

    pub fn advance_owner_receipt(
        &mut self,
        receipt: truth_domain::review_effect_contract::ReconcileTruthReviewsReceipt,
    ) -> Result<(), EffectDagError> {
        self.inner.advance_owner_receipt(receipt)
    }

    pub fn finish(self) -> Result<AdmittedDagReceiptV5, EffectDagError> {
        Ok(AdmittedDagReceiptV5 {
            schema_revision: ADMITTED_DAG_RECEIPT_SCHEMA_V5,
            result: self.inner.finish()?,
        })
    }
}
