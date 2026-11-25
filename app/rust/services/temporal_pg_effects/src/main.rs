//! Experimental V3 native PostgreSQL effect owner. Not the pure DAG worker.
//!
//! Two distinct Task Queues in one opt-in binary:
//! - `alegria-pg-command-v3`: one explicit typed Workflow only;
//! - `alegria-pg-effects-v3`: one PostgreSQL Activity only.
//!
//! The Workflow's start payload never authorizes an effect. The PG Activity
//! cross-checks its *actual Temporal ActivityInfo* and operator-issued
//! immutable PostgreSQL admission, canonical run, full candidate fingerprint,
//! stale-run fence and committed receipt before reporting success.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use operation_protocol::{
    AdmittedArtifactV1, EffectChildInvocationV4, EffectChildResultV4, OwnerInvocationV1,
    OwnerResultV1, PortId, OWNER_OPERATION_CHILD_TYPE_V1,
};
#[cfg(test)]
use postgres_store::truth_review_effect_v3::ReconcileTruthReviewsError;
use postgres_store::truth_review_effect_v3::{
    assert_unprivileged_pg_effect_role, execute_admitted_reconcile,
    execute_admitted_reconcile_owner_v1, execute_admitted_reconcile_v4,
    validate_pg_owner_invocation_v1, ReconcileTruthReviewsInput, ReconcileTruthReviewsReceipt,
    RetryClass, PG_EFFECT_TASK_QUEUE, PG_EFFECT_WORKFLOW_TYPE, PG_EFFECT_WORKFLOW_TYPE_V4,
    PG_EFFECT_WORKFLOW_TYPE_V5, PG_OWNER_TASK_QUEUE_V1,
};
use sqlx::PgPool;
use temporalio_client::WorkflowHistory;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityOptions, ApplicationFailure, Runtime, SyncWorkflowContext, Worker, WorkerOptions,
    WorkflowContext, WorkflowContextView, WorkflowResult,
};

mod operator;
mod operator_dag_governance;
mod operator_owner_v1;
mod operator_v4;

/// Production can mount a Docker/Orchestrator secret as a private file;
/// credentials never have to appear in a process environment. Exactly one
/// distinct V3/V4 Worker DSN source is permitted; there is no DATABASE_URL
/// fallback. The operator has its own separately authorized credentials.
fn worker_database_url() -> Result<String> {
    let inline = std::env::var("PG_EFFECT_WORKER_DATABASE_URL").ok();
    let file_path = std::env::var("PG_EFFECT_WORKER_DATABASE_URL_FILE").ok();
    let url = match (inline, file_path) {
        (Some(_), Some(_)) => anyhow::bail!(
            "configure only one of PG_EFFECT_WORKER_DATABASE_URL or PG_EFFECT_WORKER_DATABASE_URL_FILE"
        ),
        (Some(url), None) => url,
        (None, Some(file_path)) => {
            let path = std::path::Path::new(&file_path);
            anyhow::ensure!(path.is_absolute(), "PG effect DSN secret path must be absolute");
            let metadata = std::fs::symlink_metadata(path)
                .with_context(|| format!("stat PG effect DSN secret {}", path.display()))?;
            anyhow::ensure!(metadata.file_type().is_file(), "PG effect DSN secret must be a regular file, not symlink");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                anyhow::ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "PG effect DSN secret must not be readable or writable by group/others (0600/0400)"
                );
            }
            std::fs::read_to_string(path)
                .with_context(|| format!("read PG effect DSN secret {}", path.display()))?
                .trim_end_matches(|c| c == '\r' || c == '\n')
                .to_owned()
        }
        (None, None) => anyhow::bail!(
            "PG effect Worker requires PG_EFFECT_WORKER_DATABASE_URL_FILE or PG_EFFECT_WORKER_DATABASE_URL"
        ),
    };
    anyhow::ensure!(
        !url.trim().is_empty() && url.trim() == url && !url.contains('\r') && !url.contains('\n'),
        "PG effect Worker DSN is blank or contains unsafe whitespace"
    );
    Ok(url)
}

// Required by Temporal proc-macro expansions.
use futures_util as _;
use temporalio_common as _;

