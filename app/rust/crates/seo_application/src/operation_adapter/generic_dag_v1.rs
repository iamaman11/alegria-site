//! Stable provider-neutral DAG interpreter for trusted Temporal owner routes.
//!
//! Unlike the frozen site-specific V1–V5 workflows, this module imports no
//! PostgreSQL/Qdrant/Neo4j operation DTOs, SDKs or domain policies. Every
//! operation, even a pure one, executes through an independently registered
//! typed Temporal owner child. New owners require no changes to this engine.
//!
//! Whole-plan preflight proves STATIC port schemas, ordering, scopes, effects,
//! contracts and trusted routes before any child starts. It does NOT prove
//! concrete business DTO bytes for future owners: a planned late command can
//! still fail domain validation after an earlier owner effect. Consequently
//! this first serial runner is NOT a production-ready cross-owner atomic unit;
//! staged effectful rollouts must use owner-side validation-only preflight or
//! explicitly accept partial completion and compensating operations.
//! A result produced by
//! an effect is not known in advance: its actual bytes are only materialized
//! after an independently admitted child completes with a checked receipt.
//! The next owner MUST decode its own Rust Input before any external effect.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    canonical_owner_dispatch_input_bytes, AdmittedArtifactV1, ArtifactBinding, ArtifactId,
    BoundArtifactV1, CheckedDispatch, DispatchAttempt, OperationDagRunV6,
    OperationInstanceId, OwnerInvocationV1, OwnerResultV1, PinnedOwnerRouteV1, PlanError,
    PlanHash, PortId, RouteCatalogError, TemporalExecutionIdentityV4, TrustedRouteCatalog,
    ValidatedPlan, OWNER_INVOCATION_SCHEMA_REVISION,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{enforce_json_budget, OperationRunError, PURE_DAG_MAX_INPUT_BYTES,
    PURE_DAG_MAX_NODES, PURE_DAG_MAX_RESULT_BYTES};

#[derive(Debug, Error)]
pub enum GenericDagError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Route(#[from] RouteCatalogError),
    #[error(transparent)]
    Run(#[from] OperationRunError),
    #[error("generic DAG has no operations")]
    EmptyPlan,
    #[error("Temporal Workflow ID differs from immutable DAG run scope")]
    TemporalRunIdMismatch,
    #[error("DAG has missing or unexpected owner command at {0:?}")]
    InvalidCommand(OperationInstanceId),
    #[error("owner command at {0:?} is not pinned to its trusted contract")]
    CommandContractMismatch(OperationInstanceId),
    #[error("artifact {0:?} is missing for the next checked operation")]
    MissingProducedArtifact(ArtifactId),
    #[error("artifact {0:?} has incorrect scope, schema or provenance")]
    InvalidArtifact(ArtifactId),
    #[error("generic DAG finished before every node completed")]
    Incomplete,
}

/// Immutable, typed final result of one fully completed operation DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDagReceiptV1 {
    pub plan_hash: PlanHash,
    pub order: Vec<OperationInstanceId>,
    pub artifacts: BTreeMap<ArtifactId, AdmittedArtifactV1>,
    pub owner_results: BTreeMap<OperationInstanceId, OwnerResultV1>,
}

/// Serial-by-default durable runner. Parallel groups can only be enabled
/// after proving actual owner independence, admission and compensation.
pub struct OperationDagMachine {
    run: OperationDagRunV6,
    routes: TrustedRouteCatalog,
    validated: ValidatedPlan,
    artifacts: BTreeMap<ArtifactId, AdmittedArtifactV1>,
    owner_results: BTreeMap<OperationInstanceId, OwnerResultV1>,
    cursor: usize,
}

