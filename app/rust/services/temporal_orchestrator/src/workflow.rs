//! Standalone, provider-free native Temporal operation Workflows.
//!
//! Generic history-visible DAG types operate on `operation_protocol` only.
//! Historical site-specific types are optional and retain their original
//! `seo_application` execution semantics for replay compatibility.
//! This process cannot execute Activities, access provider SDKs or publish.

#[cfg(feature = "legacy-workflows")]
use operation_protocol::EffectChildInvocationV4;
use operation_protocol::{
    owner_child_workflow_id, owner_child_workflow_id_v2, owner_validation_child_workflow_id_v1,
    OperationDagMachine, OperationDagReceiptV1, OperationDagRunV6, OwnerInvocationV1,
    OwnerResultV1, OwnerValidationRequestV1, OwnerValidationResultV1, TemporalExecutionIdentityV4,
    TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1, OWNER_VALIDATION_CHILD_TYPE_V1,
};
#[cfg(feature = "legacy-qdrant-v1")]
use qdrant_contracts::{
    QdrantKeywordChildInvocationV1, QdrantKeywordChildResultV1, QdrantProjectionRunV1,
    QDRANT_OWNER_CHILD_TYPE_V1, QDRANT_OWNER_COMMAND_QUEUE_V1,
};
#[cfg(feature = "legacy-workflows")]
use seo_application::operation_adapter::{
    decode_owner_child_result_v4, execute_checked_dag_v1, execute_pure_dag_legacy_v3,
    EffectDagRunV1, MixedDagMachineV4, MixedDagMachineV5, NativeEffectRouteV5, PreparedDagStepV1,
};
#[cfg(feature = "legacy-workflows")]
use seo_application::operation_adapter::{execute_pure_dag, PureDagReceipt, PureDagRun};
#[cfg(feature = "legacy-workflows")]
use operation_protocol::{OperationRef, PinnedOwnerOperationSchemaV3, DAG_MAX_INPUT_BYTES};
#[cfg(feature = "legacy-workflows")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "legacy-workflows")]
use std::collections::BTreeSet;
use std::sync::OnceLock;
use temporalio_common::WorkflowDefinition;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::{
    ApplicationFailure, ChildWorkflowOptions, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult, WorkflowTermination,
};

#[cfg(feature = "legacy-workflows")]
pub(crate) const PG_V4_CHILD_WORKFLOW_TYPE: &str = "PgTruthReviewReconcileV4Workflow";
#[cfg(feature = "legacy-workflows")]
pub(crate) const PG_V4_CHILD_COMMAND_QUEUE: &str = "alegria-pg-command-v3";
static TRUSTED_OWNER_CATALOG: OnceLock<TrustedRouteCatalog> = OnceLock::new();

pub(crate) fn install_owner_catalog(catalog: TrustedRouteCatalog) -> anyhow::Result<()> {
    TRUSTED_OWNER_CATALOG
        .set(catalog)
        .map_err(|_| anyhow::anyhow!("trusted owner route catalog already installed"))
}

/// A typed client-side Workflow *definition*, not a handler/Activity. Only
/// the separate PG owner service registers and implements this Workflow.
/// Temporal persists the complete typed input in parent and child Histories.
#[cfg(feature = "legacy-workflows")]
#[derive(Clone, Copy)]
struct PgTruthReviewReconcileV4Definition;

#[cfg(feature = "legacy-workflows")]
impl WorkflowDefinition for PgTruthReviewReconcileV4Definition {
    type Input = EffectChildInvocationV4;
    type Output = String;

    fn name(&self) -> &str {
        PG_V4_CHILD_WORKFLOW_TYPE
    }
}

/// No Qdrant SDK, DB connection or Activity handler can enter this pure
/// process. The separately registered child on a dedicated Task Queue owns
/// the provider RPC and cannot execute before operator admission.
#[cfg(feature = "legacy-qdrant-v1")]
#[derive(Clone, Copy)]
struct QdrantKeywordProjectionV1Definition;