const COMMAND_TASK_QUEUE: &str = "alegria-pg-command-v3";
// New typed-result owner child is isolated from the frozen V3/V4 History
// contracts and their existing Task Queue.
const V5_CHILD_COMMAND_TASK_QUEUE: &str = "alegria-pg-command-v5";

struct PgTruthReviewActivities {
    pool: Arc<PgPool>,
}

#[activities]
impl PgTruthReviewActivities {
    /// A generic DAG cannot schedule PostgreSQL writes by itself: the
    /// independent operator must attest the actual generic child History
    /// and issue revision 5 admission before this Activity may consume it.
    #[activity]
    pub(crate) async fn reconcile_truth_owner_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<ReconcileTruthReviewsReceipt, ActivityError> {
        let info = ctx.info();
        let workflow_id = info
            .workflow_id
            .as_deref()
            .ok_or_else(|| activity_terminal("PG owner needs server child Workflow ID"))?;
        let run_id = info
            .workflow_run_id
            .as_deref()
            .ok_or_else(|| activity_terminal("PG owner needs server child Workflow Run ID"))?;
        if info.workflow_type.as_deref() != Some(OWNER_OPERATION_CHILD_TYPE_V1)
            || info.task_queue != PG_EFFECT_TASK_QUEUE
        {
            return Err(activity_terminal(
                "PG generic Activity invoked outside exact owner Workflow/Activity queue",
            ));
        }
        execute_admitted_reconcile_owner_v1(&self.pool, workflow_id, run_id, &invocation)
            .await
            .map_err(|err| {
                let failure = match err.retry_class() {
                    RetryClass::Never => {
                        ApplicationFailure::non_retryable(std::io::Error::other(err.to_string()))
                    }
                    RetryClass::RetryWithSameAdmission => {
                        ApplicationFailure::new(std::io::Error::other(err.to_string()))
                    }
                };
                ActivityError::application(failure)
            })
    }

    /// Reconcile actual canonical source candidates, with one atomic PG
    /// transaction + receipt. An arbitrary direct Activity RPC is rejected:
    /// it has no admitted Workflow provenance or exact operation admission.
    #[activity]
    pub(crate) async fn reconcile_truth_candidate_reviews_v3(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: ReconcileTruthReviewsInput,
    ) -> Result<ReconcileTruthReviewsReceipt, ActivityError> {
        let info = ctx.info();
        let actual_workflow_id = info.workflow_id.as_deref().ok_or_else(|| {
            activity_terminal("PG effect must originate from a Temporal Workflow")
        })?;
        let actual_workflow_run_id = info.workflow_run_id.as_deref().ok_or_else(|| {
            activity_terminal(
                "PG effect must originate from a specific Temporal Workflow execution",
            )
        })?;
        if info.workflow_type.as_deref() != Some(PG_EFFECT_WORKFLOW_TYPE)
            || info.task_queue != PG_EFFECT_TASK_QUEUE
        {
            return Err(activity_terminal(
                "PG effect Activity provenance is not its pinned V3 owner Workflow/Queue",
            ));
        }
        // No `workflow_id` is read from untrusted Activity input. It comes
        // from server-attested ActivityInfo and must match operator admission.
        execute_admitted_reconcile(
            &self.pool,
            actual_workflow_id,
            actual_workflow_run_id,
            &input,
        )
        .await
        .map_err(|err| {
            let failure = match err.retry_class() {
                RetryClass::Never => {
                    ApplicationFailure::non_retryable(std::io::Error::other(err.to_string()))
                }
                RetryClass::RetryWithSameAdmission => {
                    ApplicationFailure::new(std::io::Error::other(err.to_string()))
                }
            };
            ActivityError::application(failure)
        })
    }

