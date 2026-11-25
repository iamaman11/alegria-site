//! Standalone, provider-free native Temporal operation Workflows.
//!
//! Deliberately the same versioned history-visible type and typed execution
//! semantics as the old opt-in Workflow: the owner implementation and bound
//! artifact contracts are reused from `seo_application`, not reimplemented.
//! This process cannot execute Activities, access provider SDKs or publish.

use seo_application::operation_adapter::{
    execute_checked_dag_v1, execute_pure_dag, EffectDagRunV1, PureDagRun,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::{
    ApplicationFailure, SyncWorkflowContext, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult, WorkflowTermination,
};

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
