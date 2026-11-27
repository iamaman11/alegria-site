//! Read-only authoring surface for the existing typed DAG contract.
//!
//! This is not another planner, scheduler, provider connector or admission
//! authority. A recipe is a small, editable input to `operation_protocol`;
//! only the existing validator can seal and accept its exact graph.
//! Production execution still requires independent Temporal/owner admission.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    owner_content_hash, ArtifactBinding, ArtifactId, ArtifactSchema, ContractCatalog, EffectId,
    ExternalArtifact, IdempotencyPolicy, OperationInstanceId, OperationNode, PlanError, PlanHash,
    PortId, DAG_MAX_NODES, PLAN_SCHEMA_REVISION,
};
pub use operation_protocol::{ExecutionPlan, OperationDagReceiptV1, OperationRef, RunScope};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod evidence;
mod operation_inspection;
mod parameters_json;
mod readiness;
mod source_bridge;
pub use evidence::{
    preview_consumer_need_evidence, ConsumerNeedAssessmentV1, ConsumerNeedEvidenceBatchV1,
    ConsumerNeedEvidenceError, ConsumerNeedEvidencePointerV1, ConsumerNeedEvidencePreviewV1,
    ConsumerNeedEvidenceStatusV1, ConsumerNeedHypothesisV1, ConsumerNeedObservationV1,
    NeedEvidenceRepresentationV1, NeedEvidenceSourceV1, NEED_EVIDENCE_SCHEMA_REVISION_V1,
};
pub use operation_inspection::{
    inspect_operation, CheckedOperationParameters, OperationInspection,
};
pub use parameters_json::{
    installed_semantic_lab_catalog, NoParameters, ParameterDescription, ParameterError,
    RecipeParameters, SiteLabCatalog,
};
pub use readiness::{
    summarize_local_execution_readiness, OperationExecutionReadinessV1, OperationGateV1,
    OperationReadinessSummaryV1,
    OPERATION_EXECUTION_READINESS_SCHEMA_REVISION_V1,
};
pub use source_bridge::{
    bind_public_page_findings_v1, preview_serp_fetch_candidates_v1, PublicPageEvidenceInputV1,
    PublicPageFindingV1, ResearchSourceBridgeErrorV1, SerpFetchCandidateStatusV1,
    SerpFetchCandidateV1, SerpFetchPolicyV1,
};

pub const SITE_RECIPE_SCHEMA_REVISION: u16 = 2;
const LEGACY_SITE_RECIPE_SCHEMA_REVISION: u16 = 1;

/// An external artifact is only *declared* here. A string provenance label is
/// never evidence of an authenticated Truth/source revision or permission to
/// perform an effect. The authoritative runtime must verify those separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeSource {
    pub schema: ArtifactSchema,
    pub provenance: String,
}

/// Human-authored references. Output identities and contract fingerprints
/// are derived from the trusted catalog rather than copied from the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeInput {
    External {
        artifact: ArtifactId,
    },
    Produced {
        step: OperationInstanceId,
        port: PortId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeStep {
    pub id: OperationInstanceId,
    pub operation: OperationRef,
    pub inputs: BTreeMap<PortId, RecipeInput>,
    /// Versioned owner parameters; validated against a trusted Rust DTO or
    /// explicitly deferred to the independent owner ValidateOnly preflight.
    #[serde(default)]
    pub parameters: RecipeParameters,
    #[serde(default)]
    pub after: BTreeSet<OperationInstanceId>,
}

/// No provider names, Task Queues, credentials or CLI commands belong in a
/// recipe. Changing its step order/parameters never requires a Cargo build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteRecipe {
    pub schema_revision: u16,
    pub scope: RunScope,
    #[serde(default)]
    pub sources: BTreeMap<ArtifactId, RecipeSource>,
    pub steps: Vec<RecipeStep>,
}

/// Small, stateless edits of a recipe. No step or provider is executable by
/// editing it; the returned draft must still pass the existing DAG validator.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeChange {
    AddStep {
        step: RecipeStep,
    },
    RemoveStep {
        step: OperationInstanceId,
    },
    SetOperation {
        step: OperationInstanceId,
        operation: OperationRef,
    },
    SetParameters {
        step: OperationInstanceId,
        parameters: RecipeParameters,
    },
    SetInput {
        step: OperationInstanceId,
        port: PortId,
        input: RecipeInput,
    },
    RemoveInput {
        step: OperationInstanceId,
        port: PortId,
    },
    SetAfter {
        step: OperationInstanceId,
        after: BTreeSet<OperationInstanceId>,
    },
    UpsertSource {
        artifact: ArtifactId,
        source: RecipeSource,
    },
    RemoveSource {
        artifact: ArtifactId,
    },
}