#[cfg(feature = "legacy-qdrant-v1")]
impl WorkflowDefinition for QdrantKeywordProjectionV1Definition {
    type Input = QdrantKeywordChildInvocationV1;
    type Output = String;

    fn name(&self) -> &str {
        QDRANT_OWNER_CHILD_TYPE_V1
    }
}

/// ONE shared, statically typed native SDK boundary for all independent owner
/// workers. Their Task Queues and trust domains remain separate; each owner
/// validates its concrete Rust input before a local Activity may have effects.
#[derive(Clone, Copy)]
struct OwnerOperationV1Definition;

impl WorkflowDefinition for OwnerOperationV1Definition {
    type Input = OwnerInvocationV1;
    type Output = OwnerResultV1;

    fn name(&self) -> &str {
        OWNER_OPERATION_CHILD_TYPE_V1
    }
}

/// Owner-specific ValidateOnly Child. It executes on the same independent
/// owner queue but has NO Activity handler or external effects. The owner
/// itself must parse its concrete domain command and available External data.
#[derive(Clone, Copy)]
struct OwnerValidationV1Definition;

impl WorkflowDefinition for OwnerValidationV1Definition {
    type Input = OwnerValidationRequestV1;
    type Output = OwnerValidationResultV1;

    fn name(&self) -> &str {
        OWNER_VALIDATION_CHILD_TYPE_V1
    }
}

fn workflow_failure(error: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(error.to_string()),
    ))
}

#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaPureOperationDagV1Workflow {
    immutable_input: PureDagRun,
}

/// A separate History-visible strict input contract. Never reinterpret an
/// existing V3 History with new duplicate/unknown-field rejection semantics.
/// This V4 type runs only on its own queue and returns a typed receipt rather
/// than the historical V3 JSON String.
#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaPureOperationDagV2Workflow {
    immutable_input: PureDagRun,
}

/// V5 is an additive, separately queued Temporal contract. Its complete
/// owner-generated structural DTO pins are recorded in the immutable Workflow
/// start event. They are never authoritative merely because a caller supplied
/// them: the worker compares every referenced operation with actual owner
/// schemas generated by its linked semantic domain code before execution.
/// The old V3/V4 Workflow histories and their wire inputs stay untouched.
#[cfg(feature = "legacy-workflows")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PureDagRunSchemaPinnedV5 {
    pub schema_revision: u16,
    pub run: PureDagRun,
    /// Sorted, unique pins. `OperationRef` is a struct, so a JSON map keyed
    /// by it would be invalid as a Temporal payload: use a canonical list.
    pub owner_schemas: Vec<PinnedOwnerOperationSchemaV3>,
}

#[cfg(feature = "legacy-workflows")]
pub(crate) const PURE_DAG_SCHEMA_PIN_REVISION_V5: u16 = 5;

#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaPureOperationDagV3Workflow {
    immutable_input: PureDagRunSchemaPinnedV5,
}

