//! Provider-free contracts and validation of versioned operation graphs.
//!
//! This library never executes an operation or reads external state. It
//! validates the complete immutable graph before a runner may schedule I/O.
//! Operation implementations and provider adapters belong to their owners.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod effect_child_v4;
pub mod owner_dispatch;
pub mod owner_validation;
pub use effect_child_v4::{
    canonical_effect_dispatch_input_bytes, checked_execution_key, BoundEffectInputV4,
    EffectChildInvocationV4, EffectChildProtocolError, EffectChildResultV4,
    TemporalExecutionIdentityV4, EFFECT_CHILD_SCHEMA_REVISION,
};
pub use owner_dispatch::{
    canonical_owner_dispatch_input_bytes, canonical_owner_result_hash,
    operation_dag_parent_workflow_id, operation_dag_parent_workflow_id_v2, owner_child_workflow_id,
    owner_child_workflow_id_v2, owner_content_hash, AdmittedArtifactV1, BoundArtifactV1,
    OperationDagRunV6, OwnerCommandWireV1,
    OwnerInvocationV1, OwnerResultV1, PinnedOwnerRouteV1, ProducedArtifactEvidenceV1,
    RouteCatalogError, TrustedRouteCatalog, OPERATION_DAG_PARENT_QUEUE,
    OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE, OPERATION_DAG_PARENT_TYPE_V2,
    OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX, OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX_V2,
    OPERATION_DAG_RUN_SCHEMA_REVISION_V6, OWNER_INVOCATION_SCHEMA_REVISION,
    OWNER_OPERATION_CHILD_TYPE_V1,
};
pub use owner_validation::{
    owner_validation_child_workflow_id_v1, OwnerValidationError, OwnerValidationRequestV1,
    OwnerValidationResultV1, OWNER_VALIDATION_CHILD_TYPE_V1, OWNER_VALIDATION_SCHEMA_REVISION_V1,
};