#[derive(Debug, Error)]
pub enum RecipeEditError {
    #[error("recipe edit batch is empty or exceeds 128 changes")]
    InvalidBatchSize,
    #[error("step {0:?} already exists")]
    DuplicateStep(OperationInstanceId),
    #[error("step {0:?} does not exist")]
    UnknownStep(OperationInstanceId),
    #[error("cannot add more than {DAG_MAX_NODES} steps")]
    TooManySteps,
}

/// Apply a batch to a clone, so a rejected change never partially mutates a
/// caller's recipe. Removing a referenced step does not silently rewire its
/// dependents; a subsequent `preview_recipe` reports the broken binding.
pub fn revise_recipe(
    recipe: &SiteRecipe,
    changes: &[RecipeChange],
) -> Result<SiteRecipe, RecipeEditError> {
    if changes.is_empty() || changes.len() > 128 {
        return Err(RecipeEditError::InvalidBatchSize);
    }
    let mut draft = recipe.clone();
    for change in changes {
        match change {
            RecipeChange::AddStep { step } => {
                if draft.steps.iter().any(|existing| existing.id == step.id) {
                    return Err(RecipeEditError::DuplicateStep(step.id.clone()));
                }
                if draft.steps.len() >= DAG_MAX_NODES {
                    return Err(RecipeEditError::TooManySteps);
                }
                draft.steps.push(step.clone());
            }
            RecipeChange::UpsertSource { artifact, source } => {
                draft.sources.insert(artifact.clone(), source.clone());
            }
            RecipeChange::RemoveSource { artifact } => {
                draft.sources.remove(artifact);
            }
            RecipeChange::RemoveStep { step } => {
                let position = draft
                    .steps
                    .iter()
                    .position(|existing| &existing.id == step)
                    .ok_or_else(|| RecipeEditError::UnknownStep(step.clone()))?;
                draft.steps.remove(position);
            }
            RecipeChange::SetOperation { step, operation } => {
                find_step(&mut draft, step)?.operation = operation.clone();
            }
            RecipeChange::SetParameters { step, parameters } => {
                find_step(&mut draft, step)?.parameters = parameters.clone();
            }
            RecipeChange::SetInput { step, port, input } => {
                find_step(&mut draft, step)?
                    .inputs
                    .insert(port.clone(), input.clone());
            }
            RecipeChange::RemoveInput { step, port } => {
                find_step(&mut draft, step)?.inputs.remove(port);
            }
            RecipeChange::SetAfter { step, after } => {
                find_step(&mut draft, step)?.after = after.clone();
            }
        }
    }
    Ok(draft)
}

fn find_step<'a>(
    draft: &'a mut SiteRecipe,
    step: &OperationInstanceId,
) -> Result<&'a mut RecipeStep, RecipeEditError> {
    draft
        .steps
        .iter_mut()
        .find(|existing| &existing.id == step)
        .ok_or_else(|| RecipeEditError::UnknownStep(step.clone()))
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescription {
    pub operation: OperationRef,
    pub owner: String,
    pub inputs: BTreeMap<PortId, ArtifactSchema>,
    pub outputs: BTreeMap<PortId, ArtifactSchema>,
    pub invalidates: BTreeSet<operation_protocol::ArtifactTypeId>,
    pub effects: BTreeSet<EffectId>,
    pub idempotency: IdempotencyPolicy,
    pub contract_hash: String,
    pub parameters: Option<ParameterDescription>,
    /// An operation's contract being registered does not grant operator
    /// admission or mean a provider is online.
    pub readiness: &'static str,
    /// Typed per-operation launch gates. A local catalog cannot prove a
    /// remote user, one-use grant, owner History or provider usage receipt.
    pub execution_readiness: OperationExecutionReadinessV1,
}