    /// The server-attested V4 child workflow can consume only a V4 admission
    /// pinned to its exact child execution and checked parent dispatch.
    #[activity]
    pub(crate) async fn reconcile_truth_candidate_reviews_v4(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: EffectChildInvocationV4,
    ) -> Result<ReconcileTruthReviewsReceipt, ActivityError> {
        let info = ctx.info();
        let workflow_id = info.workflow_id.as_deref().ok_or_else(|| {
            activity_terminal("V4 effect requires server-attested child Workflow ID")
        })?;
        let run_id = info.workflow_run_id.as_deref().ok_or_else(|| {
            activity_terminal("V4 effect requires server-attested child Workflow Run ID")
        })?;
        if !info.workflow_type.as_deref().is_some_and(|workflow_type| {
            workflow_type == PG_EFFECT_WORKFLOW_TYPE_V4
                || workflow_type == PG_EFFECT_WORKFLOW_TYPE_V5
        }) || info.task_queue != PG_EFFECT_TASK_QUEUE
        {
            return Err(activity_terminal(
                "V4/V5 Activity invoked outside exact registered PG child Workflow/Queue",
            ));
        }
        execute_admitted_reconcile_v4(&self.pool, workflow_id, run_id, &invocation)
            .await
            .map_err(|err| {
                let failure = match err.retry_class() {
                    RetryClass::Never => {
                        ApplicationFailure::non_retryable(std::io::Error::other(err.to_string()))
                    }
                    RetryClass::RetryWithSameAdmission => {
                        ApplicationFailure::new(std::io::Error::other(err.to_string()))
                    }
                };
                ActivityError::application(failure)
            })
    }
}

fn activity_terminal(message: &str) -> ActivityError {
    ActivityError::application(ApplicationFailure::non_retryable(std::io::Error::other(
        message.to_string(),
    )))
}

#[workflow]
struct PgTruthReviewReconcileV3Workflow {
    immutable_command: ReconcileTruthReviewsInput,
    admission_signal_received: bool,
}

/// New history-visible type: V3 history cannot silently acquire a new
/// parent/child authorization interpretation during rolling deployment.
#[workflow]
struct PgTruthReviewReconcileV4Workflow {
    immutable_invocation: EffectChildInvocationV4,
    admission_signal_received: bool,
}

/// Dedicated typed child for the mixed-owner V6 parent. An old V4 History
/// must never be silently reinterpreted as an SDK-typed result.
#[workflow]
struct PgTruthReviewReconcileV5Workflow {
    immutable_invocation: EffectChildInvocationV4,
    admission_signal_received: bool,
}

/// ONE Temporal Workflow Type for all independent owners, with a distinct
/// PostgreSQL Task Queue. Only this PG Worker decodes the typed Truth command.
/// It is not a generic SQL execution service.
#[workflow]
struct AlegriaOwnerOperationV1Workflow {
    immutable_invocation: OwnerInvocationV1,
    admission_signal_received: bool,
}

#[workflow_methods]
impl AlegriaOwnerOperationV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: OwnerInvocationV1) -> Self {
        Self {
            immutable_invocation: invocation,
            admission_signal_received: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerResultV1> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        validate_pg_owner_invocation_v1(&invocation).map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })?;
        // Signal is deliberately NOT a capability. SQL independently checks
        // exact operator-issued rev5 admission and candidate snapshot.
        ctx.wait_condition(|state| state.admission_signal_received)
            .await?;
        let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
        options.task_queue = Some(PG_EFFECT_TASK_QUEUE.to_string());
        let receipt: ReconcileTruthReviewsReceipt = ctx
            .start_activity(
                PgTruthReviewActivities::reconcile_truth_owner_v1,
                invocation.clone(),
                options,
            )
            .await?;
        let receipt_bytes = serde_json::to_vec(&receipt).map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })?;
        let output_contract = truth_domain::review_effect_contract::operation_contract();
        let output_port = PortId::from(truth_domain::review_effect_contract::OUTPUT_PORT);
        let output_schema = output_contract
            .outputs
            .get(&output_port)
            .cloned()
            .ok_or_else(|| {
                temporalio_sdk::WorkflowTermination::failed_application(
                    ApplicationFailure::non_retryable(std::io::Error::other(
                        "missing PG output schema",
                    )),
                )
            })?;
        Ok(OwnerResultV1 {
            dispatch_execution_key: invocation.checked_dispatch.execution_key,
            outputs: BTreeMap::from([(
                output_port,
                AdmittedArtifactV1 {
                    schema: output_schema,
                    scope: invocation.checked_dispatch.scope,
                    provenance: format!("pg.truth_review.receipt_v1:{}", receipt.receipt_id),
                    payload: receipt_bytes.clone(),
                },
            )]),
            receipt_bytes,
        })
    }

    #[signal(name = "operator_admission_ready_v5")]
    fn operator_admission_ready_v5(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.admission_signal_received = true;
    }
}

