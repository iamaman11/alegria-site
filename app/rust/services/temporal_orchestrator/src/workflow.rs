//! Standalone, provider-free native Temporal operation Workflows.
//!
//! Deliberately the same versioned history-visible type and typed execution
//! semantics as the old opt-in Workflow: the owner implementation and bound
//! artifact contracts are reused from `seo_application`, not reimplemented.
//! This process cannot execute Activities, access provider SDKs or publish.

use operation_protocol::{EffectChildInvocationV4, TemporalExecutionIdentityV4};
use seo_application::operation_adapter::{
    decode_owner_child_result_v4, execute_checked_dag_v1, execute_pure_dag, EffectDagRunV1,
    MixedDagMachineV4, PreparedDagStepV1, PureDagRun,
};
use temporalio_common::WorkflowDefinition;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::{
    ApplicationFailure, ChildWorkflowOptions, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult, WorkflowTermination,
};

pub(crate) const PG_V4_CHILD_WORKFLOW_TYPE: &str = "PgTruthReviewReconcileV4Workflow";
pub(crate) const PG_V4_CHILD_COMMAND_QUEUE: &str = "alegria-pg-command-v3";

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