impl OperationDagMachine {
    pub fn new(
        temporal_workflow_id: &str,
        run: OperationDagRunV6,
        routes: TrustedRouteCatalog,
    ) -> Result<Self, GenericDagError> {
        if temporal_workflow_id != run.plan.scope.run_id {
            return Err(GenericDagError::TemporalRunIdMismatch);
        }
        if run.plan.nodes.is_empty() {
            return Err(GenericDagError::EmptyPlan);
        }
        if run.plan.nodes.len() > PURE_DAG_MAX_NODES {
            return Err(OperationRunError::ResourceBudgetExceeded {
                resource: "operation_nodes",
                limit: PURE_DAG_MAX_NODES,
            }.into());
        }
        enforce_json_budget(&run, PURE_DAG_MAX_INPUT_BYTES, "generic_dag_input")?;
        routes.validate_run_routes(&run)?;
        let validated = run.plan.validate(routes.catalog())?;
        for (id, expected) in &run.plan.external_artifacts {
            let actual = run.admitted.get(id)
                .ok_or_else(|| GenericDagError::InvalidArtifact(id.clone()))?;
            if actual.schema != expected.schema
                || actual.scope != expected.scope
                || actual.provenance != expected.provenance
                || actual.provenance.trim().is_empty()
            {
                return Err(GenericDagError::InvalidArtifact(id.clone()));
            }
        }
        if run.admitted.len() != run.plan.external_artifacts.len() {
            return Err(GenericDagError::InvalidArtifact(
                run.admitted.keys().find(|key|
                    !run.plan.external_artifacts.contains_key(*key))
                    .cloned().unwrap_or_else(|| ArtifactId::from("missing-external")),
            ));
        }
        // Each operation, not only externally effectful operations, has one
        // trusted owner command. The kernel knows no provider-specific type.
        if run.owner_commands.len() != run.plan.nodes.len() {
            return Err(GenericDagError::InvalidCommand(
                run.plan.nodes.first().expect("nonempty DAG").instance.clone(),
            ));
        }
        let mut seen = BTreeSet::new();
        for node in &run.plan.nodes {
            let command = run.owner_commands.get(&node.instance)
                .ok_or_else(|| GenericDagError::InvalidCommand(node.instance.clone()))?;
            let contract = routes.catalog().contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            if command.operation != node.operation
                || command.owner != contract.owner
                || command.contract_hash != contract.fingerprint()?
                || command.command_bytes.is_empty()
            {
                return Err(GenericDagError::CommandContractMismatch(node.instance.clone()));
            }
            if !seen.insert(node.instance.clone()) {
                return Err(GenericDagError::InvalidCommand(node.instance.clone()));
            }
        }
        for instance in run.owner_commands.keys() {
            if !seen.contains(instance) {
                return Err(GenericDagError::InvalidCommand(instance.clone()));
            }
        }
        Ok(Self {
            artifacts: run.admitted.clone(),
            run,
            routes,
            validated,
            owner_results: BTreeMap::new(),
            cursor: 0,
        })
    }

    pub fn next_instance(&self) -> Option<&OperationInstanceId> {
        self.validated.execution_order().get(self.cursor)
    }

    fn checked_input(
        &self,
    ) -> Result<(CheckedDispatch, BTreeMap<PortId, BoundArtifactV1>), GenericDagError> {
        let instance = self.next_instance().ok_or(GenericDagError::Incomplete)?;
        let node = self.run.plan.nodes.iter()
            .find(|node| &node.instance == instance).expect("validated node");
        let contract = self.routes.catalog().contract(&node.operation)
            .expect("validated owner contract");
        let command = self.run.owner_commands.get(instance)
            .expect("preflight command");
        let mut bound = BTreeMap::new();
        for (port, source) in &node.inputs {
            let id = match source {
                ArtifactBinding::External { artifact }
                | ArtifactBinding::Produced { artifact, .. } => artifact,
            };
            let actual = self.artifacts.get(id)
                .ok_or_else(|| GenericDagError::MissingProducedArtifact(id.clone()))?;
            if contract.inputs.get(port) != Some(&actual.schema)
                || actual.scope != self.run.plan.scope
                || actual.provenance.trim().is_empty()
            {
                return Err(GenericDagError::InvalidArtifact(id.clone()));
            }
            bound.insert(port.clone(), BoundArtifactV1 {
                artifact_id: id.clone(),
                source: source.clone(),
                artifact: actual.clone(),
            });
        }
        let actual_bytes = canonical_owner_dispatch_input_bytes(
            &bound, &command.command_bytes,
        ).map_err(|error| OperationRunError::EnvelopeSerialization(error.to_string()))?;
        let dispatch = self.validated.checked_dispatch(
            instance, &actual_bytes, DispatchAttempt::First,
        )?;
        self.routes.route_for_dispatch(&dispatch)?;
        Ok((dispatch, bound))
    }