/// Fail closed before a single pure operation runs. Schema bytes, owner,
/// artifact identities and the V2 contract fingerprint are all checked
/// against the owner-linked implementation, never against a manifest supplied
/// by a caller. Successful validation does NOT grant external effects.
#[cfg(feature = "legacy-workflows")]
fn verify_owner_generated_pins_v5(input: &PureDagRunSchemaPinnedV5) -> anyhow::Result<()> {
    anyhow::ensure!(
        input.schema_revision == PURE_DAG_SCHEMA_PIN_REVISION_V5,
        "unsupported pure DAG structural schema pin revision"
    );
    let input_size = serde_json::to_vec(input)?.len();
    anyhow::ensure!(
        input_size <= DAG_MAX_INPUT_BYTES,
        "schema-pinned pure DAG exceeds Temporal input budget"
    );
    let required: BTreeSet<OperationRef> = input
        .run
        .plan
        .nodes
        .iter()
        .map(|node| node.operation.clone())
        .collect();
    anyhow::ensure!(!required.is_empty(), "schema-pinned pure DAG requires nodes");
    anyhow::ensure!(
        input.owner_schemas.len() == required.len(),
        "pure DAG structural pins must cover exactly the referenced operations"
    );
    let installed = seo_application::operation_adapter::semantic_extraction_registry()?;
    let generated = seo_application::operation_adapter::semantic_extraction_owner_schemas()?;
    let mut previous: Option<&OperationRef> = None;
    for pin in &input.owner_schemas {
        let reference = &pin.schema.operation;
        anyhow::ensure!(
            previous.is_none_or(|last| last < reference),
            "pure DAG structural pins must be strictly sorted and unique"
        );
        previous = Some(reference);
        anyhow::ensure!(
            required.contains(reference),
            "pure DAG structural pins include an unreferenced operation"
        );
        let expected = generated
            .get(reference)
            .ok_or_else(|| anyhow::anyhow!("pure DAG has no owner-generated structural schema"))?;
        let contract = installed
            .catalog()
            .contract(reference)
            .ok_or_else(|| anyhow::anyhow!("pure DAG operation lacks a trusted owner contract"))?;
        pin.verify_against_trusted_owner(expected, contract)?;
    }
    Ok(())
}

/// The manifest may contain real PostgreSQL effects, but the current V3
/// admission schema cannot bind independently attested child Workflow Run
/// IDs to one canonical pipeline run. This Workflow intentionally performs
/// complete typed preflight and then fails non-retryably BEFORE any Activity
/// or child Workflow. It cannot emit a fabricated PG receipt.
#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaCheckedEffectDagV1Workflow {
    immutable_input: EffectDagRunV1,
}

/// New history-visible type: never reinterpret V1 preflight-only histories
/// as authorized effects. This V2 parent can only invoke typed PG child
/// Workflows, which wait for independent operator-issued V4 admission before
/// their distinct owner Activity can perform any PostgreSQL business write.
#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaAdmittedEffectDagV2Workflow {
    immutable_input: EffectDagRunV1,
}

/// New History-visible V5 parent. It sequences independently admissible PG
/// owner child Workflows in deterministic topological order. The grouped
/// start/join boundary is reserved for genuinely disjoint future owner
/// routes; today's single canonical PG Truth context always serializes.
/// Old V2 histories are never upgraded in place.
#[cfg(feature = "legacy-workflows")]
#[workflow]
struct AlegriaAdmittedEffectDagV3Workflow {
    immutable_input: EffectDagRunV1,
}

/// V7 new History-visible typed serial owner route. Deliberately not a
/// reinterpretation of prior V5 DAG Histories or arbitrary PG/Qdrant fanout.
/// All Qdrant effects must have independent Temporal parent/child proof.
#[cfg(feature = "legacy-qdrant-v1")]
#[workflow]
struct AlegriaQdrantProjectionDagV1Workflow {
    immutable_input: QdrantProjectionRunV1,
}

/// Stable generic History-visible DAG parent. No provider-specific operation,
/// queue, SDK, command DTO or special case can be added to this Workflow.
/// Only the configured trusted catalog may select a statically typed owner
/// Child Workflow on its separately credentialed Task Queue.
#[workflow]
struct AlegriaOperationDagV1Workflow {
    immutable_input: OperationDagRunV6,
}