#[workflow_methods]
impl PgTruthReviewReconcileV5Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: EffectChildInvocationV4) -> Self {
        Self {
            immutable_invocation: invocation,
            admission_signal_received: false,
        }
    }

    #[run]
    async fn run(
        ctx: &mut WorkflowContext<Self>,
    ) -> WorkflowResult<EffectChildResultV4<ReconcileTruthReviewsReceipt>> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        invocation.verify_integrity().map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })?;
        ctx.wait_condition(|state| state.admission_signal_received)
            .await?;
        let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
        options.task_queue = Some(PG_EFFECT_TASK_QUEUE.to_string());
        let receipt: ReconcileTruthReviewsReceipt = ctx
            .start_activity(
                PgTruthReviewActivities::reconcile_truth_candidate_reviews_v4,
                invocation.clone(),
                options,
            )
            .await?;
        Ok(EffectChildResultV4 {
            schema_revision: 4,
            dispatch_execution_key: invocation.checked_dispatch.execution_key,
            pg_receipt: receipt,
        })
    }

    #[signal(name = "operator_admission_ready_v4")]
    fn operator_admission_ready_v4(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        // Signal wakes the Workflow. Independent PostgreSQL admission and
        // server-provenance checks still authorize the actual effect.
        self.admission_signal_received = true;
    }
}

#[workflow_methods]
impl PgTruthReviewReconcileV4Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: EffectChildInvocationV4) -> Self {
        Self {
            immutable_invocation: invocation,
            admission_signal_received: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        invocation.verify_integrity().map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })?;
        ctx.wait_condition(|state| state.admission_signal_received)
            .await?;
        let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
        options.task_queue = Some(PG_EFFECT_TASK_QUEUE.to_string());
        let receipt: ReconcileTruthReviewsReceipt = ctx
            .start_activity(
                PgTruthReviewActivities::reconcile_truth_candidate_reviews_v4,
                invocation.clone(),
                options,
            )
            .await?;
        // V4's durable SQL receipt execution_key equals the checked dispatch
        // key; V3 retains its older command/workflow-derived key namespace.
        // Preserve both V4 fields so the parent validates this exact link.
        serde_json::to_string(&EffectChildResultV4::<ReconcileTruthReviewsReceipt> {
            schema_revision: 4,
            dispatch_execution_key: invocation.checked_dispatch.execution_key,
            pg_receipt: receipt,
        })
        .map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })
    }

    #[signal(name = "operator_admission_ready_v4")]
    fn operator_admission_ready_v4(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        // An arbitrary Signal is not authority; only PG V4 admitted effect
        // consumption with exact server ActivityInfo can cross this gate.
        self.admission_signal_received = true;
    }
}

#[workflow_methods]
impl PgTruthReviewReconcileV3Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, input: ReconcileTruthReviewsInput) -> Self {
        Self {
            immutable_command: input,
            admission_signal_received: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let input = ctx.state(|state| state.immutable_command.clone());
        // Operator starts the Workflow first, observes the server-generated
        // Workflow Run ID, issues a *distinct privileged PostgreSQL admission*,
        // then signals readiness. Without this gate the Activity could run
        // before the operator knew the Temporal Run ID and wrongly fail the
        // legitimate issuance path. Signals do NOT authorize anything.
        ctx.wait_condition(|state| state.admission_signal_received)
            .await?;
        // No arbitrary graph plan / allowed_effects is accepted here. The
        // trusted operator must separately insert an admission in PostgreSQL.
        // The Activity has the final authoritative veto even if this History
        // is replayed or the caller forges a fully sealed manifest.
        // Schedule-to-close caps the entire retry window (including queued
        // retries). No indefinitely retrying PostgreSQL effect in V3.
        let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
        options.task_queue = Some(PG_EFFECT_TASK_QUEUE.to_string());
        let receipt: ReconcileTruthReviewsReceipt = ctx
            .start_activity(
                PgTruthReviewActivities::reconcile_truth_candidate_reviews_v3,
                input,
                options,
            )
            .await?;
        serde_json::to_string(&receipt).map_err(|error| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(error.to_string())),
            )
        })
    }

    #[signal(name = "operator_admission_ready")]
    fn operator_admission_ready(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        // Untrusted callers can signal this too. The Activity still refuses
        // without exact operator-issued admission from PostgreSQL.
        self.admission_signal_received = true;
    }
}

