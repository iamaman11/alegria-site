//! In-process, checked binding from generic plans to real pure operations.
//!
//! Only this adapter serializes operation values. Domain owners retain concrete
//! Rust Input/Output types; the protocol is unaware of the current site flow.
//! Stateful operations will need owner-specific adapters with preflight and
//! fencing; this executor intentionally admits only side-effect-free work.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    enforce_json_budget as protocol_enforce_json_budget, ArtifactBinding, ArtifactId,
    ArtifactSchema, ContractCatalog, ExecutionPlan, IdempotencyPolicy, JsonBudgetError, Operation,
    OperationInstanceId, OperationRef, PlanError, PlanHash, PortId, RunScope,
};
use seo_domain::extraction::operations::{
    CanonicalizeMentions, EditorialTopics, EntitySpans, LayerRoute, OperationalEntities,
    PureExtractionOperation,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod effectful_v1;
mod effectful_v4;
mod effectful_v5;
mod owner_dto_schema;
pub use effectful_v1::{
    decode_owner_child_result_v4, execute_checked_dag_v1, preflight_effect_dag_v1,
    CheckedDagOutcomeV1, EffectDagError, EffectDagRunV1, OwnerCommandV1, PreparedDagStepV1,
    PreparedDagV1,
};
pub use effectful_v4::{MixedDagMachineV4, MixedDagReceiptV4};
pub use effectful_v5::{
    AdmittedDagReceiptV5, IndependentOwnerChildV5, MixedDagMachineV5, NativeEffectRouteV5,
    ADMITTED_DAG_RECEIPT_SCHEMA_V5,
};
pub use owner_dto_schema::semantic_extraction_owner_schemas;
pub use operation_protocol::{GenericDagError, OperationDagMachine, OperationDagReceiptV1};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValue {
    pub schema: ArtifactSchema,
    pub scope: RunScope,
    pub provenance: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub order: Vec<OperationInstanceId>,
    pub artifacts: BTreeMap<ArtifactId, ArtifactValue>,
}

/// Opt-in, provider-free durable Workflow input. The external artifacts and
/// their payloads are captured in the Temporal start event. The provenance
/// field is a declared origin, NOT an authenticated database revision; this
/// runner is therefore deliberately forbidden from performing any I/O.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PureDagRun {
    pub plan: ExecutionPlan,
    pub admitted: BTreeMap<ArtifactId, ArtifactValue>,
}

/// Stable, inspectable output of a deterministic, side-effect-free run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PureDagReceipt {
    pub plan_hash: PlanHash,
    pub order: Vec<OperationInstanceId>,
    pub artifacts: BTreeMap<ArtifactId, ArtifactValue>,
}

// This first pure Temporal runner is intentionally small. These are limits
// on the actual JSON history payloads, not just on raw source text. A future
// worker/activity protocol may support larger artifacts outside History.
pub use operation_protocol::{
    DAG_MAX_INPUT_BYTES as PURE_DAG_MAX_INPUT_BYTES, DAG_MAX_NODES as PURE_DAG_MAX_NODES,
    DAG_MAX_RESULT_BYTES as PURE_DAG_MAX_RESULT_BYTES,
};

fn enforce_json_budget<T: Serialize>(
    value: &T,
    limit: usize,
    resource: &'static str,
) -> Result<(), OperationRunError> {
    protocol_enforce_json_budget(value, limit, resource).map_err(|error| match error {
        JsonBudgetError::ResourceBudgetExceeded { resource, limit } => {
            OperationRunError::ResourceBudgetExceeded { resource, limit }
        }
        JsonBudgetError::EnvelopeSerialization(message) => {
            OperationRunError::EnvelopeSerialization(message)
        }
    })
}

fn enforce_pure_dag_input_budget(run: &PureDagRun) -> Result<(), OperationRunError> {
    if run.plan.nodes.len() > PURE_DAG_MAX_NODES {
        return Err(OperationRunError::ResourceBudgetExceeded {
            resource: "operation_nodes",
            limit: PURE_DAG_MAX_NODES,
        });
    }
    let raw_bytes = run
        .admitted
        .values()
        .try_fold(0usize, |total, value| {
            total.checked_add(value.payload.len())
        })
        .unwrap_or(usize::MAX);
    if raw_bytes > PURE_DAG_MAX_INPUT_BYTES {
        return Err(OperationRunError::ResourceBudgetExceeded {
            resource: "input_bytes",
            limit: PURE_DAG_MAX_INPUT_BYTES,
        });
    }
    enforce_json_budget(run, PURE_DAG_MAX_INPUT_BYTES, "input_bytes")
}