/// Every registered owner contract is discoverable without adding a new
/// provider-specific case to the CLI or the DAG kernel. The caller must pass
/// a *trusted* catalog; never register contracts from a user recipe.
pub fn describe_tools(catalog: &SiteLabCatalog) -> Result<Vec<ToolDescription>, SiteLabError> {
    catalog
        .catalog()
        .contracts()
        .map(|contract| {
            let parameters = catalog.describe(&contract.reference);
            let missing_schema = parameters.is_none();
            Ok(ToolDescription {
                operation: contract.reference.clone(),
                owner: contract.owner.as_str().to_owned(),
                inputs: contract.inputs.clone(),
                outputs: contract.outputs.clone(),
                invalidates: contract.invalidates.clone(),
                effects: contract.effects.clone(),
                idempotency: contract.idempotency,
                contract_hash: contract.fingerprint()?.as_str().to_owned(),
                execution_readiness:
                    readiness::local_catalog_execution_readiness(contract, parameters.as_ref()),
                parameters,
                readiness: if missing_schema {
                    "missing_parameter_schema"
                } else if contract.effects.is_empty()
                    && contract.idempotency == IdempotencyPolicy::Pure
                {
                    "pure_plan_only"
                } else {
                    "requires_owner_admission"
                },
            })
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum SiteLabError {
    #[error(
        "unsupported recipe revision {0}; supported revisions are 1 (without parameters) and 2"
    )]
    UnsupportedRevision(u16),
    #[error("recipe must include at least one operation")]
    EmptyRecipe,
    #[error("recipe contains {0} steps; DAG node limit is {DAG_MAX_NODES}")]
    TooManySteps(usize),
    #[error("operation {0:?} is not registered by an installed owner")]
    UnavailableOperation(OperationRef),
    #[error("step {step:?} ({operation:?}) has invalid or unavailable typed parameters: {source}")]
    StepParameters {
        step: OperationInstanceId,
        operation: OperationRef,
        #[source]
        source: ParameterError,
    },
    #[error(transparent)]
    InvalidParameters(#[from] ParameterError),
    #[error("recipe parameters cannot be encoded")]
    ParameterEncoding,
    #[error(transparent)]
    InvalidPlan(#[from] PlanError),
}

#[derive(Debug, Serialize)]
pub struct SiteLabProblem {
    pub status: &'static str,
    pub code: &'static str,
    pub message: String,
    pub next_action: &'static str,
    /// Identifies the exact recipe step without copying parameter values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<OperationInstanceId>,
}

impl SiteLabError {
    pub fn problem(&self) -> SiteLabProblem {
        let (code, next_action) = match self {
            Self::UnsupportedRevision(_) => (
                "recipe_revision_unsupported",
                "Use the installed recipe schema; do not silently upgrade an existing recipe.",
            ),
            Self::EmptyRecipe => (
                "recipe_empty",
                "Select at least one registered operation from the catalog.",
            ),
            Self::TooManySteps(_) => (
                "recipe_too_large",
                "Reduce the number of steps or split the research into separate runs.",
            ),
            Self::UnavailableOperation(_) => (
                "operation_unavailable",
                "Inspect the catalog and install an independently authorized owner adapter.",
            ),
            Self::StepParameters {
                source: ParameterError::MissingSchema,
                ..
            } => (
                "operation_parameter_schema_missing",
                "Install this owner's trusted parameter contract; recipe JSON cannot supply it.",
            ),
            Self::StepParameters {
                source: ParameterError::MismatchedContract,
                ..
            } => (
                "operation_parameter_contract_mismatch",
                "Refresh the trusted operation catalog; do not reinterpret a stale parameter schema.",
            ),
            Self::StepParameters { .. } => (
                "operation_parameters_invalid",
                "Correct this step using the installed operation's exact typed parameter schema.",
            ),
            Self::InvalidParameters(ParameterError::MissingSchema) => (
                "operation_parameter_schema_missing",
                "Install this owner's trusted parameter contract; recipe JSON cannot supply it.",
            ),
            Self::InvalidParameters(ParameterError::MismatchedContract) => (
                "operation_parameter_contract_mismatch",
                "Refresh the trusted operation catalog; do not reinterpret a stale parameter schema.",
            ),
            Self::InvalidParameters(_) | Self::ParameterEncoding => (
                "operation_parameters_invalid",
                "Use the installed operation's exact typed parameter schema; remote owners require ValidateOnly before effects.",
            ),
            Self::InvalidPlan(PlanError::Cycle) => (
                "recipe_cycle",
                "Remove a circular dependency between steps.",
            ),
            Self::InvalidPlan(PlanError::ArtifactSchemaMismatch { .. }) => (
                "incompatible_ports",
                "Connect an output and input with the same versioned artifact schema.",
            ),
            Self::InvalidPlan(PlanError::MissingExternalArtifact(_)) => (
                "missing_source",
                "Declare the referenced source with its schema and provenance.",
            ),
            Self::InvalidPlan(PlanError::MissingProducedArtifact { .. }) => (
                "missing_output",
                "Choose a producer step and one of its declared output ports.",
            ),
            Self::InvalidPlan(PlanError::MissingProvenance(_)) => (
                "missing_provenance",
                "Provide source provenance; runtime admission must verify it independently.",
            ),
            Self::InvalidPlan(PlanError::UnknownInstance(_)) => {
                ("unknown_step", "Select a step that exists in this recipe.")
            }
            Self::InvalidPlan(_) => (
                "invalid_recipe",
                "Inspect the exact offending step, port or contract and correct the recipe.",
            ),
        };
        SiteLabProblem {
            status: "invalid",
            code,
            message: self.to_string(),
            next_action,
            step: match self {
                Self::StepParameters { step, .. } => Some(step.clone()),
                _ => None,
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RecipePreview {
    pub status: &'static str,
    /// The full authoring identity changes when typed parameters change even
    /// if the generic ExecutionPlan graph and plan_hash remain unchanged.
    pub recipe_hash: String,
    pub plan_hash: PlanHash,
    pub ordered_steps: Vec<OperationInstanceId>,
    pub source_count: usize,
    pub parameter_hashes: BTreeMap<OperationInstanceId, String>,
    pub steps_requiring_owner_parameter_validation: Vec<OperationInstanceId>,
    /// These steps can be designed and checked, but never run merely because
    /// their contracts are present in the catalog or a preview was accepted.
    pub steps_requiring_owner_admission: Vec<OperationInstanceId>,
    /// This is never a production provenance, permission or execution proof.
    pub source_authentication: &'static str,
    pub execution: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RecipeDifference {
    pub status: &'static str,
    pub first: RecipePreview,
    pub second: RecipePreview,
    pub same_plan: bool,
    pub same_recipe: bool,
    /// Same *declarations*, not necessarily the same fetched source bytes.
    pub same_declared_sources: bool,
    pub note: &'static str,
}

/// A model-friendly connection preview from the *real installed* contracts.
/// Schema-compatible options are suggestions, not authorization or a sealed
/// DAG. The normal validator still checks effects, invalidations and scopes.
#[derive(Debug, Serialize)]
pub struct RecipeConnectionPreview {
    pub status: &'static str,
    pub ports: Vec<RecipePortConnections>,
    pub execution: &'static str,
    pub note: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RecipePortConnections {
    pub step: OperationInstanceId,
    pub port: PortId,
    pub required_schema: ArtifactSchema,
    pub current: Option<RecipeInput>,
    pub candidates: Vec<RecipeInput>,
}

/// Is `dependent` already causally downstream from `ancestor`? The new
/// Produced edge ancestor <- dependent would then introduce a cycle.
fn depends_on_step(
    recipe: &SiteRecipe,
    dependent: &OperationInstanceId,
    ancestor: &OperationInstanceId,
    visited: &mut BTreeSet<OperationInstanceId>,
) -> bool {
    if dependent == ancestor {
        return true;
    }
    if !visited.insert(dependent.clone()) {
        return false;
    }
    let Some(step) = recipe.steps.iter().find(|step| &step.id == dependent) else {
        return false;
    };
    step.after
        .iter()
        .any(|prerequisite| depends_on_step(recipe, prerequisite, ancestor, visited))
        || step.inputs.values().any(|input| match input {
            RecipeInput::Produced {
                step: prerequisite, ..
            } => depends_on_step(recipe, prerequisite, ancestor, visited),
            RecipeInput::External { .. } => false,
        })
}

/// Suggest candidate bindings even for an incomplete recipe, so ChatGPT can
/// construct the graph incrementally without inventing ports or DTO types.
/// The existing `preview_recipe` remains the single authoritative check.
pub fn suggest_compatible_inputs(
    recipe: &SiteRecipe,
    catalog: &SiteLabCatalog,
) -> Result<RecipeConnectionPreview, SiteLabError> {
    if recipe.schema_revision != SITE_RECIPE_SCHEMA_REVISION {
        return Err(SiteLabError::UnsupportedRevision(recipe.schema_revision));
    }
    if recipe.steps.len() > DAG_MAX_NODES {
        return Err(SiteLabError::TooManySteps(recipe.steps.len()));
    }
    let contracts = catalog.catalog();
    let mut steps: Vec<_> = recipe.steps.iter().collect();
    steps.sort_by(|first, second| first.id.cmp(&second.id));
    let mut ports = Vec::new();
    for step in &steps {
        let contract = contracts
            .contract(&step.operation)
            .ok_or_else(|| SiteLabError::UnavailableOperation(step.operation.clone()))?;
        for (port, schema) in &contract.inputs {
            let mut candidates = Vec::new();
            for (artifact, source) in &recipe.sources {
                if &source.schema == schema {
                    candidates.push(RecipeInput::External {
                        artifact: artifact.clone(),
                    });
                }
            }
            for producer in &steps {
                let producer_contract =
                    contracts.contract(&producer.operation).ok_or_else(|| {
                        SiteLabError::UnavailableOperation(producer.operation.clone())
                    })?;
                if depends_on_step(recipe, &producer.id, &step.id, &mut BTreeSet::new()) {
                    continue;
                }
                for (output_port, output_schema) in &producer_contract.outputs {
                    if output_schema == schema {
                        candidates.push(RecipeInput::Produced {
                            step: producer.id.clone(),
                            port: output_port.clone(),
                        });
                    }
                }
            }
            ports.push(RecipePortConnections {
                step: step.id.clone(),
                port: port.clone(),
                required_schema: schema.clone(),
                current: step.inputs.get(port).cloned(),
                candidates,
            });
        }
    }
    Ok(RecipeConnectionPreview {
        status: "schema_candidates_only",
        ports,
        execution: "not_started",
        note: "Exact versioned port schemas match and immediate dependency cycles are excluded; owner admission, provenance and full V2 DAG validation remain independent.",
    })
}

fn output_artifact_id(step: &OperationInstanceId, port: &PortId) -> ArtifactId {
    ArtifactId(format!("generated/{}/{}", step.as_str(), port.as_str()))
}

/// Sealing is allowed only while authoring this new draft. A persisted,
/// server-issued plan must instead be validated with its original pins/hash.
pub fn compile_recipe(
    recipe: &SiteRecipe,
    catalog: &ContractCatalog,
) -> Result<ExecutionPlan, SiteLabError> {
    if recipe.schema_revision != SITE_RECIPE_SCHEMA_REVISION
        && recipe.schema_revision != LEGACY_SITE_RECIPE_SCHEMA_REVISION
    {
        return Err(SiteLabError::UnsupportedRevision(recipe.schema_revision));
    }
    if recipe.schema_revision == LEGACY_SITE_RECIPE_SCHEMA_REVISION
        && recipe.steps.iter().any(|step| !step.parameters.is_empty())
    {
        return Err(SiteLabError::UnsupportedRevision(recipe.schema_revision));
    }
    if recipe.steps.is_empty() {
        return Err(SiteLabError::EmptyRecipe);
    }
    if recipe.steps.len() > DAG_MAX_NODES {
        return Err(SiteLabError::TooManySteps(recipe.steps.len()));
    }

    let mut nodes = Vec::with_capacity(recipe.steps.len());
    let mut declared_effects = BTreeSet::new();
    for step in &recipe.steps {
        let contract = catalog
            .contract(&step.operation)
            .ok_or_else(|| SiteLabError::UnavailableOperation(step.operation.clone()))?;
        // This only constructs a manifest. Even a valid effect declaration
        // cannot grant owner admission, a signed operator route or execution.
        declared_effects.extend(contract.effects.iter().cloned());
        let outputs = contract
            .outputs
            .keys()
            .map(|port| (port.clone(), output_artifact_id(&step.id, port)))
            .collect();
        let inputs = step
            .inputs
            .iter()
            .map(|(port, input)| {
                let binding = match input {
                    RecipeInput::External { artifact } => ArtifactBinding::External {
                        artifact: artifact.clone(),
                    },
                    RecipeInput::Produced {
                        step: producer,
                        port: output_port,
                    } => ArtifactBinding::Produced {
                        producer: producer.clone(),
                        port: output_port.clone(),
                        artifact: output_artifact_id(producer, output_port),
                    },
                };
                (port.clone(), binding)
            })
            .collect();
        nodes.push(OperationNode {
            instance: step.id.clone(),
            operation: step.operation.clone(),
            inputs,
            outputs,
            after: step.after.clone(),
            permitted_effects: contract.effects.clone(),
        });
    }
    let sources = recipe
        .sources
        .iter()
        .map(|(id, source)| {
            (
                id.clone(),
                ExternalArtifact {
                    schema: source.schema.clone(),
                    scope: recipe.scope.clone(),
                    provenance: source.provenance.clone(),
                },
            )
        })
        .collect();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: recipe.scope.clone(),
        allowed_effects: declared_effects,
        external_artifacts: sources,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(catalog)?;
    plan.validate(catalog)?;
    Ok(plan)
}

pub fn preview_recipe(
    recipe: &SiteRecipe,
    catalog: &SiteLabCatalog,
) -> Result<RecipePreview, SiteLabError> {
    let plan = compile_recipe(recipe, catalog.catalog())?;
    let checked = plan.validate(catalog.catalog())?;
    let mut parameter_hashes = BTreeMap::new();
    let mut steps_requiring_owner_parameter_validation = Vec::new();
    for step in &recipe.steps {
        let checked = catalog
            .validate(&step.operation, &step.parameters)
            .map_err(|source| SiteLabError::StepParameters {
                step: step.id.clone(),
                operation: step.operation.clone(),
                source,
            })?;
        if checked.verification == "requires_owner_validate_only" {
            steps_requiring_owner_parameter_validation.push(step.id.clone());
        }
        parameter_hashes.insert(step.id.clone(), checked.digest);
    }
    // Use the sealed plan's order-independent identity plus versioned owner
    // parameter digests. A reordering of independent steps or a harmless v1
    // -> v2 migration must not create a fictitious new experiment, while a
    // parameter schema change must change the experiment identity.
    let recipe_hash = owner_content_hash(
        &serde_json::to_vec(&(&plan.hash, &parameter_hashes))
            .map_err(|_| SiteLabError::ParameterEncoding)?,
    );
    let steps_requiring_owner_admission = plan
        .nodes
        .iter()
        .filter(|node| {
            catalog
                .catalog()
                .contract(&node.operation)
                .is_some_and(|contract| {
                    !contract.effects.is_empty() || contract.idempotency != IdempotencyPolicy::Pure
                })
        })
        .map(|node| node.instance.clone())
        .collect();
    Ok(RecipePreview {
        status: if steps_requiring_owner_parameter_validation.is_empty() {
            "valid_preview_only"
        } else {
            "pending_owner_parameter_validation"
        },
        recipe_hash,
        plan_hash: plan.hash,
        ordered_steps: checked.execution_order().to_vec(),
        source_count: recipe.sources.len(),
        parameter_hashes,
        steps_requiring_owner_parameter_validation,
        steps_requiring_owner_admission,
        source_authentication: "not_verified",
        execution: "not_started",
    })
}

pub fn diff_recipes(
    first: &SiteRecipe,
    second: &SiteRecipe,
    catalog: &SiteLabCatalog,
) -> Result<RecipeDifference, SiteLabError> {
    let first_preview = preview_recipe(first, catalog)?;
    let second_preview = preview_recipe(second, catalog)?;
    Ok(RecipeDifference {
        status: "structural_comparison_only",
        same_plan: first_preview.plan_hash == second_preview.plan_hash,
        same_recipe: first_preview.recipe_hash == second_preview.recipe_hash,
        same_declared_sources: first.scope.tenant_id == second.scope.tenant_id
            && first.sources == second.sources,
        first: first_preview,
        second: second_preview,
        note: "Only recipes, pinned parameter bytes and declared sources were compared; provider data, execution and SEO outcomes were not measured.",
    })
}

#[cfg(test)]
mod tests;

mod receipts;
pub use receipts::{
    compare_executions, inspect_execution, ExecutionComparison, ExecutionInspection,
    InspectionError,
};

#[cfg(test)]
mod receipts_tests;