/// A NEW history-visible parent: unlike frozen serial V1, it may start a
/// bounded wave of independent children before awaiting their typed results.
/// Effectful children remain blocked until all owner-local ValidateOnly
/// children complete; independent operators must additionally attest V2
/// Histories and issue exact owner SQL permissions before any Activity effect.
#[workflow]
struct AlegriaOperationDagV2Workflow {
    immutable_input: OperationDagRunV6,
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    opts.register_workflow::<AlegriaCheckedEffectDagV1Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_for_replay(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    opts.register_workflow::<AlegriaCheckedEffectDagV1Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_pure_v4(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV2Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_for_replay_pure_v4(
    opts: &mut WorkflowReplayerOptions,
) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV2Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_pure_v5(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV3Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_for_replay_pure_v5(
    opts: &mut WorkflowReplayerOptions,
) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV3Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_v4(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV2Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_for_replay_v4(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV2Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_v5(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV3Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-workflows")]
pub(crate) fn register_for_replay_v5(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV3Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-qdrant-v1")]
pub(crate) fn register_qdrant_v1(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaQdrantProjectionDagV1Workflow>()?;
    Ok(())
}

#[cfg(feature = "legacy-qdrant-v1")]
pub(crate) fn register_for_replay_qdrant_v1(
    opts: &mut WorkflowReplayerOptions,
) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaQdrantProjectionDagV1Workflow>()?;
    Ok(())
}

pub(crate) fn register_operation_dag_v1(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaOperationDagV1Workflow>()?;
    Ok(())
}

pub(crate) fn register_for_replay_operation_dag_v1(
    opts: &mut WorkflowReplayerOptions,
) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaOperationDagV1Workflow>()?;
    Ok(())
}

pub(crate) fn register_operation_dag_v2(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaOperationDagV2Workflow>()?;
    Ok(())
}

pub(crate) fn register_for_replay_operation_dag_v2(
    opts: &mut WorkflowReplayerOptions,
) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaOperationDagV2Workflow>()?;
    Ok(())
}

#[workflow_methods]
impl AlegriaOperationDagV2Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: OperationDagRunV6) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OperationDagReceiptV1> {
        let trusted_routes = TRUSTED_OWNER_CATALOG
            .get()
            .ok_or_else(|| workflow_failure("missing trusted owner route catalog"))?
            .clone();
        let run = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        let parent = TemporalExecutionIdentityV4 {
            workflow_id: workflow_id.clone(),
            run_id: ctx.run_id().to_string(),
        };
        // Whole-plan structural preflight occurs BEFORE any child. The exact
        // versioned owner-local business DTO validation below is a further
        // required gate before the first potentially irreversible owner.
        let mut machine =
            OperationDagMachine::new_v2_with_owner_validation(&workflow_id, run, trusted_routes)
                .map_err(workflow_failure)?;
        let validations = machine
            .v2_owner_validation_requests(parent.clone())
            .map_err(workflow_failure)?;
        let mut pending_validation_children = Vec::with_capacity(validations.len());
        for (route, request) in validations {
            if route.workflow_type != OWNER_OPERATION_CHILD_TYPE_V1 {
                return Err(workflow_failure("untrusted owner validation Task Queue"));
            }
            let options = ChildWorkflowOptions::builder()
                .workflow_id(owner_validation_child_workflow_id_v1(
                    &parent,
                    &request.plan_hash,
                    &request.instance,
                ))
                .task_queue(route.task_queue)
                .build();
            let child = ctx
                .start_child_workflow(OwnerValidationV1Definition, request, options)
                .await
                .map_err(workflow_failure)?;
            pending_validation_children.push(child);
        }
        let mut validation_results = Vec::with_capacity(pending_validation_children.len());
        for child in pending_validation_children {
            let result: OwnerValidationResultV1 = child.result().await.map_err(workflow_failure)?;
            validation_results.push(result);
        }
        // Never schedule a single effect Child until every owner has returned
        // a typed, exact request-hash validation receipt. The owner operators
        // independently re-attest these SERVER completion Histories and may
        // still deny the concrete effect or an invalid later Produced input.
        machine
            .complete_v2_owner_validation(parent.clone(), &validation_results)
            .map_err(workflow_failure)?;
        while machine.next_instance().is_some() {
            let ready = machine
                .next_independent_owner_invocations(parent.clone())
                .map_err(workflow_failure)?;
            if ready.is_empty() {
                return Err(workflow_failure("parallel DAG has no ready owner child"));
            }
            // A deterministic fan-out: no provider SDK, credentials or
            // provider-specific operation enters this coordinator.
            let mut children = Vec::with_capacity(ready.len());
            for (route, invocation) in ready {
                if route.workflow_type != OWNER_OPERATION_CHILD_TYPE_V1 {
                    return Err(workflow_failure("untrusted owner Workflow Type"));
                }
                let options = ChildWorkflowOptions::builder()
                    .workflow_id(owner_child_workflow_id_v2(
                        &invocation.checked_dispatch.execution_key,
                    ))
                    .task_queue(route.task_queue)
                    .build();
                let started = ctx
                    .start_child_workflow(OwnerOperationV1Definition, invocation, options)
                    .await
                    .map_err(workflow_failure)?;
                children.push(started);
            }
            // A deterministic fan-in: every actual typed Child result is
            // validated and published in sealed plan order. Produced edges
            // become ready in a *later* wave, never in the producer's wave.
            for child in children {
                let actual: OwnerResultV1 = child.result().await.map_err(workflow_failure)?;
                machine
                    .advance_owner_result(actual)
                    .map_err(workflow_failure)?;
            }
        }
        machine.finish().map_err(workflow_failure)
    }
}

#[workflow_methods]
impl AlegriaOperationDagV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: OperationDagRunV6) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OperationDagReceiptV1> {
        // The runner trusts only its operator-installed snapshot, never the
        // `pinned_routes` claimed by a caller. A different snapshot during
        // replay fails before emitting any new Temporal commands.
        let trusted_routes = TRUSTED_OWNER_CATALOG
            .get()
            .ok_or_else(|| workflow_failure("missing trusted owner route catalog"))?
            .clone();
        let run = ctx.state(|state| state.immutable_input.clone());
        // `run.plan.scope.run_id` is the canonical BUSINESS run UUID. The
        // Temporal Workflow ID is a deterministic prefixed identity, while
        // Temporal Run ID below is independently issued by the server.
        // Never pass the business UUID as a fabricated Temporal Run ID.
        let workflow_id = ctx.info().workflow_id().to_string();
        let parent = TemporalExecutionIdentityV4 {
            workflow_id: workflow_id.clone(),
            run_id: ctx.run_id().to_string(),
        };
        // Complete static plan/contract/route/command preflight before the
        // first child. Later Produced data is bound from actual child output,
        // never guessed from static manifest claims.
        let mut machine = OperationDagMachine::new(&workflow_id, run, trusted_routes)
            .map_err(workflow_failure)?;
        while machine.next_instance().is_some() {
            let (route, invocation) = machine
                .next_owner_invocation(parent.clone())
                .map_err(workflow_failure)?;
            if route.workflow_type != OWNER_OPERATION_CHILD_TYPE_V1 {
                return Err(workflow_failure("untrusted dynamic owner Workflow Type"));
            }
            // Each owner independently registers the SAME typed Workflow Type
            // on its own queue. The checked dispatch key fences retries across
            // parent/child Histories and must be independently verified by the
            // owner operator before any side-effectful Activity runs.
            let child_id = owner_child_workflow_id(&invocation.checked_dispatch.execution_key);
            let options = ChildWorkflowOptions::builder()
                .workflow_id(child_id)
                .task_queue(route.task_queue)
                .build();
            let started = ctx
                .start_child_workflow(OwnerOperationV1Definition, invocation, options)
                .await
                .map_err(workflow_failure)?;
            // Native Temporal SDK Input/Output are typed owner-neutral
            // envelopes; the owner separately decodes concrete Rust DTOs.
            let result: OwnerResultV1 = started.result().await.map_err(workflow_failure)?;
            machine
                .advance_owner_result(result)
                .map_err(workflow_failure)?;
        }
        machine.finish().map_err(workflow_failure)
    }
}

#[cfg(feature = "legacy-qdrant-v1")]
#[workflow_methods]
impl AlegriaQdrantProjectionDagV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: QdrantProjectionRunV1) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let run = ctx.state(|state| state.immutable_input.clone());
        // This is a complete preflight of every future irreversible child.
        // An invalid later node cannot follow an earlier external mutation.
        run.preflight().map_err(workflow_failure)?;
        let parent = TemporalExecutionIdentityV4 {
            workflow_id: ctx.info().workflow_id().to_string(),
            run_id: ctx.run_id().to_string(),
        };
        let mut results = Vec::<QdrantKeywordChildResultV1>::new();
        for command in run.commands {
            let options = ChildWorkflowOptions::builder()
                .workflow_id(format!("alegria-qdrant-effect-v1-{}", command.admission_id))
                .task_queue(QDRANT_OWNER_COMMAND_QUEUE_V1.to_string())
                .build();
            let child = ctx
                .start_child_workflow(
                    QdrantKeywordProjectionV1Definition,
                    QdrantKeywordChildInvocationV1 {
                        schema_revision: 1,
                        parent: parent.clone(),
                        canonical_run_id: run.canonical_run_id,
                        command: command.clone(),
                    },
                    options,
                )
                .await
                .map_err(workflow_failure)?;
            // Strictly sequential: cannot start a second remote write until
            // the first independently admitted child has a durable receipt.
            let result: String = child.result().await.map_err(workflow_failure)?;
            let parsed =
                super::qdrant_child_wire_json::decode_result(&result).map_err(workflow_failure)?;
            if parsed.schema_revision != 1 || parsed.admission_id != command.admission_id {
                return Err(workflow_failure(
                    "Qdrant child returned mismatched durable receipt",
                ));
            }
            results.push(parsed);
        }
        serde_json::to_string(&results).map_err(workflow_failure)
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaAdmittedEffectDagV3Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: EffectDagRunV1) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        let parent = TemporalExecutionIdentityV4 {
            workflow_id: workflow_id.clone(),
            run_id: ctx.run_id().to_string(),
        };
        // Preflight all contracts, admission UUIDs, input bytes and pure
        // implementations before even one irreversible child is initiated.
        let mut machine = MixedDagMachineV5::new(&workflow_id, input).map_err(workflow_failure)?;
        while let Some(step) = machine.next_step().cloned() {
            match step {
                PreparedDagStepV1::Pure { .. } => {
                    machine.advance_pure().map_err(workflow_failure)?;
                }
                PreparedDagStepV1::Owner { .. } => {
                    let children = machine
                        .next_independent_children(parent.clone())
                        .map_err(workflow_failure)?;
                    // Initiate only proven-disjoint children first (one in
                    // the current Truth-only owner model). Every child
                    // remains blocked until its own operator
                    // verifies reciprocal server Histories and grants its
                    // exact immutable PG admission. This is not batch auth.
                    let mut started_children = Vec::with_capacity(children.len());
                    for child in children {
                        let child_id =
                            format!("alegria-pg-effect-v4-{}", child.checked.execution_key);
                        let options = ChildWorkflowOptions::builder()
                            .workflow_id(child_id)
                            .task_queue(PG_V4_CHILD_COMMAND_QUEUE.to_string())
                            .build();
                        let started = match child.route {
                            NativeEffectRouteV5::PgTruthReviewV4 => ctx
                                .start_child_workflow(
                                    PgTruthReviewReconcileV4Definition,
                                    child.invocation,
                                    options,
                                )
                                .await
                                .map_err(workflow_failure)?,
                        };
                        started_children.push((child.checked, started));
                    }
                    // Fan-in is deliberately ordered, so a deterministic
                    // replay never changes which receipt is inspected next.
                    // A failed child does not assert rollback of siblings:
                    // each durable PG effect and revocation is independent.
                    for (checked, started) in started_children {
                        let result_json: String =
                            started.result().await.map_err(workflow_failure)?;
                        let receipt = decode_owner_child_result_v4(&result_json, &checked)
                            .map_err(workflow_failure)?;
                        machine
                            .advance_owner_receipt(receipt)
                            .map_err(workflow_failure)?;
                    }
                }
            }
        }
        let receipt = machine.finish().map_err(workflow_failure)?;
        serde_json::to_string(&receipt).map_err(workflow_failure)
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaAdmittedEffectDagV2Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: EffectDagRunV1) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        // The actual server-issued parent Run ID is read from the SDK context,
        // never from the caller's manifest, a claimed hash or local storage.
        let parent = TemporalExecutionIdentityV4 {
            workflow_id: workflow_id.clone(),
            run_id: ctx.run_id().to_string(),
        };
        // Complete contract, owner and input preflight BEFORE first child.
        let mut machine = MixedDagMachineV4::new(&workflow_id, input).map_err(workflow_failure)?;
        while let Some(step) = machine.next_step().cloned() {
            match step {
                PreparedDagStepV1::Pure { .. } => {
                    machine.advance_pure().map_err(workflow_failure)?;
                }
                PreparedDagStepV1::Owner { checked, .. } => {
                    let invocation = machine
                        .next_owner_child_invocation(parent.clone())
                        .map_err(workflow_failure)?;
                    // A deterministic, immutable instance-scoped child ID.
                    // The operator separately verifies parent History's
                    // StartChildInitiated -> ChildStarted link and the child
                    // History's reciprocal parent execution before admission.
                    let child_id = format!("alegria-pg-effect-v4-{}", checked.execution_key);
                    let options = ChildWorkflowOptions::builder()
                        .workflow_id(child_id)
                        .task_queue(PG_V4_CHILD_COMMAND_QUEUE.to_string())
                        .build();
                    let started = ctx
                        .start_child_workflow(
                            PgTruthReviewReconcileV4Definition,
                            invocation,
                            options,
                        )
                        .await
                        .map_err(workflow_failure)?;
                    // Wait for a genuine typed child result. The child stays
                    // blocked on operator_admission_ready_v4 until a separate
                    // privileged process commits V4 PG admission and signals.
                    let result_json: String = started.result().await.map_err(workflow_failure)?;
                    let receipt = decode_owner_child_result_v4(&result_json, &checked)
                        .map_err(workflow_failure)?;
                    machine
                        .advance_owner_receipt(receipt)
                        .map_err(workflow_failure)?;
                }
            }
        }
        let receipt = machine.finish().map_err(workflow_failure)?;
        serde_json::to_string(&receipt).map_err(workflow_failure)
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaCheckedEffectDagV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: EffectDagRunV1) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        let outcome = execute_checked_dag_v1(&workflow_id, &input).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(error.to_string()),
            ))
        })?;
        serde_json::to_string(&outcome).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(error.to_string()),
            ))
        })
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaPureOperationDagV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: PureDagRun) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        let result = execute_pure_dag_legacy_v3(&workflow_id, &input).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(error.to_string()),
            ))
        })?;
        serde_json::to_string(&result).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(error.to_string()),
            ))
        })
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaPureOperationDagV2Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: PureDagRun) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<PureDagReceipt> {
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        execute_pure_dag(&workflow_id, &input).map_err(workflow_failure)
    }
}