#[derive(Debug, Error)]
pub enum OperationRunError {
    #[error(transparent)]
    InvalidPlan(#[from] PlanError),
    #[error("pure DAG is empty and cannot claim a completed operation run")]
    EmptyPurePlan,
    #[error("pure DAG refuses effectful operation or permission at {0:?}")]
    EffectfulPlan(OperationRef),
    #[error("pure DAG refuses global effects even without a currently consuming node")]
    EffectfulPlanEnvelope,
    #[error("pure DAG run id differs from the immutable Temporal workflow id")]
    TemporalRunIdMismatch,
    #[error("pure DAG {resource} exceeds resource budget {limit}")]
    ResourceBudgetExceeded {
        resource: &'static str,
        limit: usize,
    },
    #[error("pure DAG envelope cannot be serialized: {0}")]
    EnvelopeSerialization(String),
    #[error("operation {0:?} is not registered with this executor")]
    MissingExecutor(OperationRef),
    #[error("pure operation {0:?} requires exactly one input and output port and no effects")]
    UnsupportedPureBinding(OperationRef),
    #[error("missing admitted artifact {0:?}")]
    MissingArtifact(ArtifactId),
    #[error("external artifact {0:?} was not admitted by the manifest")]
    UnexpectedArtifact(ArtifactId),
    #[error("artifact {0:?} differs from its admitted schema, scope or provenance")]
    InvalidArtifactEnvelope(ArtifactId),
    #[error("operation {operation:?} cannot decode {artifact:?}: {reason}")]
    InvalidInput {
        operation: OperationRef,
        artifact: ArtifactId,
        reason: String,
    },
    #[error("operation {operation:?} output cannot be encoded: {reason}")]
    InvalidOutput {
        operation: OperationRef,
        reason: String,
    },
    #[error("operation {operation:?} produced invalid output port {port:?}")]
    InvalidOutputPort {
        operation: OperationRef,
        port: PortId,
    },
}

type Payloads = BTreeMap<PortId, (ArtifactId, Vec<u8>)>;
type Outputs = BTreeMap<PortId, Vec<u8>>;
type CheckedHandler = Box<dyn Fn(&Payloads) -> Result<Outputs, OperationRunError> + Send + Sync>;

/// Registration is the only location aware of the concrete domain operation.
/// Its key is the versioned contract, not an enum in the composition kernel.
pub struct OperationRegistry {
    catalog: ContractCatalog,
    handlers: BTreeMap<OperationRef, CheckedHandler>,
}

/// V3 pure Temporal History must retain its earlier Serde-only decode.
/// New V4 starts use strict, lossless JSON validation instead.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PureInputPolicy {
    LegacyV3Serde,
    StrictV4,
}

impl Default for OperationRegistry {
    fn default() -> Self {
        Self {
            catalog: ContractCatalog::default(),
            handlers: BTreeMap::new(),
        }
    }
}

impl OperationRegistry {
    pub fn register_pure<O: PureExtractionOperation>(&mut self) -> Result<(), OperationRunError> {
        self.register_pure_with_policy::<O>(PureInputPolicy::StrictV4)
    }

    fn register_pure_with_policy<O: PureExtractionOperation>(
        &mut self,
        input_policy: PureInputPolicy,
    ) -> Result<(), OperationRunError> {
        let contract = O::contract();
        if contract.inputs.len() != 1
            || contract.outputs.len() != 1
            || !contract.effects.is_empty()
            || contract.idempotency != IdempotencyPolicy::Pure
        {
            return Err(OperationRunError::UnsupportedPureBinding(
                contract.reference,
            ));
        }
        let (input_port, _) = contract.inputs.iter().next().expect("checked len");
        let (output_port, _) = contract.outputs.iter().next().expect("checked len");
        let input_port = input_port.clone();
        let output_port = output_port.clone();
        let reference = contract.reference.clone();
        let error_reference = reference.clone();
        let handler: CheckedHandler = Box::new(move |inputs: &Payloads| {
            let (artifact, bytes) =
                inputs
                    .get(&input_port)
                    .ok_or_else(|| OperationRunError::InvalidInput {
                        operation: error_reference.clone(),
                        artifact: ArtifactId::from("missing-input"),
                        reason: "required input port was not bound".to_owned(),
                    })?;
            // V3 was deployed with plain Serde decoding; keep that behavior
            // for historical replay. Only V4 refuses raw duplicate keys and
            // caller fields lost through a concrete owner's typed decoder.
            let authored = if input_policy == PureInputPolicy::StrictV4 {
                Some(
                    crate::typed_json::parse_unique_json_bytes(bytes).map_err(|error| {
                        OperationRunError::InvalidInput {
                            operation: error_reference.clone(),
                            artifact: artifact.clone(),
                            reason: format!("ambiguous JSON input: {error}"),
                        }
                    })?,
                )
            } else {
                None
            };
            let typed: O::Input =
                serde_json::from_slice(bytes).map_err(|error| OperationRunError::InvalidInput {
                    operation: error_reference.clone(),
                    artifact: artifact.clone(),
                    reason: error.to_string(),
                })?;
            if let Some(authored) = authored {
                let normalized =
                    crate::typed_json::normalized_wire_json_value(&typed).map_err(|error| {
                        OperationRunError::InvalidInput {
                            operation: error_reference.clone(),
                            artifact: artifact.clone(),
                            reason: format!("typed JSON normalization failed: {error}"),
                        }
                    })?;
                if !crate::typed_json::preserves_authored_fields(&authored, &normalized) {
                    return Err(OperationRunError::InvalidInput {
                        operation: error_reference.clone(),
                        artifact: artifact.clone(),
                        reason: "typed input discarded or altered an authored JSON field".into(),
                    });
                }
            }
            let output: O::Output = O::evaluate(&typed);
            let encoded =
                serde_json::to_vec(&output).map_err(|error| OperationRunError::InvalidOutput {
                    operation: error_reference.clone(),
                    reason: error.to_string(),
                })?;
            Ok(BTreeMap::from([(output_port.clone(), encoded)]))
        });
        self.catalog.register::<O>()?;
        self.handlers.insert(reference, handler);
        Ok(())
    }