    pub fn next_owner_invocation(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<(PinnedOwnerRouteV1, OwnerInvocationV1), GenericDagError> {
        let instance = self.next_instance().ok_or(GenericDagError::Incomplete)?;
        let command = self.run.owner_commands.get(instance)
            .expect("preflight owner command");
        let (checked, bound_inputs) = self.checked_input()?;
        let route = self.routes.route_for_dispatch(&checked)?.clone();
        let invocation = OwnerInvocationV1 {
            schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
            parent,
            checked_dispatch: checked,
            command_bytes: command.command_bytes.clone(),
            bound_inputs,
        };
        invocation.verify_integrity(&self.routes)?;
        Ok((route, invocation))
    }

    /// The result is genuinely returned by the selected Temporal child.
    /// Validate all output schemas and bind exactly the declared artifacts.
    /// Dynamic producer payloads are never guessed in the static preflight.
    /// Budget-check the projected *entire* receipt before publishing any
    /// artifact, because a finish-only budget check would be too late to stop
    /// subsequent external effects. Already committed owners cannot roll back.
    pub fn advance_owner_result(&mut self, result: OwnerResultV1)
        -> Result<(), GenericDagError>
    {
        let instance = self.next_instance().ok_or(GenericDagError::Incomplete)?.clone();
        let (checked, _) = self.checked_input()?;
        result.verify_integrity(&checked, &self.routes)?;
        enforce_json_budget(&result, PURE_DAG_MAX_RESULT_BYTES, "owner_result_bytes")?;
        let node = self.run.plan.nodes.iter()
            .find(|node| node.instance == instance).expect("validated DAG node");
        let mut next_artifacts = self.artifacts.clone();
        for (port, value) in &result.outputs {
            let id = node.outputs.get(port).expect("validated output port");
            if next_artifacts.contains_key(id) {
                return Err(GenericDagError::InvalidArtifact(id.clone()));
            }
            next_artifacts.insert(id.clone(), value.clone());
        }
        let mut next_results = self.owner_results.clone();
        next_results.insert(instance, result);
        let prospective_receipt = OperationDagReceiptV1 {
            plan_hash: self.run.plan.hash.clone(),
            order: self.validated.execution_order().to_vec(),
            artifacts: next_artifacts.clone(),
            owner_results: next_results.clone(),
        };
        enforce_json_budget(&prospective_receipt, PURE_DAG_MAX_RESULT_BYTES,
            "generic_dag_result")?;
        self.artifacts = next_artifacts;
        self.owner_results = next_results;
        self.cursor += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<OperationDagReceiptV1, GenericDagError> {
        if self.cursor != self.validated.execution_order().len() {
            return Err(GenericDagError::Incomplete);
        }
        let receipt = OperationDagReceiptV1 {
            plan_hash: self.run.plan.hash,
            order: self.validated.execution_order().to_vec(),
            artifacts: self.artifacts,
            owner_results: self.owner_results,
        };
        enforce_json_budget(&receipt, PURE_DAG_MAX_RESULT_BYTES, "generic_dag_result")?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        ArtifactSchema, ArtifactTypeId, ContractHash, EffectId, ExecutionPlan,
        IdempotencyPolicy, OperationContract, OperationId, OperationNode, OperationOwnerId,
        OperationRef, OwnerCommandWireV1, PLAN_SCHEMA_REVISION, PlanHash, RunScope,
        OWNER_OPERATION_CHILD_TYPE_V1, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
    };

    fn scope() -> RunScope {
        RunScope { tenant_id: "tenant-a".into(), run_id: "run-a".into() }
    }

    fn schema() -> ArtifactSchema {
        ArtifactSchema { id: ArtifactTypeId::from("validated.document"), revision: 1 }
    }

    fn contract(name: &str, input: bool) -> OperationContract {
        OperationContract {
            reference: OperationRef { id: OperationId(name.into()), revision: 1 },
            owner: OperationOwnerId(format!("{name}.owner")),
            inputs: if input { BTreeMap::from([(PortId::from("source"), schema())]) }
                else { BTreeMap::new() },
            outputs: BTreeMap::from([(PortId::from("document"), schema())]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }

    fn setup() -> (OperationDagRunV6, TrustedRouteCatalog) {
        let mut routes = TrustedRouteCatalog::default();
        let mut commands = BTreeMap::new();
        for name in ["producer", "consumer"] {
            let contract = contract(name, name == "consumer");
            let hash = contract.fingerprint().unwrap();
            commands.insert(OperationInstanceId::from(name), OwnerCommandWireV1 {
                operation: contract.reference.clone(), owner: contract.owner.clone(),
                contract_hash: hash.clone(), command_bytes: format!("{name}-typed-command").into_bytes(),
            });
            routes.register(contract.clone(), PinnedOwnerRouteV1 {
                operation: contract.reference, owner: contract.owner, contract_hash: hash,
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: format!("{name}-independent-owner-queue"),
                build_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            }).unwrap();
        }
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION, scope: scope(),
            allowed_effects: BTreeSet::<EffectId>::new(),
            external_artifacts: BTreeMap::new(), pinned_contracts: vec![],
            nodes: vec![
                OperationNode {
                    instance: "producer".into(),
                    operation: contract("producer", false).reference,
                    inputs: BTreeMap::new(),
                    outputs: BTreeMap::from([("document".into(), "artifact-a".into())]),
                    after: BTreeSet::new(), permitted_effects: BTreeSet::new(),
                },
                OperationNode {
                    instance: "consumer".into(),
                    operation: contract("consumer", true).reference,
                    inputs: BTreeMap::from([("source".into(), ArtifactBinding::Produced {
                        producer: "producer".into(), port: "document".into(),
                        artifact: "artifact-a".into(),
                    })]),
                    outputs: BTreeMap::from([("document".into(), "artifact-b".into())]),
                    after: BTreeSet::new(), permitted_effects: BTreeSet::new(),
                },
            ],
            hash: PlanHash::from("unsealed"),
        }.seal(routes.catalog()).unwrap();
        let pinned_routes = routes.trusted_snapshot_for(
            commands.values().map(|command| &command.operation),
        ).unwrap();
        (OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan, admitted: BTreeMap::new(), owner_commands: commands, pinned_routes,
        }, routes)
    }

    fn parent() -> TemporalExecutionIdentityV4 {
        TemporalExecutionIdentityV4 {
            workflow_id: "run-a".into(), run_id: "temporal-server-run".into(),
        }
    }

    fn result(invocation: &OwnerInvocationV1, payload: &[u8]) -> OwnerResultV1 {
        OwnerResultV1 {
            dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
            outputs: BTreeMap::from([("document".into(), AdmittedArtifactV1 {
                schema: schema(), scope: scope(), provenance: "attested-child-receipt".into(),
                payload: payload.to_vec(),
            })]),
            receipt_bytes: b"independent-operator-receipt".to_vec(),
        }
    }

    #[test]
    fn materializes_actual_producer_bytes_as_strictly_bound_consumer_input() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new("run-a", run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        assert!(producer.bound_inputs.is_empty());
        machine.advance_owner_result(result(&producer, b"real-owner-payload")).unwrap();

        let (_, consumer) = machine.next_owner_invocation(parent()).unwrap();
        let binding = consumer.bound_inputs.get(&PortId::from("source")).unwrap();
        assert_eq!(binding.artifact_id, ArtifactId::from("artifact-a"));
        assert_eq!(binding.artifact.payload, b"real-owner-payload");
        assert_eq!(binding.source, ArtifactBinding::Produced {
            producer: "producer".into(), port: "document".into(),
            artifact: "artifact-a".into(),
        });
        assert_ne!(producer.checked_dispatch.execution_key, consumer.checked_dispatch.execution_key);
        machine.advance_owner_result(result(&consumer, b"second-owner-output")).unwrap();
        let completed = machine.finish().unwrap();
        assert_eq!(completed.artifacts[&ArtifactId::from("artifact-b")].payload,
            b"second-owner-output");
    }

    #[test]
    fn preflight_rejects_forged_route_and_owner_contract() {
        let (mut run, routes) = setup();
        run.pinned_routes[0].task_queue = "attacker-queue".into();
        assert!(matches!(OperationDagMachine::new("run-a", run, routes),
            Err(GenericDagError::Route(RouteCatalogError::UntrustedRouteSnapshot))));

        let (mut run, routes) = setup();
        run.owner_commands.get_mut(&OperationInstanceId::from("producer"))
            .unwrap().contract_hash = ContractHash::from("forged");
        assert!(matches!(OperationDagMachine::new("run-a", run, routes),
            Err(GenericDagError::CommandContractMismatch(_))));
    }

    #[test]
    fn rejects_wrong_owner_output_schema_scope_or_missing_port_before_publication() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new("run-a", run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        let mut forged = result(&producer, b"real-owner-payload");
        forged.outputs.get_mut(&PortId::from("document")).unwrap().schema.revision = 4;
        assert!(matches!(machine.advance_owner_result(forged),
            Err(GenericDagError::Route(RouteCatalogError::InvalidOwnerResult))));

        let mut forged = result(&producer, b"real-owner-payload");
        forged.outputs.get_mut(&PortId::from("document")).unwrap().scope.tenant_id = "other".into();
        assert!(machine.advance_owner_result(forged).is_err());

        let mut forged = result(&producer, b"real-owner-payload");
        forged.outputs.clear();
        assert!(machine.advance_owner_result(forged).is_err());
        // Rejections never advance the cursor or publish attacker artifacts.
        assert_eq!(machine.next_instance(), Some(&OperationInstanceId::from("producer")));
    }

    #[test]
    fn rejects_unbound_produced_edge_with_incompatible_static_input_schema() {
        let (mut run, routes) = setup();
        run.plan.nodes[1].inputs.insert("source".into(), ArtifactBinding::Produced {
            producer: "producer".into(), port: "document".into(),
            artifact: "artifact-b".into(),
        });
        run.plan.hash = run.plan.canonical_hash().unwrap();
        assert!(OperationDagMachine::new("run-a", run, routes).is_err());
    }

    #[test]
    fn oversized_child_result_never_publishes_an_artifact_or_advances_dag() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new("run-a", run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        let excessive = result(&producer, &vec![0xff; PURE_DAG_MAX_RESULT_BYTES]);
        assert!(matches!(machine.advance_owner_result(excessive),
            Err(GenericDagError::Run(OperationRunError::ResourceBudgetExceeded { .. }))));
        assert_eq!(machine.next_instance(), Some(&OperationInstanceId::from("producer")));
    }
}
