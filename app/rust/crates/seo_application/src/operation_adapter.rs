//! In-process, checked binding from generic plans to real pure operations.
//!
//! Only this adapter serializes operation values. Domain owners retain concrete
//! Rust Input/Output types; the protocol is unaware of the current site flow.
//! Stateful operations will need owner-specific adapters with preflight and
//! fencing; this executor intentionally admits only side-effect-free work.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactBinding, ArtifactId, ArtifactSchema, ContractCatalog, ExecutionPlan, IdempotencyPolicy,
    Operation, OperationInstanceId, OperationRef, PlanError, PortId, RunScope,
};
use seo_domain::extraction::operations::{
    CanonicalizeMentions, EditorialTopics, EntitySpans, LayerRoute, OperationalEntities,
    PureExtractionOperation,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Error)]
pub enum OperationRunError {
    #[error(transparent)]
    InvalidPlan(#[from] PlanError),
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
    InvalidOutputPort { operation: OperationRef, port: PortId },
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
        let contract = O::contract();
        if contract.inputs.len() != 1
            || contract.outputs.len() != 1
            || !contract.effects.is_empty()
            || contract.idempotency != IdempotencyPolicy::Pure
        {
            return Err(OperationRunError::UnsupportedPureBinding(contract.reference));
        }
        let (input_port, _) = contract.inputs.iter().next().expect("checked len");
        let (output_port, _) = contract.outputs.iter().next().expect("checked len");
        let input_port = input_port.clone();
        let output_port = output_port.clone();
        let reference = contract.reference.clone();
        let error_reference = reference.clone();
        let handler: CheckedHandler = Box::new(move |inputs: &Payloads| {
            let (artifact, bytes) = inputs
                .get(&input_port)
                .ok_or_else(|| OperationRunError::InvalidInput {
                    operation: error_reference.clone(),
                    artifact: ArtifactId::from("missing-input"),
                    reason: "required input port was not bound".to_owned(),
                })?;
            let typed: O::Input =
                serde_json::from_slice(bytes).map_err(|error| OperationRunError::InvalidInput {
                    operation: error_reference.clone(),
                    artifact: artifact.clone(),
                    reason: error.to_string(),
                })?;
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

#[cfg(test)]
mod tests;