    pub fn catalog(&self) -> &ContractCatalog {
        &self.catalog
    }

    /// Every manifest, revision, graph edge and external envelope is validated
    /// before a domain function runs. Domain calls here are pure and in-memory.
    pub fn execute(
        &self,
        plan: &ExecutionPlan,
        admitted: BTreeMap<ArtifactId, ArtifactValue>,
    ) -> Result<ExecutionResult, OperationRunError> {
        let checked = plan.validate(&self.catalog)?;
        // The registry contains only pure handlers. Reject a missing binding
        // for any node before calling the first operation, even if the missing
        // node would otherwise be reached only after earlier computations.
        for node in &plan.nodes {
            if !self.handlers.contains_key(&node.operation) {
                return Err(OperationRunError::MissingExecutor(node.operation.clone()));
            }
        }
        for (id, expected) in &plan.external_artifacts {
            let actual = admitted
                .get(id)
                .ok_or_else(|| OperationRunError::MissingArtifact(id.clone()))?;
            if actual.schema != expected.schema
                || actual.scope != expected.scope
                || actual.provenance != expected.provenance
            {
                return Err(OperationRunError::InvalidArtifactEnvelope(id.clone()));
            }
        }
        for id in admitted.keys() {
            if !plan.external_artifacts.contains_key(id) {
                return Err(OperationRunError::UnexpectedArtifact(id.clone()));
            }
        }

        let nodes: BTreeMap<_, _> = plan
            .nodes
            .iter()
            .map(|node| (node.instance.clone(), node))
            .collect();
        let mut artifacts = admitted;
        for instance in checked.execution_order() {
            let node = nodes
                .get(instance)
                .expect("validated DAG contains every scheduled instance");
            let handler = self
                .handlers
                .get(&node.operation)
                .ok_or_else(|| OperationRunError::MissingExecutor(node.operation.clone()))?;
            let mut inputs = Payloads::new();
            for (port, binding) in &node.inputs {
                let artifact = match binding {
                    ArtifactBinding::External { artifact }
                    | ArtifactBinding::Produced { artifact, .. } => artifact,
                };
                let value = artifacts
                    .get(artifact)
                    .ok_or_else(|| OperationRunError::MissingArtifact(artifact.clone()))?;
                inputs.insert(port.clone(), (artifact.clone(), value.payload.clone()));
            }
            let mut outputs = handler(&inputs)?;
            let contract = self
                .catalog
                .contract(&node.operation)
                .expect("validated contract remains immutable for the execution");
            let expected_ports: BTreeSet<_> = contract.outputs.keys().cloned().collect();
            if outputs.keys().cloned().collect::<BTreeSet<_>>() != expected_ports {
                let port = outputs
                    .keys()
                    .find(|port| !expected_ports.contains(*port))
                    .cloned()
                    .or_else(|| {
                        expected_ports
                            .iter()
                            .find(|port| !outputs.contains_key(*port))
                            .cloned()
                    })
                    .expect("unequal output port sets have a difference");
                return Err(OperationRunError::InvalidOutputPort {
                    operation: node.operation.clone(),
                    port,
                });
            }
            for (port, id) in &node.outputs {
                let schema = contract
                    .outputs
                    .get(port)
                    .expect("plan validated every producer output port");
                let payload = outputs
                    .remove(port)
                    .expect("checked output keys cover every producer port");
                artifacts.insert(
                    id.clone(),
                    ArtifactValue {
                        schema: schema.clone(),
                        scope: plan.scope.clone(),
                        provenance: format!(
                            "operation:{}@{}/{}#{}",
                            node.operation.id.as_str(),
                            node.operation.revision,
                            instance.as_str(),
                            plan.hash.as_str()
                        ),
                        payload,
                    },
                );
            }
        }
        Ok(ExecutionResult {
            order: checked.execution_order().to_vec(),
            artifacts,
        })
    }
}

