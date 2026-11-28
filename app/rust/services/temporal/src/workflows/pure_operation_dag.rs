//! Opt-in real Temporal execution of provider-free, versioned operation DAGs.
//!
//! This is an intentionally narrow first executable: only the five registered
//! deterministic pure domain operations may run. Their complete input bytes,
//! plan hash, owner contract fingerprints and artifact envelopes are recorded
//! in the Temporal start event. No Activities, clocks, DB reads, provider calls
//! or publication effects occur in this workflow.
//! At most 64 nodes and 512 KiB of actual serialized input/result are allowed
//! to keep the initial in-Workflow computation and History payload bounded.
//!
//! The workflow is pinned to a deployment's exact source SHA and can be
//! replayed offline. It does NOT make claimed external provenance trustworthy
//! or authorize owner-specific effects. Those require a separate durable
//! owner protocol, never the `allowed_effects` field of a submitted manifest.

use seo_application::operation_adapter::{execute_pure_dag, PureDagRun};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::{
    SyncWorkflowContext, WorkerOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
};

use super::runtime::workflow_contract_failure;

#[workflow]
struct AlegriaPureOperationDagV1Workflow {
    immutable_input: PureDagRun,
}

pub(super) fn register(opts: &mut WorkerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    Ok(())
}

pub(super) fn register_for_replay(opts: &mut WorkflowReplayerOptions) -> anyhow::Result<()> {
    opts.register_workflow::<AlegriaPureOperationDagV1Workflow>()?;
    Ok(())
}

#[workflow_methods]
impl AlegriaPureOperationDagV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, immutable_input: PureDagRun) -> Self {
        Self { immutable_input }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        // Validate the plan's *existing* hash and pinned contracts, run ID,
        // declared effects and all registered executor bindings without ever
        // re-sealing the manifest or scheduling a Temporal Activity.
        let input = ctx.state(|state| state.immutable_input.clone());
        let workflow_id = ctx.info().workflow_id().to_string();
        let result = execute_pure_dag(&workflow_id, &input)
            .map_err(|error| workflow_contract_failure(error.to_string()))?;
        // Carry typed artifact envelopes through a canonical JSON result so
        // callers can inspect actual output. Only a future admission protocol
        // may promote those values to trusted persistent evidence.
        serde_json::to_string(&result).map_err(|error| workflow_contract_failure(error.to_string()))
    }
}