#[cfg(feature = "legacy-workflows")]
#[workflow_methods]
impl AlegriaPureOperationDagV3Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: PureDagRunSchemaPinnedV5) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<PureDagReceipt> {
        let input = ctx.state(|state| state.immutable_input.clone());
        verify_owner_generated_pins_v5(&input).map_err(workflow_failure)?;
        let workflow_id = ctx.info().workflow_id().to_string();
        execute_pure_dag(&workflow_id, &input.run).map_err(workflow_failure)
    }
}

#[cfg(test)]
mod typed_native_child_contract_tests {
    use super::*;

    // A compile-time ABI tripwire: replacing the new generic owner result
    // with the historical JSON `String` would fail type-checking here.
    // The actual V1/V2 child must retain concrete Rust input/output types,
    // even though Temporal's transport stores serialized payload bytes.
    fn owner_child_abi<T: WorkflowDefinition<Input = OwnerInvocationV1, Output = OwnerResultV1>>(
        _: &T,
    ) {
    }

    fn validation_child_abi<
        T: WorkflowDefinition<Input = OwnerValidationRequestV1, Output = OwnerValidationResultV1>,
    >(
        _: &T,
    ) {
    }

    #[cfg(feature = "legacy-workflows")]
    fn strict_pure_v4_abi<T: WorkflowDefinition<Input = PureDagRun, Output = PureDagReceipt>>() {}