/// Bind a generic immutable DAG to real domain operations in a pure-only
/// Temporal-compatible envelope. This has no SQL, HTTP, Graph, Vector or
/// Temporal SDK dependency. Effectful operations require their own future
/// owner-authorized durable activity/runtime and must never be admitted here.
pub fn execute_pure_dag(
    workflow_id: &str,
    run: &PureDagRun,
) -> Result<PureDagReceipt, OperationRunError> {
    execute_pure_dag_with_registry(workflow_id, run, semantic_extraction_registry()?)
}

/// Historical V3 pure Workflow replay must keep earlier typed Serde-only
/// admission. New V4 workflow starts always use strict `execute_pure_dag`.
pub fn execute_pure_dag_legacy_v3(
    workflow_id: &str,
    run: &PureDagRun,
) -> Result<PureDagReceipt, OperationRunError> {
    execute_pure_dag_with_registry(workflow_id, run, semantic_extraction_registry_legacy_v3()?)
}

fn execute_pure_dag_with_registry(
    workflow_id: &str,
    run: &PureDagRun,
    registry: OperationRegistry,
) -> Result<PureDagReceipt, OperationRunError> {
    if workflow_id != run.plan.scope.run_id {
        return Err(OperationRunError::TemporalRunIdMismatch);
    }
    if run.plan.nodes.is_empty() {
        return Err(OperationRunError::EmptyPurePlan);
    }
    if !run.plan.allowed_effects.is_empty() {
        return Err(OperationRunError::EffectfulPlanEnvelope);
    }
    enforce_pure_dag_input_budget(run)?;
    // Never seal an incoming plan: its original fingerprints and hash must
    // survive Temporal workflow replay unchanged.
    run.plan.validate(registry.catalog())?;
    for node in &run.plan.nodes {
        let contract = registry
            .catalog()
            .contract(&node.operation)
            .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
        if contract.idempotency != IdempotencyPolicy::Pure
            || !contract.effects.is_empty()
            || !node.permitted_effects.is_empty()
        {
            return Err(OperationRunError::EffectfulPlan(node.operation.clone()));
        }
    }
    let executed = registry.execute(&run.plan, run.admitted.clone())?;
    let receipt = PureDagReceipt {
        plan_hash: run.plan.hash.clone(),
        order: executed.order,
        artifacts: executed.artifacts,
    };
    // Do not return an oversized payload to the Temporal SDK for History
    // recording. Any failure here is pure and can be retried without effects.
    enforce_json_budget(&receipt, PURE_DAG_MAX_RESULT_BYTES, "result_bytes")?;
    Ok(receipt)
}

/// One owner-specific binding site. New domain operations are registered here
/// without changing the protocol, DAG validator or neighboring operations.
pub fn semantic_extraction_registry() -> Result<OperationRegistry, OperationRunError> {
    let mut registry = OperationRegistry::default();
    registry.register_pure::<LayerRoute>()?;
    registry.register_pure::<EntitySpans>()?;
    registry.register_pure::<CanonicalizeMentions>()?;
    registry.register_pure::<EditorialTopics>()?;
    registry.register_pure::<OperationalEntities>()?;
    Ok(registry)
}

fn semantic_extraction_registry_legacy_v3() -> Result<OperationRegistry, OperationRunError> {
    let mut registry = OperationRegistry::default();
    registry.register_pure_with_policy::<LayerRoute>(PureInputPolicy::LegacyV3Serde)?;
    registry.register_pure_with_policy::<EntitySpans>(PureInputPolicy::LegacyV3Serde)?;
    registry.register_pure_with_policy::<CanonicalizeMentions>(PureInputPolicy::LegacyV3Serde)?;
    registry.register_pure_with_policy::<EditorialTopics>(PureInputPolicy::LegacyV3Serde)?;
    registry.register_pure_with_policy::<OperationalEntities>(PureInputPolicy::LegacyV3Serde)?;
    Ok(registry)
}

#[cfg(test)]
mod tests;
