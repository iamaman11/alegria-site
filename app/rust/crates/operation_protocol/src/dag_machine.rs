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

use crate::{
    canonical_owner_dispatch_input_bytes, canonical_owner_result_hash,
    operation_dag_parent_workflow_id, operation_dag_parent_workflow_id_v2, owner_child_workflow_id,
    owner_child_workflow_id_v2, owner_content_hash, AdmittedArtifactV1, ArtifactBinding,
    ArtifactId, BoundArtifactV1, CheckedDispatch, DispatchAttempt, OperationDagRunV6,
    OperationInstanceId, OwnerInvocationV1, OwnerResultV1, OwnerValidationRequestV1,
    OwnerValidationResultV1, PinnedOwnerRouteV1, PlanError, PlanHash, PortId,
    ProducedArtifactEvidenceV1, RouteCatalogError, TemporalExecutionIdentityV4,
    TrustedRouteCatalog, ValidatedPlan, OWNER_INVOCATION_SCHEMA_REVISION,
    OWNER_VALIDATION_SCHEMA_REVISION_V1,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::json_budget::{
    enforce_json_budget, JsonBudgetError, DAG_MAX_INPUT_BYTES as PURE_DAG_MAX_INPUT_BYTES,
    DAG_MAX_NODES as PURE_DAG_MAX_NODES, DAG_MAX_RESULT_BYTES as PURE_DAG_MAX_RESULT_BYTES,
};

#[derive(Debug, Error)]
pub enum GenericDagError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Route(#[from] RouteCatalogError),
    #[error(transparent)]
    Run(#[from] JsonBudgetError),
    #[error("generic DAG has no operations")]
    EmptyPlan,
    #[error("Temporal parent Workflow ID is not the canonical generic DAG ID for its business run scope")]
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
    #[error("parallel V2 parent cannot execute unadmitted effectful operation {0:?}")]
    UnadmittedV2Effect(OperationInstanceId),
    #[error("V2 owner-local ValidateOnly preflight did not complete before effect dispatch")]
    OwnerPreflightPending,
}

/// Immutable, typed final result of one fully completed operation DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDagReceiptV1 {
    pub plan_hash: PlanHash,
    pub order: Vec<OperationInstanceId>,
    pub artifacts: BTreeMap<ArtifactId, AdmittedArtifactV1>,
    /// Deterministic evidence of each upstream Temporal child result.
    /// Actual server History/receipt corroboration remains the owner's duty.
    pub produced_evidence: BTreeMap<ArtifactId, ProducedArtifactEvidenceV1>,
    pub owner_results: BTreeMap<OperationInstanceId, OwnerResultV1>,
}

/// Deterministic durable runner. A ready wave may start independent children
/// concurrently, but each child still has its own History, operator admission
/// and receipt. A failed sibling never implies a rollback of another owner.
pub struct OperationDagMachine {
    run: OperationDagRunV6,
    routes: TrustedRouteCatalog,
    validated: ValidatedPlan,
    artifacts: BTreeMap<ArtifactId, AdmittedArtifactV1>,
    produced_evidence: BTreeMap<ArtifactId, ProducedArtifactEvidenceV1>,
    owner_results: BTreeMap<OperationInstanceId, OwnerResultV1>,
    cursor: usize,
    v2: bool,
    validation_required: bool,
    validation_completed: bool,
}

impl OperationDagMachine {
    pub fn new(
        temporal_workflow_id: &str,
        run: OperationDagRunV6,
        routes: TrustedRouteCatalog,
    ) -> Result<Self, GenericDagError> {
        let expected_id = operation_dag_parent_workflow_id(&run.plan.scope.run_id);
        Self::new_for_identity(temporal_workflow_id, expected_id, run, routes, false)
    }

    /// A completely different Temporal parent Workflow Type/ID is required
    /// for parallel scheduling. Until independent owner operators support
    /// V2 History admission, only trusted *pure* owner contracts are legal.
    pub fn new_v2(
        temporal_workflow_id: &str,
        run: OperationDagRunV6,
        routes: TrustedRouteCatalog,
    ) -> Result<Self, GenericDagError> {
        let expected_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let machine = Self::new_for_identity(temporal_workflow_id, expected_id, run, routes, true)?;
        for node in &machine.run.plan.nodes {
            let contract = machine
                .routes
                .catalog()
                .contract(&node.operation)
                .expect("preflight registered the contract");
            if !contract.effects.is_empty()
                || !node.permitted_effects.is_empty()
                || contract.idempotency != crate::IdempotencyPolicy::Pure
            {
                return Err(GenericDagError::UnadmittedV2Effect(node.instance.clone()));
            }
        }
        Ok(machine)
    }

    /// Effectful V2 is **not** authorized by this constructor. It builds a
    /// strictly preflight-gated machine: the caller must first receive and
    /// verify one pure typed ValidateOnly Child result from EVERY owner, and
    /// each external effect still needs independently attested V2 admission.
    pub fn new_v2_with_owner_validation(
        temporal_workflow_id: &str,
        run: OperationDagRunV6,
        routes: TrustedRouteCatalog,
    ) -> Result<Self, GenericDagError> {
        let expected_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let mut machine =
            Self::new_for_identity(temporal_workflow_id, expected_id, run, routes, true)?;
        machine.validation_required = true;
        Ok(machine)
    }

    fn new_for_identity(
        temporal_workflow_id: &str,
        expected_id: String,
        run: OperationDagRunV6,
        routes: TrustedRouteCatalog,
        v2: bool,
    ) -> Result<Self, GenericDagError> {
        if temporal_workflow_id != expected_id {
            return Err(GenericDagError::TemporalRunIdMismatch);
        }
        if run.plan.nodes.is_empty() {
            return Err(GenericDagError::EmptyPlan);
        }
        if run.plan.nodes.len() > PURE_DAG_MAX_NODES {
            return Err(JsonBudgetError::ResourceBudgetExceeded {
                resource: "operation_nodes",
                limit: PURE_DAG_MAX_NODES,
            }
            .into());
        }
        enforce_json_budget(&run, PURE_DAG_MAX_INPUT_BYTES, "generic_dag_input")?;
        routes.validate_run_routes(&run)?;
        let validated = run.plan.validate(routes.catalog())?;
        for (id, expected) in &run.plan.external_artifacts {
            let actual = run
                .admitted
                .get(id)
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
                run.admitted
                    .keys()
                    .find(|key| !run.plan.external_artifacts.contains_key(*key))
                    .cloned()
                    .unwrap_or_else(|| ArtifactId::from("missing-external")),
            ));
        }
        // Each operation, not only externally effectful operations, has one
        // trusted owner command. The kernel knows no provider-specific type.
        if run.owner_commands.len() != run.plan.nodes.len() {
            return Err(GenericDagError::InvalidCommand(
                run.plan
                    .nodes
                    .first()
                    .expect("nonempty DAG")
                    .instance
                    .clone(),
            ));
        }
        let mut seen = BTreeSet::new();
        for node in &run.plan.nodes {
            let command = run
                .owner_commands
                .get(&node.instance)
                .ok_or_else(|| GenericDagError::InvalidCommand(node.instance.clone()))?;
            let contract = routes
                .catalog()
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            if command.operation != node.operation
                || command.owner != contract.owner
                || command.contract_hash != contract.fingerprint()?
                || command.command_bytes.is_empty()
            {
                return Err(GenericDagError::CommandContractMismatch(
                    node.instance.clone(),
                ));
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
            produced_evidence: BTreeMap::new(),
            owner_results: BTreeMap::new(),
            cursor: 0,
            v2,
            validation_required: false,
            validation_completed: false,
        })
    }

    pub fn next_instance(&self) -> Option<&OperationInstanceId> {
        self.validated.execution_order().get(self.cursor)
    }

    /// V2-only owner-local pre-effect validation requests for *every* node.
    /// Exact External bytes are available; Produced sources are represented
    /// solely by their already checked versioned port schemas. Owners MUST
    /// validate concrete Produced bytes again before any downstream effect.
    pub fn v2_owner_validation_requests(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<Vec<(PinnedOwnerRouteV1, OwnerValidationRequestV1)>, GenericDagError> {
        if !self.v2 || self.cursor != 0 {
            return Err(GenericDagError::Incomplete);
        }
        let mut requests = Vec::with_capacity(self.run.plan.nodes.len());
        for instance in self.validated.execution_order() {
            let node = self
                .run
                .plan
                .nodes
                .iter()
                .find(|node| &node.instance == instance)
                .expect("validated plan node");
            let contract = self
                .routes
                .catalog()
                .contract(&node.operation)
                .expect("validated owner contract");
            let command = self
                .run
                .owner_commands
                .get(instance)
                .expect("validated owner command");
            let route = self
                .routes
                .trusted_snapshot_for([&node.operation])?
                .into_iter()
                .next()
                .expect("validated trusted owner route");
            let mut external_inputs = BTreeMap::new();
            let mut produced_input_ports = BTreeSet::new();
            for (port, source) in &node.inputs {
                match source {
                    ArtifactBinding::External { artifact } => {
                        external_inputs.insert(
                            port.clone(),
                            self.run
                                .admitted
                                .get(artifact)
                                .expect("validated External")
                                .clone(),
                        );
                    }
                    ArtifactBinding::Produced { .. } => {
                        produced_input_ports.insert(port.clone());
                    }
                }
            }
            let request = OwnerValidationRequestV1 {
                schema_revision: OWNER_VALIDATION_SCHEMA_REVISION_V1,
                parent: parent.clone(),
                plan_hash: self.run.plan.hash.clone(),
                scope: self.run.plan.scope.clone(),
                instance: instance.clone(),
                operation: node.operation.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract.fingerprint()?,
                command_bytes: command.command_bytes.clone(),
                input_schemas: contract.inputs.clone(),
                external_inputs,
                produced_input_ports,
            };
            request
                .validate_shape()
                .map_err(|error| JsonBudgetError::EnvelopeSerialization(error.to_string()))?;
            requests.push((route, request));
        }
        Ok(requests)
    }

    /// Called only once after the parent observed all typed owner validation
    /// Child completions. No proof can be supplied by the untrusted run plan.
    pub fn complete_v2_owner_validation(
        &mut self,
        parent: TemporalExecutionIdentityV4,
        results: &[OwnerValidationResultV1],
    ) -> Result<(), GenericDagError> {
        if !self.v2
            || !self.validation_required
            || self.validation_completed
            || self.cursor != 0
            || results.len() != self.run.plan.nodes.len()
        {
            return Err(GenericDagError::OwnerPreflightPending);
        }
        let requests = self.v2_owner_validation_requests(parent)?;
        for ((_, request), result) in requests.iter().zip(results) {
            result
                .verify_success(request)
                .map_err(|_| GenericDagError::OwnerPreflightPending)?;
        }
        self.validation_completed = true;
        Ok(())
    }

    fn checked_input_for(
        &self,
        instance: &OperationInstanceId,
    ) -> Result<(CheckedDispatch, BTreeMap<PortId, BoundArtifactV1>), GenericDagError> {
        if self.validation_required && !self.validation_completed {
            return Err(GenericDagError::OwnerPreflightPending);
        }
        let node = self
            .run
            .plan
            .nodes
            .iter()
            .find(|node| &node.instance == instance)
            .expect("validated node");
        let contract = self
            .routes
            .catalog()
            .contract(&node.operation)
            .expect("validated owner contract");
        let command = self
            .run
            .owner_commands
            .get(instance)
            .expect("preflight command");
        let mut bound = BTreeMap::new();
        for (port, source) in &node.inputs {
            let id = match source {
                ArtifactBinding::External { artifact }
                | ArtifactBinding::Produced { artifact, .. } => artifact,
            };
            let actual = self
                .artifacts
                .get(id)
                .ok_or_else(|| GenericDagError::MissingProducedArtifact(id.clone()))?;
            if contract.inputs.get(port) != Some(&actual.schema)
                || actual.scope != self.run.plan.scope
                || actual.provenance.trim().is_empty()
            {
                return Err(GenericDagError::InvalidArtifact(id.clone()));
            }
            let produced_evidence = match source {
                ArtifactBinding::External { .. } => None,
                ArtifactBinding::Produced {
                    producer,
                    port: producer_port,
                    artifact,
                } => {
                    let proof = self
                        .produced_evidence
                        .get(id)
                        .ok_or_else(|| GenericDagError::MissingProducedArtifact(id.clone()))?;
                    let earlier = self
                        .owner_results
                        .get(producer)
                        .ok_or_else(|| GenericDagError::MissingProducedArtifact(id.clone()))?;
                    let actual_result_hash =
                        canonical_owner_result_hash(earlier).map_err(|error| {
                            JsonBudgetError::EnvelopeSerialization(error.to_string())
                        })?;
                    if proof.producer != *producer
                        || proof.port != *producer_port
                        || proof.artifact_id != *artifact
                        || proof.dispatch_execution_key != earlier.dispatch_execution_key
                        || proof.child_workflow_id
                            != if self.v2 {
                                owner_child_workflow_id_v2(&proof.dispatch_execution_key)
                            } else {
                                owner_child_workflow_id(&proof.dispatch_execution_key)
                            }
                        || proof.result_hash != actual_result_hash
                        || proof.receipt_hash != owner_content_hash(&earlier.receipt_bytes)
                        || proof.payload_hash != owner_content_hash(&actual.payload)
                        || earlier.outputs.get(producer_port) != Some(actual)
                    {
                        return Err(GenericDagError::InvalidArtifact(id.clone()));
                    }
                    Some(proof.clone())
                }
            };
            bound.insert(
                port.clone(),
                BoundArtifactV1 {
                    artifact_id: id.clone(),
                    source: source.clone(),
                    artifact: actual.clone(),
                    produced_evidence,
                },
            );
        }
        let actual_bytes = canonical_owner_dispatch_input_bytes(&bound, &command.command_bytes)
            .map_err(|error| JsonBudgetError::EnvelopeSerialization(error.to_string()))?;
        let dispatch =
            self.validated
                .checked_dispatch(instance, &actual_bytes, DispatchAttempt::First)?;
        self.routes.route_for_dispatch(&dispatch)?;
        Ok((dispatch, bound))
    }

    fn checked_input(
        &self,
    ) -> Result<(CheckedDispatch, BTreeMap<PortId, BoundArtifactV1>), GenericDagError> {
        let instance = self.next_instance().ok_or(GenericDagError::Incomplete)?;
        self.checked_input_for(instance)
    }

    fn invocation_for(
        &self,
        instance: &OperationInstanceId,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<(PinnedOwnerRouteV1, OwnerInvocationV1), GenericDagError> {
        let command = self
            .run
            .owner_commands
            .get(instance)
            .expect("preflight owner command");
        let (checked, bound_inputs) = self.checked_input_for(instance)?;
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

    pub fn next_owner_invocation(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<(PinnedOwnerRouteV1, OwnerInvocationV1), GenericDagError> {
        let instance = self.next_instance().ok_or(GenericDagError::Incomplete)?;
        self.invocation_for(instance, parent)
    }

    /// Build the maximal contiguous ready wave in canonical topological order.
    /// Every edge is determined by the sealed DAG (explicit `after` or typed
    /// Produced binding); overlapping declared effect classes are serialized.
    /// No child starts before all commands in this wave pass preflight.
    ///
    /// The prefix restriction keeps final receipt ordering and replay stable
    /// even when a newly-ready dependent sorts before an unrelated node.
    pub fn next_independent_owner_invocations(
        &self,
        parent: TemporalExecutionIdentityV4,
    ) -> Result<Vec<(PinnedOwnerRouteV1, OwnerInvocationV1)>, GenericDagError> {
        const MAX_IN_FLIGHT_OWNER_CHILDREN: usize = 8;

        let mut wave = Vec::new();
        let mut wave_ids = BTreeSet::new();
        let mut wave_effects = BTreeSet::new();
        let mut wave_invalidates = BTreeSet::new();
        for instance in self.validated.execution_order().iter().skip(self.cursor) {
            let node = self
                .run
                .plan
                .nodes
                .iter()
                .find(|node| &node.instance == instance)
                .expect("validated DAG node");
            let contract = self
                .routes
                .catalog()
                .contract(&node.operation)
                .expect("validated owner contract");
            let depends_on_in_flight = node.after.iter().any(|id| wave_ids.contains(id))
                || node.inputs.values().any(|binding| match binding {
                    ArtifactBinding::Produced { producer, .. } => wave_ids.contains(producer),
                    ArtifactBinding::External { .. } => false,
                });
            let conflicts = contract
                .effects
                .iter()
                .any(|effect| wave_effects.contains(effect))
                || contract
                    .invalidates
                    .iter()
                    .any(|artifact_type| wave_invalidates.contains(artifact_type));
            if !wave.is_empty() && (depends_on_in_flight || conflicts) {
                break;
            }
            let invocation = self.invocation_for(instance, parent.clone())?;
            wave.push(invocation);
            wave_ids.insert(instance.clone());
            wave_effects.extend(contract.effects.iter().cloned());
            wave_invalidates.extend(contract.invalidates.iter().cloned());
            if wave.len() == MAX_IN_FLIGHT_OWNER_CHILDREN {
                break;
            }
        }
        Ok(wave)
    }

    /// The result is genuinely returned by the selected Temporal child.
    /// Validate all output schemas and bind exactly the declared artifacts.
    /// Dynamic producer payloads are never guessed in the static preflight.
    /// Budget-check the projected *entire* receipt before publishing any
    /// artifact, because a finish-only budget check would be too late to stop
    /// subsequent external effects. Already committed owners cannot roll back.
    pub fn advance_owner_result(&mut self, result: OwnerResultV1) -> Result<(), GenericDagError> {
        let instance = self
            .next_instance()
            .ok_or(GenericDagError::Incomplete)?
            .clone();
        let (checked, _) = self.checked_input()?;
        result.verify_integrity(&checked, &self.routes)?;
        enforce_json_budget(&result, PURE_DAG_MAX_RESULT_BYTES, "owner_result_bytes")?;
        let node = self
            .run
            .plan
            .nodes
            .iter()
            .find(|node| node.instance == instance)
            .expect("validated DAG node");
        let mut next_artifacts = self.artifacts.clone();
        let mut next_evidence = self.produced_evidence.clone();
        let result_hash = canonical_owner_result_hash(&result)
            .map_err(|error| JsonBudgetError::EnvelopeSerialization(error.to_string()))?;
        let receipt_hash = owner_content_hash(&result.receipt_bytes);
        for (port, value) in &result.outputs {
            let id = node.outputs.get(port).expect("validated output port");
            if next_artifacts.contains_key(id) {
                return Err(GenericDagError::InvalidArtifact(id.clone()));
            }
            next_evidence.insert(
                id.clone(),
                ProducedArtifactEvidenceV1 {
                    producer: instance.clone(),
                    port: port.clone(),
                    artifact_id: id.clone(),
                    dispatch_execution_key: checked.execution_key.clone(),
                    child_workflow_id: if self.v2 {
                        owner_child_workflow_id_v2(&checked.execution_key)
                    } else {
                        owner_child_workflow_id(&checked.execution_key)
                    },
                    result_hash: result_hash.clone(),
                    receipt_hash: receipt_hash.clone(),
                    payload_hash: owner_content_hash(&value.payload),
                },
            );
            next_artifacts.insert(id.clone(), value.clone());
        }
        let mut next_results = self.owner_results.clone();
        next_results.insert(instance, result);
        let prospective_receipt = OperationDagReceiptV1 {
            plan_hash: self.run.plan.hash.clone(),
            order: self.validated.execution_order().to_vec(),
            artifacts: next_artifacts.clone(),
            produced_evidence: next_evidence.clone(),
            owner_results: next_results.clone(),
        };
        enforce_json_budget(
            &prospective_receipt,
            PURE_DAG_MAX_RESULT_BYTES,
            "generic_dag_result",
        )?;
        self.artifacts = next_artifacts;
        self.produced_evidence = next_evidence;
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
            produced_evidence: self.produced_evidence,
            owner_results: self.owner_results,
        };
        enforce_json_budget(&receipt, PURE_DAG_MAX_RESULT_BYTES, "generic_dag_result")?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArtifactSchema, ArtifactTypeId, ContractHash, EffectId, ExecutionPlan, IdempotencyPolicy,
        OperationContract, OperationId, OperationNode, OperationOwnerId, OperationRef,
        OwnerCommandWireV1, PlanHash, RunScope, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };

    fn site_lab_expected_start(run: &OperationDagRunV6) -> crate::SiteLabExpectedStartV1 {
        crate::SiteLabExpectedStartV1 {
            schema_revision: crate::SITE_LAB_START_PREFLIGHT_SCHEMA_REVISION_V1,
            plan_hash: run.plan.hash.clone(),
            command_hashes: run
                .owner_commands
                .iter()
                .map(|(instance, command)| {
                    (
                        instance.clone(),
                        crate::owner_content_hash(&command.command_bytes),
                    )
                })
                .collect(),
            source_payload_hashes: run
                .admitted
                .iter()
                .map(|(artifact_id, artifact)| {
                    (
                        artifact_id.clone(),
                        crate::owner_content_hash(&artifact.payload),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn site_lab_start_preflight_is_only_a_hash_only_unadmitted_result() {
        let (run, routes) = setup();
        let expected = site_lab_expected_start(&run);
        let report = crate::preflight_unadmitted_site_lab_v2_start(&run, &routes, &expected)
            .expect("existing V2 machine accepts the unchanged snapshot");
        assert_eq!(report.status, "preflight_only_not_admitted");
        assert_eq!(report.execution, "not_started");
        assert_eq!(report.owner_admission, "not_issued");
        assert_eq!(report.source_authentication, "not_checked");
        assert_eq!(report.steps_requiring_owner_validate_only.len(), 2);
        assert_eq!(report.command_hashes, expected.command_hashes);
        assert!(report.source_payload_hashes.is_empty());
    }

    #[test]
    fn site_lab_preflight_rejects_changed_owner_bytes_and_unpinned_command_set() {
        let (run, routes) = setup();
        let expected = site_lab_expected_start(&run);
        let mut changed = run.clone();
        changed
            .owner_commands
            .get_mut(&OperationInstanceId::from("consumer"))
            .unwrap()
            .command_bytes
            .push(b'!');
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&changed, &routes, &expected),
            Err(crate::SiteLabStartPreflightError::CommandDigestMismatch(instance))
                if instance.as_str() == "consumer"
        ));

        let mut incomplete = expected.clone();
        incomplete
            .command_hashes
            .remove(&OperationInstanceId::from("producer"));
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&run, &routes, &incomplete),
            Err(crate::SiteLabStartPreflightError::InvalidExpectation)
        ));
        let mut wrong_hash = expected;
        wrong_hash.command_hashes.insert(
            OperationInstanceId::from("producer"),
            "not-canonical".to_owned(),
        );
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&run, &routes, &wrong_hash),
            Err(crate::SiteLabStartPreflightError::InvalidExpectation)
        ));

        let mut wrong_plan = site_lab_expected_start(&run);
        wrong_plan.plan_hash = PlanHash::from("different-plan");
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&run, &routes, &wrong_plan),
            Err(crate::SiteLabStartPreflightError::InvalidExpectation)
        ));
    }

    #[test]
    fn site_lab_preflight_reuses_the_native_validator_for_plan_routes_and_sources() {
        let (run, routes) = setup();
        let expected = site_lab_expected_start(&run);
        let mut tampered_plan = run.clone();
        tampered_plan.plan.nodes[0].outputs.clear();
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&tampered_plan, &routes, &expected),
            Err(crate::SiteLabStartPreflightError::InvalidRun(_))
        ));

        let mut tampered_route = run.clone();
        tampered_route.pinned_routes[0].task_queue = "untrusted-queue".to_owned();
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(&tampered_route, &routes, &expected),
            Err(crate::SiteLabStartPreflightError::InvalidRun(_))
        ));

        let mut source_run = run;
        let source_id = ArtifactId::from("external-document");
        source_run.plan.external_artifacts.insert(
            source_id.clone(),
            crate::ExternalArtifact {
                schema: schema(),
                scope: scope(),
                provenance: "trusted-source".to_owned(),
            },
        );
        source_run.plan.nodes[1].inputs.insert(
            PortId::from("source"),
            ArtifactBinding::External {
                artifact: source_id.clone(),
            },
        );
        source_run.plan = source_run.plan.seal(routes.catalog()).unwrap();
        source_run.admitted.insert(
            source_id,
            AdmittedArtifactV1 {
                schema: schema(),
                scope: scope(),
                provenance: "trusted-source".to_owned(),
                payload: b"same-looking-bytes-not-admitted".to_vec(),
            },
        );
        let source_expectation = site_lab_expected_start(&source_run);
        let original = crate::preflight_unadmitted_site_lab_v2_start(
            &source_run,
            &routes,
            &source_expectation,
        )
        .unwrap();
        assert_eq!(original.source_payload_hashes.len(), 1);

        let mut changed_payload = source_run.clone();
        changed_payload
            .admitted
            .get_mut(&ArtifactId::from("external-document"))
            .unwrap()
            .payload
            .push(b'!');
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(
                &changed_payload,
                &routes,
                &source_expectation
            ),
            Err(crate::SiteLabStartPreflightError::SourceDigestMismatch(artifact))
                if artifact.as_str() == "external-document"
        ));

        source_run
            .admitted
            .get_mut(&ArtifactId::from("external-document"))
            .unwrap()
            .provenance = "wrong-source-revision".to_owned();
        assert!(matches!(
            crate::preflight_unadmitted_site_lab_v2_start(
                &source_run,
                &routes,
                &source_expectation
            ),
            Err(crate::SiteLabStartPreflightError::InvalidRun(_))
        ));
    }

    const PARENT_WORKFLOW_ID: &str = "alegria-operation-dag-v1-run-a";

    fn scope() -> RunScope {
        RunScope {
            tenant_id: "tenant-a".into(),
            run_id: "run-a".into(),
        }
    }

    fn schema() -> ArtifactSchema {
        ArtifactSchema {
            id: ArtifactTypeId::from("validated.document"),
            revision: 1,
        }
    }

    fn contract(name: &str, input: bool) -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(name.into()),
                revision: 1,
            },
            owner: OperationOwnerId(format!("{name}.owner")),
            inputs: if input {
                BTreeMap::from([(PortId::from("source"), schema())])
            } else {
                BTreeMap::new()
            },
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
            commands.insert(
                OperationInstanceId::from(name),
                OwnerCommandWireV1 {
                    operation: contract.reference.clone(),
                    owner: contract.owner.clone(),
                    contract_hash: hash.clone(),
                    command_bytes: format!("{name}-typed-command").into_bytes(),
                },
            );
            routes
                .register(
                    contract.clone(),
                    PinnedOwnerRouteV1 {
                        operation: contract.reference,
                        owner: contract.owner,
                        contract_hash: hash,
                        workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                        task_queue: format!("{name}-independent-owner-queue"),
                        build_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    },
                )
                .unwrap();
        }
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope(),
            allowed_effects: BTreeSet::<EffectId>::new(),
            external_artifacts: BTreeMap::new(),
            pinned_contracts: vec![],
            nodes: vec![
                OperationNode {
                    instance: "producer".into(),
                    operation: contract("producer", false).reference,
                    inputs: BTreeMap::new(),
                    outputs: BTreeMap::from([("document".into(), "artifact-a".into())]),
                    after: BTreeSet::new(),
                    permitted_effects: BTreeSet::new(),
                },
                OperationNode {
                    instance: "consumer".into(),
                    operation: contract("consumer", true).reference,
                    inputs: BTreeMap::from([(
                        "source".into(),
                        ArtifactBinding::Produced {
                            producer: "producer".into(),
                            port: "document".into(),
                            artifact: "artifact-a".into(),
                        },
                    )]),
                    outputs: BTreeMap::from([("document".into(), "artifact-b".into())]),
                    after: BTreeSet::new(),
                    permitted_effects: BTreeSet::new(),
                },
            ],
            hash: PlanHash::from("unsealed"),
        }
        .seal(routes.catalog())
        .unwrap();
        let pinned_routes = routes
            .trusted_snapshot_for(commands.values().map(|command| &command.operation))
            .unwrap();
        (
            OperationDagRunV6 {
                schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
                plan,
                admitted: BTreeMap::new(),
                owner_commands: commands,
                pinned_routes,
            },
            routes,
        )
    }

    fn parent() -> TemporalExecutionIdentityV4 {
        TemporalExecutionIdentityV4 {
            workflow_id: PARENT_WORKFLOW_ID.into(),
            run_id: "temporal-server-run".into(),
        }
    }

    fn setup_with_independent_node() -> (OperationDagRunV6, TrustedRouteCatalog) {
        let (mut run, mut routes) = setup();
        let extra = contract("a-independent", false);
        let hash = extra.fingerprint().unwrap();
        routes
            .register(
                extra.clone(),
                PinnedOwnerRouteV1 {
                    operation: extra.reference.clone(),
                    owner: extra.owner.clone(),
                    contract_hash: hash.clone(),
                    workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                    task_queue: "third-independent-owner-queue".into(),
                    build_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                },
            )
            .unwrap();
        run.owner_commands.insert(
            "a-independent".into(),
            OwnerCommandWireV1 {
                operation: extra.reference.clone(),
                owner: extra.owner,
                contract_hash: hash,
                command_bytes: b"third-owner-command".to_vec(),
            },
        );
        run.plan.nodes.push(OperationNode {
            instance: "a-independent".into(),
            operation: extra.reference,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::from([("document".into(), "artifact-independent".into())]),
            after: BTreeSet::new(),
            permitted_effects: BTreeSet::new(),
        });
        run.plan = run.plan.seal(routes.catalog()).unwrap();
        run.pinned_routes = routes
            .trusted_snapshot_for(
                run.owner_commands
                    .values()
                    .map(|command| &command.operation),
            )
            .unwrap();
        (run, routes)
    }

    #[test]
    fn independent_owner_children_form_a_ready_wave_but_produced_consumer_waits() {
        let (run, routes) = setup_with_independent_node();
        let mut machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).unwrap();
        let first_wave = machine
            .next_independent_owner_invocations(parent())
            .unwrap();
        assert_eq!(first_wave.len(), 2);
        assert_eq!(
            first_wave[0].1.checked_dispatch.instance,
            OperationInstanceId::from("a-independent")
        );
        assert_eq!(
            first_wave[1].1.checked_dispatch.instance,
            OperationInstanceId::from("producer")
        );
        assert!(first_wave
            .iter()
            .all(|(_, invocation)| invocation.bound_inputs.is_empty()));

        for (_, invocation) in first_wave {
            machine
                .advance_owner_result(result(&invocation, b"genuine-first-wave-output"))
                .unwrap();
        }
        let second_wave = machine
            .next_independent_owner_invocations(parent())
            .unwrap();
        assert_eq!(second_wave.len(), 1);
        let consumer = &second_wave[0].1;
        assert_eq!(
            consumer.checked_dispatch.instance,
            OperationInstanceId::from("consumer")
        );
        assert_eq!(
            consumer.bound_inputs[&PortId::from("source")]
                .artifact
                .payload,
            b"genuine-first-wave-output"
        );
        machine
            .advance_owner_result(result(consumer, b"consumer-output"))
            .unwrap();
        assert_eq!(machine.finish().unwrap().order.len(), 3);
    }

    #[test]
    fn parallel_v2_has_distinct_parent_identity_and_requires_pure_owner_contracts() {
        let (run, routes) = setup_with_independent_node();
        let old_id = operation_dag_parent_workflow_id("run-a");
        assert!(matches!(
            OperationDagMachine::new_v2(&old_id, run.clone(), routes.clone()),
            Err(GenericDagError::TemporalRunIdMismatch)
        ));
        let v2_id = operation_dag_parent_workflow_id_v2("run-a");
        assert!(matches!(
            OperationDagMachine::new(&v2_id, run.clone(), routes.clone()),
            Err(GenericDagError::TemporalRunIdMismatch)
        ));
        let machine = OperationDagMachine::new_v2(&v2_id, run, routes).unwrap();
        let mut parent = parent();
        parent.workflow_id = v2_id;
        let wave = machine.next_independent_owner_invocations(parent).unwrap();
        assert_eq!(wave.len(), 2);
    }

    #[test]
    fn v2_validation_requests_are_exactly_bound_to_immutable_parent_plan() {
        let (run, routes) = setup();
        let id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let machine = OperationDagMachine::new_v2(&id, run.clone(), routes).unwrap();
        let mut actual_parent = parent();
        actual_parent.workflow_id = id;
        let requests = machine.v2_owner_validation_requests(actual_parent).unwrap();
        assert_eq!(requests.len(), 2);
        for (_, request) in &requests {
            request.verify_against_run(&run).unwrap();
        }

        let consumer = requests
            .iter()
            .find(|(_, request)| request.instance.as_str() == "consumer")
            .unwrap()
            .1
            .clone();
        let mut wrong = consumer.clone();
        wrong.command_bytes.push(0);
        assert!(wrong.verify_against_run(&run).is_err());
        let mut wrong = consumer.clone();
        wrong.produced_input_ports.clear();
        assert!(wrong.verify_against_run(&run).is_err());
        let mut wrong = consumer.clone();
        wrong.contract_hash = ContractHash::from("another-contract");
        assert!(wrong.verify_against_run(&run).is_err());
        let mut wrong = consumer.clone();
        wrong.owner = OperationOwnerId::from("foreign-owner");
        assert!(wrong.verify_against_run(&run).is_err());
        let mut wrong = consumer.clone();
        wrong.plan_hash = PlanHash::from("foreign-plan");
        assert!(wrong.verify_against_run(&run).is_err());
        let mut wrong_run = run.clone();
        wrong_run.owner_commands.remove(&consumer.instance);
        assert!(consumer.verify_against_run(&wrong_run).is_err());
        let mut wrong_run = run.clone();
        wrong_run
            .pinned_routes
            .retain(|route| route.operation != consumer.operation);
        assert!(consumer.verify_against_run(&wrong_run).is_err());
        let mut wrong_run = run.clone();
        wrong_run
            .plan
            .pinned_contracts
            .retain(|pinned| pinned.reference != consumer.operation);
        wrong_run.plan.hash = wrong_run.plan.canonical_hash().unwrap();
        assert!(consumer.verify_against_run(&wrong_run).is_err());
    }

    #[test]
    fn v2_owner_preflight_rejects_spoofed_produced_edges_in_parent_history() {
        let (run, routes) = setup();
        let id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let machine = OperationDagMachine::new_v2(&id, run.clone(), routes).unwrap();
        let mut actual_parent = parent();
        actual_parent.workflow_id = id;
        let consumer = machine
            .v2_owner_validation_requests(actual_parent)
            .unwrap()
            .into_iter()
            .find(|(_, request)| request.instance.as_str() == "consumer")
            .unwrap()
            .1;
        consumer.verify_against_run(&run).unwrap();

        // Recompute the plan hash after every forgery so this specifically
        // tests independent owner-side Produced-edge admission, not the
        // existing parent-plan hash check.
        let reject = |mut forged: OperationDagRunV6| {
            forged.plan.hash = forged.plan.canonical_hash().unwrap();
            let mut request = consumer.clone();
            request.plan_hash = forged.plan.hash.clone();
            assert!(
                request.verify_against_run(&forged).is_err(),
                "owner must not trust an unbound Produced edge"
            );
        };

        let mut missing_producer = run.clone();
        missing_producer.plan.nodes.remove(0);
        reject(missing_producer);

        let mut missing_output = run.clone();
        missing_output.plan.nodes[0].outputs.clear();
        reject(missing_output);

        let mut wrong_producer_port = run.clone();
        wrong_producer_port.plan.nodes[1].inputs.insert(
            "source".into(),
            ArtifactBinding::Produced {
                producer: "producer".into(),
                port: "missing-port".into(),
                artifact: "artifact-a".into(),
            },
        );
        reject(wrong_producer_port);

        let mut duplicate_producer = run.clone();
        let duplicated_node = duplicate_producer.plan.nodes[0].clone();
        duplicate_producer.plan.nodes.push(duplicated_node);
        reject(duplicate_producer);

        let mut external_alias = run;
        external_alias.plan.external_artifacts.insert(
            "artifact-a".into(),
            crate::ExternalArtifact {
                schema: schema(),
                scope: scope(),
                provenance: "forged-external-alias".into(),
            },
        );
        reject(external_alias);
    }

    #[test]
    fn v2_validation_rejects_cross_tenant_or_changed_external_artifact() {
        let (mut run, routes) = setup();
        let ext: ArtifactId = "external-document".into();
        run.plan.external_artifacts.insert(
            ext.clone(),
            crate::ExternalArtifact {
                schema: schema(),
                scope: run.plan.scope.clone(),
                provenance: "trusted-source".into(),
            },
        );
        run.plan.nodes[1].inputs.insert(
            "source".into(),
            ArtifactBinding::External {
                artifact: ext.clone(),
            },
        );
        run.plan = run.plan.seal(routes.catalog()).unwrap();
        run.admitted.insert(
            ext,
            AdmittedArtifactV1 {
                schema: schema(),
                scope: run.plan.scope.clone(),
                provenance: "trusted-source".into(),
                payload: b"admitted-external-payload".to_vec(),
            },
        );
        let id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let machine = OperationDagMachine::new_v2(&id, run.clone(), routes).unwrap();
        let mut actual_parent = parent();
        actual_parent.workflow_id = id;
        let requests = machine.v2_owner_validation_requests(actual_parent).unwrap();
        let original = requests
            .iter()
            .find(|(_, request)| request.instance.as_str() == "consumer")
            .unwrap()
            .1
            .clone();
        original.verify_against_run(&run).unwrap();
        let mut forged = original.clone();
        forged
            .external_inputs
            .get_mut(&PortId::from("source"))
            .unwrap()
            .payload
            .push(0);
        assert!(forged.verify_against_run(&run).is_err());
        let mut forged = original.clone();
        forged
            .external_inputs
            .get_mut(&PortId::from("source"))
            .unwrap()
            .scope
            .tenant_id = "foreign-tenant".into();
        assert!(forged.verify_against_run(&run).is_err());
        let mut forged = original;
        forged
            .input_schemas
            .get_mut(&PortId::from("source"))
            .unwrap()
            .revision += 1;
        assert!(forged.verify_against_run(&run).is_err());
    }

    #[test]
    fn parallel_v2_rejects_effectful_owner_before_first_child_command() {
        let mut routes = TrustedRouteCatalog::default();
        let mut write = contract("admitted-external-writer", false);
        write.effects.insert(EffectId::from("external.write"));
        write.idempotency = IdempotencyPolicy::Idempotent;
        let fingerprint = write.fingerprint().unwrap();
        routes
            .register(
                write.clone(),
                PinnedOwnerRouteV1 {
                    operation: write.reference.clone(),
                    owner: write.owner.clone(),
                    contract_hash: fingerprint.clone(),
                    workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                    task_queue: "effectful-independent-owner-queue".into(),
                    build_id: "cccccccccccccccccccccccccccccccccccccccc".into(),
                },
            )
            .unwrap();
        let instance = OperationInstanceId::from("external-writer");
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope(),
            allowed_effects: BTreeSet::from([EffectId::from("external.write")]),
            external_artifacts: BTreeMap::new(),
            pinned_contracts: vec![],
            nodes: vec![OperationNode {
                instance: instance.clone(),
                operation: write.reference.clone(),
                inputs: BTreeMap::new(),
                outputs: BTreeMap::from([("document".into(), "written-artifact".into())]),
                after: BTreeSet::new(),
                permitted_effects: BTreeSet::from([EffectId::from("external.write")]),
            }],
            hash: PlanHash::from("unsealed"),
        }
        .seal(routes.catalog())
        .unwrap();
        let run = OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::new(),
            owner_commands: BTreeMap::from([(
                instance.clone(),
                OwnerCommandWireV1 {
                    operation: write.reference,
                    owner: write.owner,
                    contract_hash: fingerprint,
                    command_bytes: b"effectful-command-must-not-start".to_vec(),
                },
            )]),
            pinned_routes: routes
                .trusted_snapshot_for([&OperationRef {
                    id: OperationId::from("admitted-external-writer"),
                    revision: 1,
                }])
                .unwrap(),
        };
        let id = operation_dag_parent_workflow_id_v2("run-a");
        assert!(matches!(
            OperationDagMachine::new_v2(&id, run.clone(), routes.clone()),
            Err(GenericDagError::UnadmittedV2Effect(rejected)) if rejected == instance
        ));
        let mut gated = OperationDagMachine::new_v2_with_owner_validation(&id, run, routes)
            .expect("V2 admits an effectful contract only behind owner validation gate");
        let mut actual_parent = parent();
        actual_parent.workflow_id = id;
        assert!(matches!(
            gated.next_independent_owner_invocations(actual_parent.clone()),
            Err(GenericDagError::OwnerPreflightPending)
        ));
        let requests = gated
            .v2_owner_validation_requests(actual_parent.clone())
            .unwrap();
        let accepted = requests
            .iter()
            .map(|(_, request)| OwnerValidationResultV1::accepted(request).unwrap())
            .collect::<Vec<_>>();
        let mut forged = accepted.clone();
        forged[0].request_hash = "forged-validation-receipt".into();
        assert!(matches!(
            gated.complete_v2_owner_validation(actual_parent.clone(), &forged),
            Err(GenericDagError::OwnerPreflightPending)
        ));
        gated
            .complete_v2_owner_validation(actual_parent.clone(), &accepted)
            .unwrap();
        let ready = gated
            .next_independent_owner_invocations(actual_parent)
            .unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].1.checked_dispatch.instance, instance);
    }

    #[test]
    fn explicit_after_dependency_prevents_parallel_owner_dispatch() {
        let (mut run, routes) = setup_with_independent_node();
        run.plan
            .nodes
            .iter_mut()
            .find(|node| node.instance == OperationInstanceId::from("a-independent"))
            .unwrap()
            .after
            .insert("producer".into());
        run.plan = run.plan.seal(routes.catalog()).unwrap();
        let machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).unwrap();
        let wave = machine
            .next_independent_owner_invocations(parent())
            .unwrap();
        assert_eq!(wave.len(), 1);
        assert_eq!(
            wave[0].1.checked_dispatch.instance,
            OperationInstanceId::from("producer")
        );
    }

    #[test]
    fn generic_parent_identity_is_distinct_from_business_truth_workflow_and_server_run() {
        assert_eq!(
            operation_dag_parent_workflow_id("run-a"),
            PARENT_WORKFLOW_ID
        );
        let (run, routes) = setup();
        assert!(matches!(
            OperationDagMachine::new("run-a", run, routes),
            Err(GenericDagError::TemporalRunIdMismatch)
        ));

        let (run, routes) = setup();
        let machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes.clone()).unwrap();
        let (_, mut invocation) = machine.next_owner_invocation(parent()).unwrap();
        assert_ne!(
            invocation.parent.workflow_id,
            invocation.checked_dispatch.scope.run_id
        );
        assert_ne!(invocation.parent.run_id, invocation.parent.workflow_id);
        invocation.parent.workflow_id = "run-a".into();
        assert!(matches!(
            invocation.verify_integrity(&routes),
            Err(RouteCatalogError::InvalidInvocation)
        ));
        invocation.parent.workflow_id = "untrusted-generic-parent".into();
        assert!(matches!(
            invocation.verify_integrity(&routes),
            Err(RouteCatalogError::InvalidInvocation)
        ));
    }

    fn result(invocation: &OwnerInvocationV1, payload: &[u8]) -> OwnerResultV1 {
        OwnerResultV1 {
            dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
            outputs: BTreeMap::from([(
                "document".into(),
                AdmittedArtifactV1 {
                    schema: schema(),
                    scope: scope(),
                    provenance: "attested-child-receipt".into(),
                    payload: payload.to_vec(),
                },
            )]),
            receipt_bytes: b"independent-operator-receipt".to_vec(),
        }
    }

    #[test]
    fn materializes_actual_producer_bytes_as_strictly_bound_consumer_input() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        assert!(producer.bound_inputs.is_empty());
        let producer_result = result(&producer, b"real-owner-payload");
        let expected_result_hash = canonical_owner_result_hash(&producer_result).unwrap();
        machine.advance_owner_result(producer_result).unwrap();

        let (_, consumer) = machine.next_owner_invocation(parent()).unwrap();
        let binding = consumer.bound_inputs.get(&PortId::from("source")).unwrap();
        assert_eq!(binding.artifact_id, ArtifactId::from("artifact-a"));
        assert_eq!(binding.artifact.payload, b"real-owner-payload");
        let evidence = binding
            .produced_evidence
            .as_ref()
            .expect("required Produced proof");
        assert_eq!(evidence.producer, OperationInstanceId::from("producer"));
        assert_eq!(evidence.port, PortId::from("document"));
        assert_eq!(evidence.artifact_id, ArtifactId::from("artifact-a"));
        assert_eq!(
            evidence.dispatch_execution_key,
            producer.checked_dispatch.execution_key
        );
        assert_eq!(
            evidence.child_workflow_id,
            owner_child_workflow_id(&producer.checked_dispatch.execution_key)
        );
        assert_eq!(evidence.result_hash, expected_result_hash);
        assert_eq!(
            evidence.receipt_hash,
            owner_content_hash(b"independent-operator-receipt")
        );
        assert_eq!(
            evidence.payload_hash,
            owner_content_hash(b"real-owner-payload")
        );
        assert_eq!(
            binding.source,
            ArtifactBinding::Produced {
                producer: "producer".into(),
                port: "document".into(),
                artifact: "artifact-a".into(),
            }
        );
        assert_ne!(
            producer.checked_dispatch.execution_key,
            consumer.checked_dispatch.execution_key
        );
        machine
            .advance_owner_result(result(&consumer, b"second-owner-output"))
            .unwrap();
        let completed = machine.finish().unwrap();
        assert_eq!(
            completed.artifacts[&ArtifactId::from("artifact-b")].payload,
            b"second-owner-output"
        );
        assert_eq!(
            &completed.produced_evidence[&ArtifactId::from("artifact-a")],
            evidence
        );
    }

    #[test]
    fn forged_produced_proof_receipt_hash_and_source_are_rejected_before_dispatch() {
        let (run, routes) = setup();
        let mut machine =
            OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes.clone()).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        machine
            .advance_owner_result(result(&producer, b"original"))
            .unwrap();
        let (_, consumer) = machine.next_owner_invocation(parent()).unwrap();
        consumer.verify_integrity(&routes).unwrap();

        // Even a structurally valid hash cannot replace the actual previous
        // child receipt: machine compares against the stored OwnerResult.
        machine
            .produced_evidence
            .get_mut(&ArtifactId::from("artifact-a"))
            .unwrap()
            .receipt_hash = "a".repeat(64);
        assert!(matches!(
            machine.next_owner_invocation(parent()),
            Err(GenericDagError::InvalidArtifact(_))
        ));

        let mut forged = consumer.clone();
        forged
            .bound_inputs
            .get_mut(&PortId::from("source"))
            .unwrap()
            .produced_evidence
            .as_mut()
            .unwrap()
            .child_workflow_id = "forged-child".into();
        rehash_invocation(&mut forged);
        assert!(matches!(
            forged.verify_integrity(&routes),
            Err(RouteCatalogError::InvalidInvocation)
        ));

        let mut forged = consumer.clone();
        forged
            .bound_inputs
            .get_mut(&PortId::from("source"))
            .unwrap()
            .source = ArtifactBinding::Produced {
            producer: "other-owner".into(),
            port: "document".into(),
            artifact: "artifact-a".into(),
        };
        rehash_invocation(&mut forged);
        assert!(matches!(
            forged.verify_integrity(&routes),
            Err(RouteCatalogError::InvalidInvocation)
        ));

        let mut forged = consumer;
        forged
            .bound_inputs
            .get_mut(&PortId::from("source"))
            .unwrap()
            .artifact
            .payload = b"forged-owner-payload".to_vec();
        rehash_invocation(&mut forged);
        assert!(matches!(
            forged.verify_integrity(&routes),
            Err(RouteCatalogError::InvalidInvocation)
        ));
    }

    fn rehash_invocation(invocation: &mut OwnerInvocationV1) {
        let bytes = canonical_owner_dispatch_input_bytes(
            &invocation.bound_inputs,
            &invocation.command_bytes,
        )
        .unwrap();
        let hash = owner_content_hash(&bytes);
        invocation.checked_dispatch.input_hash = hash.clone();
        invocation.checked_dispatch.execution_key =
            crate::checked_execution_key(&invocation.checked_dispatch, &hash).unwrap();
    }

    #[test]
    fn preflight_rejects_forged_route_and_owner_contract() {
        let (mut run, routes) = setup();
        run.pinned_routes[0].task_queue = "attacker-queue".into();
        assert!(matches!(
            OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes),
            Err(GenericDagError::Route(
                RouteCatalogError::UntrustedRouteSnapshot
            ))
        ));

        let (mut run, routes) = setup();
        run.owner_commands
            .get_mut(&OperationInstanceId::from("producer"))
            .unwrap()
            .contract_hash = ContractHash::from("forged");
        assert!(matches!(
            OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes),
            Err(GenericDagError::CommandContractMismatch(_))
        ));
    }

    #[test]
    fn rejects_wrong_owner_output_schema_scope_or_missing_port_before_publication() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        let mut forged = result(&producer, b"real-owner-payload");
        forged
            .outputs
            .get_mut(&PortId::from("document"))
            .unwrap()
            .schema
            .revision = 4;
        assert!(matches!(
            machine.advance_owner_result(forged),
            Err(GenericDagError::Route(
                RouteCatalogError::InvalidOwnerResult
            ))
        ));

        let mut forged = result(&producer, b"real-owner-payload");
        forged
            .outputs
            .get_mut(&PortId::from("document"))
            .unwrap()
            .scope
            .tenant_id = "other".into();
        assert!(machine.advance_owner_result(forged).is_err());

        let mut forged = result(&producer, b"real-owner-payload");
        forged.outputs.clear();
        assert!(machine.advance_owner_result(forged).is_err());

        let mut forged = result(&producer, b"real-owner-payload");
        forged
            .outputs
            .get_mut(&PortId::from("document"))
            .unwrap()
            .provenance = "".into();
        assert!(machine.advance_owner_result(forged).is_err());

        let mut forged = result(&producer, b"real-owner-payload");
        forged.dispatch_execution_key = "forged-key".into();
        assert!(machine.advance_owner_result(forged).is_err());

        let mut forged = result(&producer, b"real-owner-payload");
        forged.receipt_bytes.clear();
        assert!(machine.advance_owner_result(forged).is_err());
        // Rejections never advance the cursor or publish attacker artifacts.
        assert_eq!(
            machine.next_instance(),
            Some(&OperationInstanceId::from("producer"))
        );
    }

    #[test]
    fn rejects_unbound_produced_edge_with_incompatible_static_input_schema() {
        let (mut run, routes) = setup();
        run.plan.nodes[1].inputs.insert(
            "source".into(),
            ArtifactBinding::Produced {
                producer: "producer".into(),
                port: "document".into(),
                artifact: "artifact-b".into(),
            },
        );
        run.plan.hash = run.plan.canonical_hash().unwrap();
        assert!(OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).is_err());
    }

    #[test]
    fn oversized_child_result_never_publishes_an_artifact_or_advances_dag() {
        let (run, routes) = setup();
        let mut machine = OperationDagMachine::new(PARENT_WORKFLOW_ID, run, routes).unwrap();
        let (_, producer) = machine.next_owner_invocation(parent()).unwrap();
        let excessive = result(&producer, &vec![0xff; PURE_DAG_MAX_RESULT_BYTES]);
        assert!(matches!(
            machine.advance_owner_result(excessive),
            Err(GenericDagError::Run(
                JsonBudgetError::ResourceBudgetExceeded { .. }
            ))
        ));
        assert_eq!(
            machine.next_instance(),
            Some(&OperationInstanceId::from("producer"))
        );
    }
}
