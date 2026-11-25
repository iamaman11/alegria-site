//! Standalone, provider-free native Temporal operation Workflows.
//!
//! Deliberately the same versioned history-visible type and typed execution
//! semantics as the old opt-in Workflow: the owner implementation and bound
//! artifact contracts are reused from `seo_application`, not reimplemented.
//! This process cannot execute Activities, access provider SDKs or publish.

use operation_protocol::{
    EffectChildInvocationV4, OperationDagRunV6, OwnerInvocationV1, OwnerResultV1,
    TemporalExecutionIdentityV4, TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1,
};
use qdrant_contracts::{
    QdrantKeywordChildInvocationV1, QdrantKeywordChildResultV1, QdrantProjectionRunV1,
    QDRANT_OWNER_CHILD_TYPE_V1, QDRANT_OWNER_COMMAND_QUEUE_V1,
};
use seo_application::operation_adapter::{
    decode_owner_child_result_v4, execute_checked_dag_v1, execute_pure_dag, EffectDagRunV1,
    MixedDagMachineV4, MixedDagMachineV5, NativeEffectRouteV5, OperationDagMachine,
    OperationDagReceiptV1, PreparedDagStepV1, PureDagRun,
};
use std::sync::OnceLock;
use temporalio_common::WorkflowDefinition;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::{
    ApplicationFailure, ChildWorkflowOptions, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult, WorkflowTermination,
};

pub(crate) const PG_V4_CHILD_WORKFLOW_TYPE: &str = "PgTruthReviewReconcileV4Workflow";
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
#[derive(Clone, Copy)]
struct PgTruthReviewReconcileV4Definition;

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
#[derive(Clone, Copy)]
struct QdrantKeywordProjectionV1Definition;

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

fn workflow_failure(error: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(error.to_string()),
    ))
}

#[workflow]
struct AlegriaPureOperationDagV1Workflow {
    immutable_input: PureDagRun,
}

/// The manifest may contain real PostgreSQL effects, but the current V3
/// admission schema cannot bind independently attested child Workflow Run
/// IDs to one canonical pipeline run. This Workflow intentionally performs
/// complete typed preflight and then fails non-retryably BEFORE any Activity
/// or child Workflow. It cannot emit a fabricated PG receipt.
#[workflow]
struct AlegriaCheckedEffectDagV1Workflow {
    immutable_input: EffectDagRunV1,
}

/// New history-visible type: never reinterpret V1 preflight-only histories
/// as authorized effects. This V2 parent can only invoke typed PG child
/// Workflows, which wait for independent operator-issued V4 admission before
/// their distinct owner Activity can perform any PostgreSQL business write.
#[workflow]
struct AlegriaAdmittedEffectDagV2Workflow {
    immutable_input: EffectDagRunV1,
}

/// New History-visible V5 parent. It sequences independently admissible PG
/// owner child Workflows in deterministic topological order. The grouped
/// start/join boundary is reserved for genuinely disjoint future owner
/// routes; today's single canonical PG Truth context always serializes.
/// Old V2 histories are never upgraded in place.
#[workflow]
struct AlegriaAdmittedEffectDagV3Workflow {
    immutable_input: EffectDagRunV1,
}

/// V7 new History-visible typed serial owner route. Deliberately not a
/// reinterpretation of prior V5 DAG Histories or arbitrary PG/Qdrant fanout.
/// All Qdrant effects must have independent Temporal parent/child proof.
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

pub(crate) fn register(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    opts.register_workflow::<AlegriaCheckedEffectDagV1Workflow>()?;
    Ok(())
}

pub(crate) fn register_for_replay(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    opts.register_workflow::<AlegriaCheckedEffectDagV1Workflow>()?;
    Ok(())
}

pub(crate) fn register_v4(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV2Workflow>()?;
    Ok(())
}

pub(crate) fn register_for_replay_v4(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV2Workflow>()?;
    Ok(())
}

pub(crate) fn register_v5(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV3Workflow>()?;
    Ok(())
}

pub(crate) fn register_for_replay_v5(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaAdmittedEffectDagV3Workflow>()?;
    Ok(())
}

pub(crate) fn register_qdrant_v1(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaQdrantProjectionDagV1Workflow>()?;
    Ok(())
}

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
            let child_id = format!(
                "alegria-owner-operation-v1-{}",
                invocation.checked_dispatch.execution_key
            );
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
        let result = execute_pure_dag(&workflow_id, &input).map_err(|error| {
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