/// Strongly typed identities prevent accidentally exchanging a node, port,
/// operation or artifact name at a binding boundary.
macro_rules! identity {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

identity!(OperationId);
identity!(OperationOwnerId);
identity!(OperationInstanceId);
identity!(PortId);
identity!(ArtifactId);
identity!(ArtifactTypeId);
identity!(EffectId);
identity!(PlanHash);
identity!(ContractHash);

// Revision 2 pins owner-declared invalidation semantics. A revision-1 plan
// cannot be reinterpreted as safe merely because its ports still match.
pub const PLAN_SCHEMA_REVISION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRef {
    pub id: OperationId,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSchema {
    pub id: ArtifactTypeId,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyPolicy {
    /// A deterministic, side-effect-free transformation.
    Pure,
    /// Effects must be idempotent under one operation-instance execution key.
    Idempotent,
    /// A runner must not retry automatically after an ambiguous effect.
    AtMostOnce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationContract {
    pub reference: OperationRef,
    pub owner: OperationOwnerId,
    pub inputs: BTreeMap<PortId, ArtifactSchema>,
    pub outputs: BTreeMap<PortId, ArtifactSchema>,
    /// The owner declares artifact *types* made stale by this operation.
    /// A plan may reorder otherwise independent operations, but it may not
    /// read an invalidated type until a causally later producer refreshes it.
    /// These declarations participate in the pinned contract fingerprint.
    pub invalidates: BTreeSet<ArtifactTypeId>,
    pub effects: BTreeSet<EffectId>,
    pub idempotency: IdempotencyPolicy,
}

impl OperationContract {
    /// Canonical owner-contract fingerprint shared with independently
    /// credentialed operator proof. Equality pins the declared schema only;
    /// it is NOT operator admission or authorization to run an Activity.
    pub fn fingerprint(&self) -> Result<ContractHash, PlanError> {
        let encoded = serde_json::to_vec(self)
            .map_err(|error| PlanError::Serialization(error.to_string()))?;
        Ok(ContractHash(blake3::hash(&encoded).to_hex().to_string()))
    }
}

/// A concrete owner retains its Rust types. Erasure is allowed only at the
/// runner's checked serialization/registration boundary, never in the domain.
pub trait Operation {
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Failure: Error + Send + Sync + 'static;

    fn contract() -> OperationContract;
}

#[derive(Debug, Default, Clone)]
pub struct ContractCatalog {
    contracts: BTreeMap<OperationRef, OperationContract>,
}

impl ContractCatalog {
    pub fn register<O: Operation>(&mut self) -> Result<(), PlanError> {
        self.register_contract(O::contract())
    }

    /// Install a trusted, owner-declared contract without linking the
    /// provider-specific Rust crate into the generic DAG runtime. The caller
    /// is responsible for obtaining this from a trusted registration source;
    /// an arbitrary manifest must never be treated as a trusted catalog.
    pub fn register_contract(&mut self, contract: OperationContract) -> Result<(), PlanError> {
        verify_contract(&contract)?;
        if self.contracts.contains_key(&contract.reference) {
            return Err(PlanError::DuplicateContract(contract.reference));
        }
        self.contracts.insert(contract.reference.clone(), contract);
        Ok(())
    }

    pub fn contract(&self, reference: &OperationRef) -> Option<&OperationContract> {
        self.contracts.get(reference)
    }

    pub fn contracts(&self) -> impl Iterator<Item = &OperationContract> {
        self.contracts.values()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunScope {
    pub tenant_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalArtifact {
    pub schema: ArtifactSchema,
    pub scope: RunScope,
    /// Non-empty, independently verifiable origin of admitted evidence.
    pub provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactBinding {
    External {
        artifact: ArtifactId,
    },
    Produced {
        producer: OperationInstanceId,
        port: PortId,
        artifact: ArtifactId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationNode {
    pub instance: OperationInstanceId,
    pub operation: OperationRef,
    /// Consumer input port -> explicitly identified external or produced data.
    pub inputs: BTreeMap<PortId, ArtifactBinding>,
    /// Producer output port -> plan-scoped artifact identity.
    pub outputs: BTreeMap<PortId, ArtifactId>,
    /// Ordering constraints that do not transport an artifact.
    pub after: BTreeSet<OperationInstanceId>,
    /// Exact set of effects this concrete node is authorized to request.
    pub permitted_effects: BTreeSet<EffectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedContract {
    pub reference: OperationRef,
    pub fingerprint: ContractHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub schema_revision: u16,
    pub scope: RunScope,
    pub allowed_effects: BTreeSet<EffectId>,
    pub external_artifacts: BTreeMap<ArtifactId, ExternalArtifact>,
    /// Full contract fingerprints (including owner/effects/schemas), pinned to
    /// operation identity and revision at the start of the plan. A sorted list
    /// is required because JSON object keys cannot be struct-valued.
    pub pinned_contracts: Vec<PinnedContract>,
    pub nodes: Vec<OperationNode>,
    pub hash: PlanHash,
}

/// The checked, deterministic schedule and identity supplied to any runner.
/// Only a validated plan may derive retry/idempotency keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPlan {
    hash: PlanHash,
    execution_order: Vec<OperationInstanceId>,
    scope: RunScope,
    operations: BTreeMap<OperationInstanceId, OperationRef>,
    dispatch_contracts: BTreeMap<OperationInstanceId, DispatchContract>,
}

/// A retry after an unknown external outcome is fundamentally different from
/// scheduling the first attempt. Temporal may retry only when an owner has
/// declared an idempotent operation and can enforce its durable receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchAttempt {
    First,
    RetryAfterUnknownOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DispatchContract {
    owner: OperationOwnerId,
    effects: BTreeSet<EffectId>,
    idempotency: IdempotencyPolicy,
}

/// Immutable plan-derived facts for one checked node. This is NOT an operator
/// admission, storage lease or permission to execute an arbitrary RPC: an
/// effect owner must independently authorize the external side effect and
/// atomically bind the returned receipt to this exact execution key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckedDispatch {
    pub plan_hash: PlanHash,
    pub scope: RunScope,
    pub instance: OperationInstanceId,
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub effects: BTreeSet<EffectId>,
    pub idempotency: IdempotencyPolicy,
    pub input_hash: String,
    pub execution_key: String,
}

impl ValidatedPlan {
    pub fn hash(&self) -> &PlanHash {
        &self.hash
    }

    pub fn execution_order(&self) -> &[OperationInstanceId] {
        &self.execution_order
    }

    /// Bind the exact input bytes to an individual versioned node before the
    /// runner schedules it. A caller-provided *label* or claimed input digest
    /// is insufficient for effectful dispatch. This only checks the manifest;
    /// external authorization, actual activity provenance and receipt fencing
    /// remain the effect owner's responsibility.
    pub fn checked_dispatch(
        &self,
        instance: &OperationInstanceId,
        actual_input_bytes: &[u8],
        attempt: DispatchAttempt,
    ) -> Result<CheckedDispatch, PlanError> {
        let operation = self
            .operations
            .get(instance)
            .ok_or_else(|| PlanError::UnknownInstance(instance.clone()))?;
        let contract = self
            .dispatch_contracts
            .get(instance)
            .ok_or_else(|| PlanError::UnknownInstance(instance.clone()))?;
        if attempt == DispatchAttempt::RetryAfterUnknownOutcome
            && contract.idempotency == IdempotencyPolicy::AtMostOnce
        {
            return Err(PlanError::AmbiguousEffectRetry(instance.clone()));
        }
        // Hash actual bytes, not a digest supplied by an untrusted operator.
        // This identity is stable across retries and distinct across intentional
        // operation instances, plan revisions and different payloads.
        let digest = blake3::hash(actual_input_bytes).to_hex().to_string();
        let execution_key = self.execution_key(instance, &digest)?;
        Ok(CheckedDispatch {
            plan_hash: self.hash.clone(),
            scope: self.scope.clone(),
            instance: instance.clone(),
            operation: operation.clone(),
            owner: contract.owner.clone(),
            effects: contract.effects.clone(),
            idempotency: contract.idempotency,
            input_hash: digest,
            execution_key,
        })
    }

    /// Retries of the same node with the same input produce an identical key;
    /// a second intentional node with identical bytes has a different key.
    pub fn execution_key(
        &self,
        instance: &OperationInstanceId,
        input_hash: &str,
    ) -> Result<String, PlanError> {
        let operation = self
            .operations
            .get(instance)
            .ok_or_else(|| PlanError::UnknownInstance(instance.clone()))?;
        nonempty(input_hash, "input_hash")?;
        let encoded =
            serde_json::to_vec(&(&self.scope, &self.hash, instance, operation, input_hash))
                .map_err(|error| PlanError::Serialization(error.to_string()))?;
        Ok(blake3::hash(&encoded).to_hex().to_string())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlanError {
    #[error("unsupported plan schema revision {0}")]
    UnsupportedPlanRevision(u16),
    #[error("missing or empty required identifier: {0}")]
    EmptyIdentifier(&'static str),
    #[error("invalid operation contract: {0}")]
    InvalidContract(String),
    #[error("operation contract already registered: {0:?}")]
    DuplicateContract(OperationRef),
    #[error("unknown operation or contract revision: {0:?}")]
    UnknownOperation(OperationRef),
    #[error("contract is not pinned in immutable plan: {0:?}")]
    ContractNotPinned(OperationRef),
    #[error("plan pins unused operation contract: {0:?}")]
    UnexpectedPinnedContract(OperationRef),
    #[error("plan pins operation contract more than once: {0:?}")]
    DuplicatePinnedContract(OperationRef),
    #[error("contract owner, schemas, invalidations, effects or idempotency changed for pinned operation: {0:?}")]
    ContractFingerprintMismatch(OperationRef),
    #[error("duplicate operation instance: {0:?}")]
    DuplicateInstance(OperationInstanceId),
    #[error("unknown operation instance: {0:?}")]
    UnknownInstance(OperationInstanceId),
    #[error("missing {direction} port {port:?} of {instance:?}")]
    MissingPort {
        instance: OperationInstanceId,
        port: PortId,
        direction: &'static str,
    },
    #[error("unexpected {direction} port {port:?} of {instance:?}")]
    UnexpectedPort {
        instance: OperationInstanceId,
        port: PortId,
        direction: &'static str,
    },
    #[error("missing external artifact {0:?}")]
    MissingExternalArtifact(ArtifactId),
    #[error("artifact {0:?} has no provenance")]
    MissingProvenance(ArtifactId),
    #[error("artifact {0:?} belongs to another run/tenant scope")]
    CrossScopeArtifact(ArtifactId),
    #[error("artifact {artifact:?} has incorrect schema at {instance:?}/{port:?}")]
    ArtifactSchemaMismatch {
        instance: OperationInstanceId,
        port: PortId,
        artifact: ArtifactId,
    },
    #[error("artifact {0:?} has multiple producers")]
    AmbiguousArtifact(ArtifactId),
    #[error("unknown produced artifact {artifact:?} from {producer:?}/{port:?}")]
    MissingProducedArtifact {
        producer: OperationInstanceId,
        port: PortId,
        artifact: ArtifactId,
    },
    #[error("node {instance:?} requests an unapproved or undeclared effect {effect:?}")]
    ForbiddenEffect {
        instance: OperationInstanceId,
        effect: EffectId,
    },
    #[error("node {instance:?} has no authorization for required effect {effect:?}")]
    MissingEffect {
        instance: OperationInstanceId,
        effect: EffectId,
    },
    #[error(
        "ambiguous outcome at at-most-once operation {0:?}: retry requires operator reconciliation"
    )]
    AmbiguousEffectRetry(OperationInstanceId),
    #[error("reader {consumer:?} of artifact type {artifact_type:?} is unordered relative to invalidator {invalidator:?}")]
    UnorderedArtifactHazard {
        invalidator: OperationInstanceId,
        consumer: OperationInstanceId,
        artifact_type: ArtifactTypeId,
    },
    #[error("consumer {consumer:?} reads artifact {artifact:?} of type {artifact_type:?} made stale by {invalidator:?}")]
    StaleArtifact {
        invalidator: OperationInstanceId,
        consumer: OperationInstanceId,
        artifact: ArtifactId,
        artifact_type: ArtifactTypeId,
    },
    #[error("invalidators {first:?} and {second:?} of artifact type {artifact_type:?} have no causal order")]
    UnorderedInvalidations {
        first: OperationInstanceId,
        second: OperationInstanceId,
        artifact_type: ArtifactTypeId,
    },
    #[error("operation dependency graph contains a cycle")]
    Cycle,
    #[error("immutable plan hash differs from its canonical contents")]
    PlanHashMismatch,
    #[error("plan serialization failed: {0}")]
    Serialization(String),
}

fn nonempty(value: &str, label: &'static str) -> Result<(), PlanError> {
    if value.trim().is_empty() {
        Err(PlanError::EmptyIdentifier(label))
    } else {
        Ok(())
    }
}

fn schema_valid(schema: &ArtifactSchema) -> Result<(), PlanError> {
    nonempty(schema.id.as_str(), "artifact_type_id")?;
    if schema.revision == 0 {
        return Err(PlanError::InvalidContract(
            "artifact schema revision must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn verify_contract(contract: &OperationContract) -> Result<(), PlanError> {
    nonempty(contract.reference.id.as_str(), "operation_id")?;
    nonempty(contract.owner.as_str(), "operation_owner_id")?;
    if contract.reference.revision == 0 {
        return Err(PlanError::InvalidContract(
            "operation revision must be positive".to_owned(),
        ));
    }
    for (port, schema) in contract.inputs.iter().chain(&contract.outputs) {
        nonempty(port.as_str(), "contract_port")?;
        schema_valid(schema)?;
    }
    for effect in &contract.effects {
        nonempty(effect.as_str(), "effect_id")?;
    }
    for artifact_type in &contract.invalidates {
        nonempty(artifact_type.as_str(), "invalidated_artifact_type_id")?;
    }
    if contract.idempotency == IdempotencyPolicy::Pure && !contract.effects.is_empty() {
        return Err(PlanError::InvalidContract(
            "pure operation cannot declare side effects".to_owned(),
        ));
    }
    Ok(())
}

impl ExecutionPlan {
    /// A manifest is sealed once, prior to scheduling. No ambient current
    /// state, remote service, clock or random generator affects its identity.
    /// Only use on a newly authored draft: sealing replaces pinned contracts.
    /// Loaded, untrusted or resumed plans MUST call `validate` as-is instead,
    /// so an owner contract change cannot be silently re-pinned.
    pub fn seal(mut self, catalog: &ContractCatalog) -> Result<Self, PlanError> {
        let referenced: BTreeSet<_> = self
            .nodes
            .iter()
            .map(|node| node.operation.clone())
            .collect();
        self.pinned_contracts.clear();
        for reference in referenced {
            let contract = catalog
                .contract(&reference)
                .ok_or_else(|| PlanError::UnknownOperation(reference.clone()))?;
            self.pinned_contracts.push(PinnedContract {
                reference,
                fingerprint: contract.fingerprint()?,
            });
        }
        self.hash = self.canonical_hash()?;
        Ok(self)
    }

    /// Normalizing independent node order avoids a spurious identity change
    /// when two authors serialize the same DAG in a different vector order.
    pub fn canonical_hash(&self) -> Result<PlanHash, PlanError> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            schema_revision: u16,
            scope: &'a RunScope,
            allowed_effects: &'a BTreeSet<EffectId>,
            external_artifacts: &'a BTreeMap<ArtifactId, ExternalArtifact>,
            pinned_contracts: Vec<&'a PinnedContract>,
            nodes: Vec<&'a OperationNode>,
        }
        let mut nodes: Vec<_> = self.nodes.iter().collect();
        nodes.sort_by(|a, b| a.instance.cmp(&b.instance));
        let mut pinned_contracts: Vec<_> = self.pinned_contracts.iter().collect();
        pinned_contracts.sort_by(|a, b| {
            a.reference
                .cmp(&b.reference)
                .then_with(|| a.fingerprint.cmp(&b.fingerprint))
        });
        let encoded = serde_json::to_vec(&Canonical {
            schema_revision: self.schema_revision,
            scope: &self.scope,
            allowed_effects: &self.allowed_effects,
            external_artifacts: &self.external_artifacts,
            pinned_contracts,
            nodes,
        })
        .map_err(|error| PlanError::Serialization(error.to_string()))?;
        Ok(PlanHash(blake3::hash(&encoded).to_hex().to_string()))
    }

    pub fn validate(&self, catalog: &ContractCatalog) -> Result<ValidatedPlan, PlanError> {
        if self.schema_revision != PLAN_SCHEMA_REVISION {
            return Err(PlanError::UnsupportedPlanRevision(self.schema_revision));
        }
        nonempty(&self.scope.tenant_id, "tenant_id")?;
        nonempty(&self.scope.run_id, "run_id")?;
        for effect in &self.allowed_effects {
            nonempty(effect.as_str(), "allowed_effect")?;
        }
        if self.hash != self.canonical_hash()? {
            return Err(PlanError::PlanHashMismatch);
        }

        let referenced: BTreeSet<_> = self
            .nodes
            .iter()
            .map(|node| node.operation.clone())
            .collect();
        let mut pinned = BTreeMap::<OperationRef, &ContractHash>::new();
        for contract in &self.pinned_contracts {
            if pinned
                .insert(contract.reference.clone(), &contract.fingerprint)
                .is_some()
            {
                return Err(PlanError::DuplicatePinnedContract(
                    contract.reference.clone(),
                ));
            }
        }
        for reference in &referenced {
            let contract = catalog
                .contract(reference)
                .ok_or_else(|| PlanError::UnknownOperation(reference.clone()))?;
            verify_contract(contract)?;
            let fingerprint = pinned
                .get(reference)
                .ok_or_else(|| PlanError::ContractNotPinned(reference.clone()))?;
            if &contract.fingerprint()? != *fingerprint {
                return Err(PlanError::ContractFingerprintMismatch(reference.clone()));
            }
        }
        for reference in pinned.keys() {
            if !referenced.contains(reference) {
                return Err(PlanError::UnexpectedPinnedContract(reference.clone()));
            }
        }

        let mut by_id = BTreeMap::new();
        for node in &self.nodes {
            nonempty(node.instance.as_str(), "operation_instance_id")?;
            if by_id.insert(node.instance.clone(), node).is_some() {
                return Err(PlanError::DuplicateInstance(node.instance.clone()));
            }
        }

        let mut outputs = BTreeMap::<ArtifactId, (OperationInstanceId, PortId)>::new();
        for node in &self.nodes {
            let contract = catalog
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            verify_contract(contract)?;
            verify_ports(node, &contract.inputs, &node.inputs, "input")?;
            verify_ports(node, &contract.outputs, &node.outputs, "output")?;
            for (port, id) in &node.outputs {
                nonempty(id.as_str(), "output_artifact_id")?;
                if self.external_artifacts.contains_key(id)
                    || outputs
                        .insert(id.clone(), (node.instance.clone(), port.clone()))
                        .is_some()
                {
                    return Err(PlanError::AmbiguousArtifact(id.clone()));
                }
            }
            for required in &contract.effects {
                if !node.permitted_effects.contains(required) {
                    return Err(PlanError::MissingEffect {
                        instance: node.instance.clone(),
                        effect: required.clone(),
                    });
                }
            }
            for permitted in &node.permitted_effects {
                if !contract.effects.contains(permitted)
                    || !self.allowed_effects.contains(permitted)
                {
                    return Err(PlanError::ForbiddenEffect {
                        instance: node.instance.clone(),
                        effect: permitted.clone(),
                    });
                }
            }
        }

        for (id, artifact) in &self.external_artifacts {
            nonempty(id.as_str(), "external_artifact_id")?;
            schema_valid(&artifact.schema)?;
            if artifact.scope != self.scope {
                return Err(PlanError::CrossScopeArtifact(id.clone()));
            }
            if artifact.provenance.trim().is_empty() {
                return Err(PlanError::MissingProvenance(id.clone()));
            }
        }

        let mut dependencies =
            BTreeMap::<OperationInstanceId, BTreeSet<OperationInstanceId>>::new();
        for node in &self.nodes {
            let contract = catalog
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            let mut after = node.after.clone();
            for (port, binding) in &node.inputs {
                let expected =
                    contract
                        .inputs
                        .get(port)
                        .ok_or_else(|| PlanError::UnexpectedPort {
                            instance: node.instance.clone(),
                            port: port.clone(),
                            direction: "input",
                        })?;
                match binding {
                    ArtifactBinding::External { artifact } => {
                        let actual = self
                            .external_artifacts
                            .get(artifact)
                            .ok_or_else(|| PlanError::MissingExternalArtifact(artifact.clone()))?;
                        if &actual.schema != expected {
                            return Err(PlanError::ArtifactSchemaMismatch {
                                instance: node.instance.clone(),
                                port: port.clone(),
                                artifact: artifact.clone(),
                            });
                        }
                    }
                    ArtifactBinding::Produced {
                        producer,
                        port: producer_port,
                        artifact,
                    } => {
                        let predecessor = by_id
                            .get(producer)
                            .ok_or_else(|| PlanError::UnknownInstance(producer.clone()))?;
                        let previous_contract =
                            catalog.contract(&predecessor.operation).ok_or_else(|| {
                                PlanError::UnknownOperation(predecessor.operation.clone())
                            })?;
                        let produced_schema = previous_contract.outputs.get(producer_port);
                        let output_id = predecessor.outputs.get(producer_port);
                        if output_id != Some(artifact) || produced_schema.is_none() {
                            return Err(PlanError::MissingProducedArtifact {
                                producer: producer.clone(),
                                port: producer_port.clone(),
                                artifact: artifact.clone(),
                            });
                        }
                        if produced_schema != Some(expected) {
                            return Err(PlanError::ArtifactSchemaMismatch {
                                instance: node.instance.clone(),
                                port: port.clone(),
                                artifact: artifact.clone(),
                            });
                        }
                        after.insert(producer.clone());
                    }
                }
            }
            for predecessor in &after {
                if !by_id.contains_key(predecessor) {
                    return Err(PlanError::UnknownInstance(predecessor.clone()));
                }
            }
            dependencies.insert(node.instance.clone(), after);
        }

        let mut dependents = BTreeMap::<OperationInstanceId, BTreeSet<OperationInstanceId>>::new();
        let mut degree = BTreeMap::<OperationInstanceId, usize>::new();
        for (id, deps) in &dependencies {
            degree.insert(id.clone(), deps.len());
            for previous in deps {
                dependents
                    .entry(previous.clone())
                    .or_default()
                    .insert(id.clone());
            }
        }
        let mut ready: BTreeSet<_> = degree
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop_first() {
            order.push(id.clone());
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    let remaining = degree
                        .get_mut(child)
                        .ok_or_else(|| PlanError::UnknownInstance(child.clone()))?;
                    *remaining -= 1;
                    if *remaining == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(PlanError::Cycle);
        }
        // A deterministic tie-break for unrelated nodes does NOT create a
        // happens-before edge. Invalidation safety uses the transitive closure
        // of explicit `after` and produced-artifact dependencies only.
        let mut ancestors = BTreeMap::<OperationInstanceId, BTreeSet<OperationInstanceId>>::new();
        for id in &order {
            let mut prior = BTreeSet::new();
            for predecessor in &dependencies[id] {
                prior.insert(predecessor.clone());
                prior.extend(ancestors[predecessor].iter().cloned());
            }
            ancestors.insert(id.clone(), prior);
        }
        self.verify_freshness_dependencies(catalog, &by_id, &order, &ancestors)?;
        Ok(ValidatedPlan {
            hash: self.hash.clone(),
            execution_order: order,
            scope: self.scope.clone(),
            operations: by_id
                .iter()
                .map(|(id, node)| (id.clone(), node.operation.clone()))
                .collect(),
            dispatch_contracts: by_id
                .into_iter()
                .map(|(id, node)| {
                    let contract = catalog
                        .contract(&node.operation)
                        .expect("already validated all registered operation contracts");
                    (
                        id,
                        DispatchContract {
                            owner: contract.owner.clone(),
                            effects: contract.effects.clone(),
                            idempotency: contract.idempotency,
                        },
                    )
                })
                .collect(),
        })
    }

    fn verify_freshness_dependencies(
        &self,
        catalog: &ContractCatalog,
        by_id: &BTreeMap<OperationInstanceId, &OperationNode>,
        order: &[OperationInstanceId],
        ancestors: &BTreeMap<OperationInstanceId, BTreeSet<OperationInstanceId>>,
    ) -> Result<(), PlanError> {
        // Conservatively inherit ALL input artifact types through every
        // output. An old support bundle embedded in a draft is still stale
        // after the support changes, even if its direct output type is Draft.
        // Owner-specific narrower lineage needs a separately pinned contract;
        // a missing declaration must never silently certify fresh artifacts.
        let mut lineage = self
            .external_artifacts
            .iter()
            .map(|(id, artifact)| (id.clone(), BTreeSet::from([artifact.schema.id.clone()])))
            .collect::<BTreeMap<ArtifactId, BTreeSet<ArtifactTypeId>>>();
        for id in order {
            let node = by_id[id];
            let contract = catalog
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            let mut inherited = BTreeSet::new();
            for binding in node.inputs.values() {
                let artifact = match binding {
                    ArtifactBinding::External { artifact }
                    | ArtifactBinding::Produced { artifact, .. } => artifact,
                };
                inherited.extend(
                    lineage
                        .get(artifact)
                        .ok_or_else(|| PlanError::MissingExternalArtifact(artifact.clone()))?
                        .iter()
                        .cloned(),
                );
            }
            for (port, artifact) in &node.outputs {
                let schema =
                    contract
                        .outputs
                        .get(port)
                        .ok_or_else(|| PlanError::UnexpectedPort {
                            instance: id.clone(),
                            port: port.clone(),
                            direction: "output",
                        })?;
                let mut inherited_output = inherited.clone();
                inherited_output.insert(schema.id.clone());
                lineage.insert(artifact.clone(), inherited_output);
            }
        }

        let mut invalidators = Vec::<(OperationInstanceId, ArtifactTypeId)>::new();
        for (id, node) in by_id {
            let contract = catalog
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            for artifact_type in &contract.invalidates {
                invalidators.push((id.clone(), artifact_type.clone()));
            }
        }

        // Without a declared aggregate-key partition, two changes to the
        // same artifact type cannot run concurrently: each may invalidate the
        // other's output, even when no reader appears in this particular DAG.
        for (index, (first, artifact_type)) in invalidators.iter().enumerate() {
            for (second, other_type) in invalidators.iter().skip(index + 1) {
                if artifact_type == other_type
                    && !is_causal_ancestor(first, second, ancestors)
                    && !is_causal_ancestor(second, first, ancestors)
                {
                    return Err(PlanError::UnorderedInvalidations {
                        first: first.clone(),
                        second: second.clone(),
                        artifact_type: artifact_type.clone(),
                    });
                }
            }
        }

        for (consumer, node) in by_id {
            let contract = catalog
                .contract(&node.operation)
                .ok_or_else(|| PlanError::UnknownOperation(node.operation.clone()))?;
            for (port, binding) in &node.inputs {
                let expected =
                    contract
                        .inputs
                        .get(port)
                        .ok_or_else(|| PlanError::UnexpectedPort {
                            instance: consumer.clone(),
                            port: port.clone(),
                            direction: "input",
                        })?;
                let (artifact, producer) = match binding {
                    ArtifactBinding::External { artifact } => (artifact, None),
                    ArtifactBinding::Produced {
                        artifact, producer, ..
                    } => (artifact, Some(producer)),
                };
                let source_lineage =
                    lineage
                        .get(artifact)
                        .ok_or_else(|| PlanError::ArtifactSchemaMismatch {
                            instance: consumer.clone(),
                            port: port.clone(),
                            artifact: artifact.clone(),
                        })?;
                // The declared direct input schema is already checked in the
                // earlier pass; this pass additionally checks ALL inherited
                // dependencies of the concrete artifact being consumed.
                debug_assert!(source_lineage.contains(&expected.id));
                for (invalidator, artifact_type) in &invalidators {
                    if !source_lineage.contains(artifact_type) || invalidator == consumer {
                        // A node may read the old value before invalidating
                        // it itself. Distinct nodes require explicit order.
                        continue;
                    }
                    if is_causal_ancestor(consumer, invalidator, ancestors) {
                        // The consumer completes BEFORE this invalidation.
                        continue;
                    }
                    if !is_causal_ancestor(invalidator, consumer, ancestors) {
                        return Err(PlanError::UnorderedArtifactHazard {
                            invalidator: invalidator.clone(),
                            consumer: consumer.clone(),
                            artifact_type: artifact_type.clone(),
                        });
                    }
                    if producer.is_some_and(|producer| {
                        (producer == invalidator && expected.id == *artifact_type)
                            || is_causal_ancestor(invalidator, producer, ancestors)
                    }) {
                        // An explicitly dependent producer of the exact type
                        // has refreshed this binding after the invalidation.
                        // An invalidator's own *different* output derived
                        // from the old type is not a certified refresh.
                        continue;
                    }
                    return Err(PlanError::StaleArtifact {
                        invalidator: invalidator.clone(),
                        consumer: consumer.clone(),
                        artifact: artifact.clone(),
                        artifact_type: artifact_type.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn is_causal_ancestor(
    earlier: &OperationInstanceId,
    later: &OperationInstanceId,
    ancestors: &BTreeMap<OperationInstanceId, BTreeSet<OperationInstanceId>>,
) -> bool {
    ancestors
        .get(later)
        .is_some_and(|prior| prior.contains(earlier))
}

fn verify_ports<V>(
    node: &OperationNode,
    expected: &BTreeMap<PortId, ArtifactSchema>,
    actual: &BTreeMap<PortId, V>,
    direction: &'static str,
) -> Result<(), PlanError> {
    for port in expected.keys() {
        if !actual.contains_key(port) {
            return Err(PlanError::MissingPort {
                instance: node.instance.clone(),
                port: port.clone(),
                direction,
            });
        }
    }
    for port in actual.keys() {
        if !expected.contains_key(port) {
            return Err(PlanError::UnexpectedPort {
                instance: node.instance.clone(),
                port: port.clone(),
                direction,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