/// Replay an actual persisted Workflow History with the SDK, offline. The
/// operator's admission, PostgreSQL connection and Activity Worker are never
/// available to replay; completed Activity outcomes come from History.
async fn replay_history(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read real PG V3 Temporal history {}", path.display()))?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|error| anyhow::anyhow!("decode PG V3 Temporal history: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_string())
        .task_queue(COMMAND_TASK_QUEUE.to_string())
        .build();
    options.register_workflow::<PgTruthReviewReconcileV3Workflow>()?;
    options.register_workflow::<PgTruthReviewReconcileV4Workflow>()?;
    options.register_workflow::<PgTruthReviewReconcileV5Workflow>()?;
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("construct PG V3 Temporal replayer: {error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("PG V3 Temporal history replay failed: {error}"))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // A replay-only invocation is intentionally independent of runtime
    // credentials, live Temporal, and the experimental Worker enable flag.
    let mut args = std::env::args_os().skip(1);
    if let Some(argument) = args.next() {
        if argument == std::ffi::OsStr::new("--replay-history") {
            let path = args
                .next()
                .context("--replay-history requires one JSON file")?;
            anyhow::ensure!(args.next().is_none(), "unexpected replay arguments");
            replay_history(std::path::Path::new(&path)).await?;
            println!("CI_TEMPORAL_PG_EFFECT_REPLAY=PASS");
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-issue") {
            let path = args
                .next()
                .context("--operator-issue requires one JSON file")?;
            anyhow::ensure!(args.next().is_none(), "unexpected operator issue arguments");
            operator::issue_from_file(std::path::Path::new(&path)).await?;
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-issue-v4") {
            let path = args
                .next()
                .context("--operator-issue-v4 requires one JSON file")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected V4 operator issue arguments"
            );
            operator_v4::issue_from_file(std::path::Path::new(&path)).await?;
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-issue-owner-v1") {
            let path = args
                .next()
                .context("--operator-issue-owner-v1 requires one strict JSON locator file")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected PG owner operator arguments"
            );
            operator_owner_v1::issue_from_file(std::path::Path::new(&path)).await?;
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-admit-dag-run-v1") {
            let path = args.next().context(
                "--operator-admit-dag-run-v1 requires strict JSON parent locator"
            )?;
            anyhow::ensure!(args.next().is_none(), "unexpected DAG governance operator arguments");
            operator_dag_governance::issue_from_file(std::path::Path::new(&path)).await?;
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-revoke") {
            let id = args.next().context("--operator-revoke requires one UUID")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected operator revoke arguments"
            );
            operator::revoke_by_id(&id.to_string_lossy()).await?;
            return Ok(());
        }
        if argument == std::ffi::OsStr::new("--operator-revoke-v4") {
            let id = args
                .next()
                .context("--operator-revoke-v4 requires one UUID")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected V4 operator revoke arguments"
            );
            operator_v4::revoke_by_id(&id.to_string_lossy()).await?;
            return Ok(());
        }
        anyhow::bail!("unexpected temporal_pg_effects argument: {argument:?}");
    }
    anyhow::ensure!(
        std::env::var("ALEGRIA_ENABLE_PG_EFFECT_V3").ok().as_deref() == Some("true"),
        "experimental PG effect owner disabled; set ALEGRIA_ENABLE_PG_EFFECT_V3=true explicitly"
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Deliberately not DATABASE_URL: an application admin connection can
    // create its own admissions and is a forbidden runtime identity.
    let database_url = worker_database_url()?;
    let pool = PgPool::connect(&database_url)
        .await
        .context("connect dedicated PG effect database role")?;
    assert_unprivileged_pg_effect_role(&pool)
        .await
        .context("PG effect admission privilege boundary")?;

    let options = RuntimeOptions::builder()
        .build()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let runtime = Runtime::from_current_tokio(options)?;
    let temporal_url =
        std::env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".to_string());
    let identity = format!("alegria-pg-effect-v3:{}", std::process::id());
    let client = temporal_gateway::connect_client(&temporal_url, identity, "default").await?;

    // Command and effect queues intentionally have disjoint registrations.
    // Every poller on one queue sees the same registered Types; this does
    // not join the legacy V1/V2 or pure-DAG worker Task Queue.
    let mut command_options = WorkerOptions::new(COMMAND_TASK_QUEUE).build();
    command_options.register_workflow::<PgTruthReviewReconcileV3Workflow>()?;
    command_options.register_workflow::<PgTruthReviewReconcileV4Workflow>()?;
    let mut command_worker = Worker::new(&runtime, client.clone(), command_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut v5_options = WorkerOptions::new(V5_CHILD_COMMAND_TASK_QUEUE).build();
    v5_options.register_workflow::<PgTruthReviewReconcileV5Workflow>()?;
    let mut v5_command_worker = Worker::new(&runtime, client.clone(), v5_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    // The generic queue is independently opt-in; a versioned deployment
    // whose operator/migration is absent cannot accidentally poll generic
    // commands through a legacy owner queue.
    let generic_enabled =
        std::env::var("ALEGRIA_ENABLE_PG_OWNER_V1").ok().as_deref() == Some("true");
    let mut generic_worker = if generic_enabled {
        let mut owner_options = WorkerOptions::new(PG_OWNER_TASK_QUEUE_V1).build();
        owner_options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
        Some(
            Worker::new(&runtime, client.clone(), owner_options)
                .map_err(|error| anyhow::anyhow!("{error}"))?,
        )
    } else {
        None
    };
    let activity_options = WorkerOptions::new(PG_EFFECT_TASK_QUEUE)
        .register_activities(PgTruthReviewActivities {
            pool: Arc::new(pool),
        })
        .build();
    let mut activity_worker = Worker::new(&runtime, client, activity_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    tracing::info!(
        command_queue = COMMAND_TASK_QUEUE,
        activity_queue = PG_EFFECT_TASK_QUEUE,
        "experimental PostgreSQL Truth review owner polling opt-in queues"
    );
    let generic_run = async {
        if let Some(worker) = &mut generic_worker {
            worker.run().await
        } else {
            std::future::pending().await
        }
    };
    tokio::try_join!(
        command_worker.run(),
        v5_command_worker.run(),
        activity_worker.run(),
        generic_run
    )
    .context("PG effect owner stopped")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_worker_has_distinct_temporal_queues() {
        assert_ne!(COMMAND_TASK_QUEUE, PG_EFFECT_TASK_QUEUE);
        assert_ne!(COMMAND_TASK_QUEUE, "alegria-pipeline");
        assert_ne!(PG_EFFECT_TASK_QUEUE, "alegria-pure-dag-v3");
        assert_ne!(V5_CHILD_COMMAND_TASK_QUEUE, COMMAND_TASK_QUEUE);
        assert_ne!(V5_CHILD_COMMAND_TASK_QUEUE, PG_EFFECT_TASK_QUEUE);
        assert_ne!(PG_OWNER_TASK_QUEUE_V1, COMMAND_TASK_QUEUE);
        assert_ne!(PG_OWNER_TASK_QUEUE_V1, V5_CHILD_COMMAND_TASK_QUEUE);
    }

    #[test]
    fn terminal_admission_errors_are_not_temporal_retryable() {
        let invalid = ReconcileTruthReviewsError::AdmissionDenied;
        assert_eq!(invalid.retry_class(), RetryClass::Never);
        let stale = ReconcileTruthReviewsError::StaleTruthRun;
        assert_eq!(stale.retry_class(), RetryClass::Never);
        let moved = ReconcileTruthReviewsError::CandidateSetChanged;
        assert_eq!(moved.retry_class(), RetryClass::Never);
    }

    #[tokio::test]
    async fn replay_requires_real_sdk_history_even_without_worker_secrets() {
        let nonexistent = std::env::temp_dir().join(format!(
            "alegria-pg-v3-no-such-history-{}",
            std::process::id()
        ));
        assert!(replay_history(&nonexistent).await.is_err());
    }
}