    #[cfg(feature = "legacy-workflows")]
    fn schema_pinned_pure_v5_abi<
        T: WorkflowDefinition<Input = PureDagRunSchemaPinnedV5, Output = PureDagReceipt>,
    >() {
    }

    #[cfg(feature = "legacy-workflows")]
    #[test]
    fn strict_pure_v4_has_new_typed_receipt_and_is_not_the_historical_v3_string_workflow() {
        strict_pure_v4_abi::<AlegriaPureOperationDagV2Workflow>();
        assert_ne!(
            "AlegriaPureOperationDagV1Workflow",
            "AlegriaPureOperationDagV2Workflow"
        );
        assert_ne!(
            crate::PURE_DAG_ORCHESTRATOR_QUEUE,
            crate::PURE_DAG_ORCHESTRATOR_V4_QUEUE
        );
    }

    #[cfg(feature = "legacy-workflows")]
    #[test]
    fn schema_pinned_pure_v5_is_a_distinct_history_visible_typed_workflow() {
        schema_pinned_pure_v5_abi::<AlegriaPureOperationDagV3Workflow>();
        assert_ne!(
            "AlegriaPureOperationDagV3Workflow",
            "AlegriaPureOperationDagV2Workflow"
        );
        assert_ne!(
            "AlegriaPureOperationDagV3Workflow",
            "AlegriaPureOperationDagV1Workflow"
        );
        assert_ne!(
            crate::PURE_DAG_ORCHESTRATOR_V5_QUEUE,
            crate::PURE_DAG_ORCHESTRATOR_V4_QUEUE
        );
        assert_ne!(
            crate::PURE_DAG_ORCHESTRATOR_V5_QUEUE,
            crate::PURE_DAG_ORCHESTRATOR_QUEUE
        );
    }

    #[test]
    fn generic_parent_uses_exact_typed_owner_and_validate_only_child_abi() {
        let owner = OwnerOperationV1Definition;
        let validation = OwnerValidationV1Definition;
        owner_child_abi(&owner);
        validation_child_abi(&validation);
        assert_eq!(owner.name(), OWNER_OPERATION_CHILD_TYPE_V1);
        assert_eq!(validation.name(), OWNER_VALIDATION_CHILD_TYPE_V1);
        assert_ne!(owner.name(), validation.name());
    }

    #[test]
    fn parallel_parent_has_a_distinct_history_visible_workflow_and_queue() {
        assert_ne!(
            operation_protocol::OPERATION_DAG_PARENT_TYPE,
            operation_protocol::OPERATION_DAG_PARENT_TYPE_V2
        );
        assert_ne!(
            operation_protocol::OPERATION_DAG_PARENT_QUEUE,
            operation_protocol::OPERATION_DAG_PARENT_QUEUE_V2
        );
        assert_ne!(
            operation_protocol::OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX,
            operation_protocol::OPERATION_DAG_PARENT_WORKFLOW_ID_PREFIX_V2
        );
    }
}
