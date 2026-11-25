//! V7 experimental real Temporal -> PostgreSQL-admitted Qdrant owner.
//! Native Workflow and Activity have separate pinned Task Queues; neither
//! a forged Workflow start nor an arbitrary Signal can authorize Qdrant I/O.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use qdrant_contracts::{
    QdrantChildOutcomeV1, QdrantKeywordChildInvocationV1, QdrantKeywordChildResultV1,
    QDRANT_OWNER_ACTIVITY_QUEUE_V1, QDRANT_OWNER_CHILD_TYPE_V1, QDRANT_OWNER_COMMAND_QUEUE_V1,
};
use qdrant_effects_core::{execute_admitted_qdrant_projection_v1, QdrantProjectionOutcomeV1};
use sqlx::{PgPool, Row};
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, WorkflowHistory};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, ApplicationFailure, Runtime, SyncWorkflowContext,
    Worker, WorkerOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
};
use url::Url;

use futures_util as _;
use temporalio_common as _;

mod operator;
mod operator_v2;
mod operator_v3;
mod owner_generic_v1;
mod owner_v2;

// This owner must not acquire the site's SEO gateway just to connect to
// Temporal. The SDK is already needed for its native Workflow and Activities.
async fn connect_client(temporal_url: &str, identity: String, namespace: &str) -> Result<Client> {
    let options = ConnectionOptions::new(Url::parse(temporal_url)?)
        .identity(identity)
        .build();
    let connection = Connection::connect(options).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn qdrant_activity_options() -> ActivityOptions {
    let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
    options.task_queue = Some(QDRANT_OWNER_ACTIVITY_QUEUE_V1.to_string());
    options
}

fn qdrant_admission_probe_options() -> ActivityOptions {
    // Admission is a strictly read-only check: after a Worker crash it may
    // safely be retried. A STARTED Activity without start-to-close timeout
    // otherwise strands the Workflow for the full 300s schedule-to-close
    // timeout, even if the operator sends a valid wakeup after SQL COMMIT.
    // The provider Activity deliberately does NOT use this timeout: its RPC
    // may have an unknown outcome and may never be blindly duplicated.
    let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(15));
    options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
        schedule_to_close: Duration::from_secs(300),
        start_to_close: Duration::from_secs(15),
    };
    options.task_queue = Some(QDRANT_OWNER_ACTIVITY_QUEUE_V1.to_string());
    options
}

#[workflow]
struct QdrantKeywordProjectionV1Workflow {
    immutable_invocation: QdrantKeywordChildInvocationV1,
    admission_signals_received: u64,
}

#[workflow_methods]
impl QdrantKeywordProjectionV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: QdrantKeywordChildInvocationV1) -> Self {
        Self {
            immutable_invocation: invocation,
            admission_signals_received: 0,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        if invocation.validate().is_err() {
            return Err(temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(
                    "invalid typed Qdrant child invocation",
                )),
            ));
        }
        // A caller capable of sending a Temporal Signal does not possess
        // PostgreSQL operator authority. A forged early wakeup must neither
        // perform a provider RPC nor permanently poison a legitimate child.
        // Count signals rather than resetting a Boolean: the real operator
        // can signal while a read-only check is pending without losing wakeup.
        let mut inspected_signals = 0;
        loop {
            let previous = inspected_signals;
            ctx.wait_condition(move |state| state.admission_signals_received > previous)
                .await?;
            inspected_signals = ctx.state(|state| state.admission_signals_received);
            let admitted: bool = ctx
                .start_activity(
                    QdrantProjectionActivities::check_operator_admission_v1,
                    invocation.clone(),
                    qdrant_admission_probe_options(),
                )
                .await?;
            if admitted {
                break;
            }
        }
        let result: QdrantKeywordChildResultV1 = ctx
            .start_activity(
                QdrantProjectionActivities::materialize_admitted_keyword_v1,
                invocation,
                qdrant_activity_options(),
            )
            .await?;
        serde_json::to_string(&result).map_err(|err| {
            temporalio_sdk::WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(std::io::Error::other(err.to_string())),
            )
        })
    }

    #[signal(name = "operator_admission_ready_qdrant_v1")]
    fn operator_admission_ready_qdrant_v1(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.admission_signals_received = self.admission_signals_received.saturating_add(1);
    }
}

struct QdrantProjectionActivities {
    pool: Arc<PgPool>,
}

// Read-only admission lookup is safely repeatable after a database outage.
// Rejected Workflow provenance or mismatched immutable SQL identity is not.
// The provider RPC itself remains non-retryable after an ambiguous outcome.
#[derive(Debug)]
enum AdmissionProbeError {
    Storage(sqlx::Error),
    Denied(anyhow::Error),
}

// SQL transport failures should retry an idempotent read. A permanently
// malformed schema, missing SELECT grant, failed constraint or SQLSTATE AV001
// must NOT create an endlessly retrying child Activity.
fn sqlstate_allows_admission_probe_retry(sqlstate: &str) -> bool {
    sqlstate.starts_with("08") // PostgreSQL connection exception
        || matches!(
            sqlstate,
            "40001" // serialization failure
                | "40003" // statement completion unknown (read-only probe)
                | "40P01" // deadlock detected
                | "55P03" // lock unavailable
                | "53300" // too many connections
                | "53400" // configuration limit exceeded
                | "57P01" // admin shutdown
                | "57P02" // crash shutdown
                | "57P03" // cannot connect now
        )
}

fn classify_admission_sql_error(error: sqlx::Error) -> AdmissionProbeError {
    let retryable = match &error {
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|code| sqlstate_allows_admission_probe_retry(code.as_ref())),
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
        // Decode errors, unknown SQLx variants and programmer/schema errors
        // are permanent. Never treat every SQL query failure as connectivity.
        _ => false,
    };
    if retryable {
        AdmissionProbeError::Storage(error)
    } else {
        AdmissionProbeError::Denied(error.into())
    }
}

impl std::fmt::Display for AdmissionProbeError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => write!(fmt, "retryable admission read: {error}"),
            Self::Denied(error) => write!(fmt, "permanent admission rejection: {error}"),
        }
    }
}

impl std::error::Error for AdmissionProbeError {}

fn admission_probe_activity_error(error: AdmissionProbeError) -> ActivityError {
    let failure = match error {
        AdmissionProbeError::Storage(error) => {
            ApplicationFailure::new(std::io::Error::other(error.to_string()))
        }
        AdmissionProbeError::Denied(error) => {
            ApplicationFailure::non_retryable(std::io::Error::other(error.to_string()))
        }
    };
    ActivityError::application(failure)
}

fn terminal_activity_error(error: impl std::fmt::Display) -> ActivityError {
    ActivityError::application(ApplicationFailure::non_retryable(std::io::Error::other(
        error.to_string(),
    )))
}

#[activities]
impl QdrantProjectionActivities {
    #[activity]
    pub(crate) async fn check_operator_admission_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: QdrantKeywordChildInvocationV1,
    ) -> Result<bool, ActivityError> {
        // The Signal is unauthenticated data. Each child has at most one
        // admission probe in flight; spacing probes inside this read-only
        // Activity bounds DB load under a Signal flood without imposing a
        // finite retry count that an attacker could exhaust before admission.
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.matching_server_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)
    }

    #[activity]
    pub(crate) async fn materialize_admitted_keyword_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: QdrantKeywordChildInvocationV1,
    ) -> Result<QdrantKeywordChildResultV1, ActivityError> {
        self.materialize_with_server_info(ctx, invocation).await
    }
}

impl QdrantProjectionActivities {
    async fn matching_server_admission(
        &self,
        ctx: &ActivityContext,
        invocation: &QdrantKeywordChildInvocationV1,
    ) -> std::result::Result<bool, AdmissionProbeError> {
        let info = ctx.info();
        let (actual_id, actual_run_id) = (|| -> Result<(&str, &str)> {
            let actual_id = info
                .workflow_id
                .as_deref()
                .context("Activity lacks server child Workflow ID")?;
            let actual_run_id = info
                .workflow_run_id
                .as_deref()
                .context("Activity lacks server child Run ID")?;
            anyhow::ensure!(
                info.workflow_type.as_deref() == Some(QDRANT_OWNER_CHILD_TYPE_V1)
                    && info.task_queue == QDRANT_OWNER_ACTIVITY_QUEUE_V1
                    && actual_id
                        == format!(
                            "alegria-qdrant-effect-v1-{}",
                            invocation.command.admission_id
                        ),
                "Qdrant Activity originated outside registered native child Workflow/Queue"
            );
            invocation.validate().map_err(anyhow::Error::msg)?;
            Ok((actual_id, actual_run_id))
        })()
        .map_err(AdmissionProbeError::Denied)?;
        let row = sqlx::query(
            "SELECT child_workflow_id,child_workflow_run_id,parent_workflow_id,
                    parent_workflow_run_id,run_id,tenant_id,context_key,keyword_key,
                    command_bytes FROM pipeline.qdrant_projection_v1_admissions
             WHERE admission_id=$1",
        )
        .bind(invocation.command.admission_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(classify_admission_sql_error)?;
        let Some(row) = row else {
            return Ok(false);
        };
        (|| -> Result<()> {
            let actual_command = serde_json::to_vec(&invocation.command)?;
            anyhow::ensure!(
            row.try_get::<String, _>("child_workflow_id")? == actual_id
                && row.try_get::<String, _>("child_workflow_run_id")? == actual_run_id
                && row.try_get::<String, _>("parent_workflow_id")? == invocation.parent.workflow_id
                && row.try_get::<String, _>("parent_workflow_run_id")? == invocation.parent.run_id
                && row.try_get::<uuid::Uuid, _>("run_id")? == invocation.canonical_run_id
                && row.try_get::<String, _>("tenant_id")? == invocation.command.tenant_id
                && row.try_get::<String, _>("context_key")? == invocation.command.context_key
                && row.try_get::<String, _>("keyword_key")? == invocation.command.keyword_key
                && row.try_get::<Vec<u8>, _>("command_bytes")? == actual_command,
            "operator admission does not pin exact server child, parent, tenant and command bytes"
            );
            Ok(())
        })()
        .map_err(AdmissionProbeError::Denied)?;
        Ok(true)
    }

    async fn materialize_with_server_info(
        &self,
        ctx: ActivityContext,
        invocation: QdrantKeywordChildInvocationV1,
    ) -> Result<QdrantKeywordChildResultV1, ActivityError> {
        if !self
            .matching_server_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)?
        {
            return Err(terminal_activity_error(
                "native Qdrant child has no independently issued operator admission",
            ));
        }
        // An ambiguous provider RPC can only reconcile by readback; SQL
        // forbids a second attempt. Never retry errors after this boundary.
        let outcome =
            execute_admitted_qdrant_projection_v1(&self.pool, invocation.command.admission_id)
                .await
                .map_err(terminal_activity_error)?;
        let outcome = match outcome {
            QdrantProjectionOutcomeV1::Applied => QdrantChildOutcomeV1::Applied,
            QdrantProjectionOutcomeV1::Reconciled => QdrantChildOutcomeV1::Reconciled,
            QdrantProjectionOutcomeV1::AlreadyConfirmed => QdrantChildOutcomeV1::AlreadyConfirmed,
        };
        Ok(QdrantKeywordChildResultV1 {
            schema_revision: 1,
            admission_id: invocation.command.admission_id,
            outcome,
        })
    }
}

fn worker_database_url() -> Result<String> {
    let inline = std::env::var("QDRANT_EFFECT_WORKER_DATABASE_URL").ok();
    let file = std::env::var("QDRANT_EFFECT_WORKER_DATABASE_URL_FILE").ok();
    let value = match (inline, file) {
        (Some(_), Some(_)) => anyhow::bail!("choose one Qdrant worker DB credential source"),
        (Some(url), None) => url,
        (None, Some(path)) => {
            let path = std::path::Path::new(&path);
            anyhow::ensure!(
                path.is_absolute(),
                "Qdrant worker DSN path must be absolute"
            );
            let meta = std::fs::symlink_metadata(path)?;
            anyhow::ensure!(
                meta.file_type().is_file(),
                "Qdrant worker DSN must be a regular non-symlink file"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                anyhow::ensure!(
                    meta.permissions().mode() & 0o077 == 0,
                    "Qdrant worker DSN secret requires 0600/0400 permissions"
                );
            }
            std::fs::read_to_string(path)?
                .trim_end_matches(|c| c == '\r' || c == '\n')
                .to_owned()
        }
        (None, None) => anyhow::bail!("dedicated Qdrant worker DSN secret required"),
    };
    anyhow::ensure!(
        !value.is_empty() && value.trim() == value,
        "Qdrant worker DSN has unsafe whitespace"
    );
    Ok(value)
}

async fn assert_effect_worker_role(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT r.rolsuper, r.rolbypassrls,
                has_table_privilege(session_user,
                  'pipeline.qdrant_projection_v1_admissions','INSERT') AS may_admit,
                pipeline.current_session_tenant_v1() AS tenant
         FROM pg_roles r WHERE r.rolname=session_user",
    )
    .fetch_one(pool)
    .await?;
    anyhow::ensure!(
        !row.try_get::<bool, _>("rolsuper")?
            && !row.try_get::<bool, _>("rolbypassrls")?
            && !row.try_get::<bool, _>("may_admit")?
            && row.try_get::<Option<String>, _>("tenant")?.is_some(),
        "Qdrant Activity role must be tenant-bound and must not issue admissions/bypass RLS"
    );
    Ok(())
}

async fn replay_history(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|e| anyhow::anyhow!("decode real Qdrant Temporal History: {e}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_string())
        .task_queue(QDRANT_OWNER_COMMAND_QUEUE_V1.to_string())
        .build();
    options.register_workflow::<QdrantKeywordProjectionV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|e| anyhow::anyhow!("construct Qdrant replayer: {e}"))?
        .replay_workflow(history)
        .await
        .map_err(|e| anyhow::anyhow!("Qdrant History replay failed: {e}"))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if let Some(arg) = args.next() {
        if arg == std::ffi::OsStr::new("--replay-history") {
            let file = args
                .next()
                .context("--replay-history requires real History JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected replay arguments");
            replay_history(std::path::Path::new(&file)).await?;
            println!("CI_TEMPORAL_QDRANT_EFFECT_REPLAY=PASS");
            return Ok(());
        }
        if arg == std::ffi::OsStr::new("--replay-history-v2") {
            let file = args
                .next()
                .context("--replay-history-v2 requires real History JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected replay arguments");
            owner_v2::replay_history(std::path::Path::new(&file)).await?;
            println!("CI_TEMPORAL_QDRANT_EFFECT_V2_REPLAY=PASS");
            return Ok(());
        }
        if arg == std::ffi::OsStr::new("--replay-history-generic-v1") {
            let file = args
                .next()
                .context("--replay-history-generic-v1 requires real History JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected generic replay arguments");
            owner_generic_v1::replay_history(std::path::Path::new(&file)).await?;
            println!("CI_TEMPORAL_QDRANT_GENERIC_OWNER_V1_REPLAY=PASS");
            return Ok(());
        }
        if arg == std::ffi::OsStr::new("--operator-issue") {
            let file = args
                .next()
                .context("--operator-issue requires request JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected operator arguments");
            return operator::issue_from_file(std::path::Path::new(&file)).await;
        }
        if arg == std::ffi::OsStr::new("--operator-resignal") {
            let file = args
                .next()
                .context("--operator-resignal requires original locator JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected operator arguments");
            return operator::resignal_from_file(std::path::Path::new(&file)).await;
        }
        if arg == std::ffi::OsStr::new("--operator-issue-v2") {
            let file = args
                .next()
                .context("--operator-issue-v2 requires exact-run locator JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected V2 operator arguments");
            return operator_v2::issue_from_file(std::path::Path::new(&file)).await;
        }
        if arg == std::ffi::OsStr::new("--operator-resignal-v2") {
            let file = args
                .next()
                .context("--operator-resignal-v2 requires exact-run locator JSON")?;
            anyhow::ensure!(args.next().is_none(), "unexpected V2 operator arguments");
            return operator_v2::resignal_from_file(std::path::Path::new(&file)).await;
        }
        if arg == std::ffi::OsStr::new("--operator-issue-v3") {
            let file = args
                .next()
                .context("--operator-issue-v3 requires exact-run locator JSON")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected generic operator arguments"
            );
            return operator_v3::issue_from_file(std::path::Path::new(&file)).await;
        }
        if arg == std::ffi::OsStr::new("--operator-resignal-v3") {
            let file = args
                .next()
                .context("--operator-resignal-v3 requires exact-run locator JSON")?;
            anyhow::ensure!(
                args.next().is_none(),
                "unexpected generic resignal arguments"
            );
            return operator_v3::resignal_from_file(std::path::Path::new(&file)).await;
        }
        anyhow::bail!("unsupported temporal_qdrant_effects argument: {arg:?}");
    }
    if std::env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1").as_deref() == Ok("true") {
        return owner_generic_v1::run_worker().await;
    }
    anyhow::ensure!(
        std::env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_V1").as_deref() == Ok("true"),
        "experimental native Qdrant owner is disabled"
    );
    anyhow::ensure!(
        std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1").as_deref() == Ok("1"),
        "underlying Qdrant provider pilot is disabled"
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let pool = PgPool::connect(&worker_database_url()?)
        .await
        .context("connect dedicated Qdrant Activity PostgreSQL LOGIN")?;
    assert_effect_worker_role(&pool).await?;
    let runtime = Runtime::from_current_tokio(
        RuntimeOptions::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("{e}"))?,
    )?;
    let url = std::env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".into());
    let namespace = std::env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| "default".into());
    let client = connect_client(
        &url,
        format!("alegria-qdrant-owner-v1:{}", std::process::id()),
        &namespace,
    )
    .await?;
    let mut command = WorkerOptions::new(QDRANT_OWNER_COMMAND_QUEUE_V1).build();
    command.register_workflow::<QdrantKeywordProjectionV1Workflow>()?;
    let mut command_worker =
        Worker::new(&runtime, client.clone(), command).map_err(|e| anyhow::anyhow!("{e}"))?;
    let activities = WorkerOptions::new(QDRANT_OWNER_ACTIVITY_QUEUE_V1)
        .register_activities(QdrantProjectionActivities {
            pool: Arc::new(pool.clone()),
        })
        .build();
    let mut activity_worker =
        Worker::new(&runtime, client.clone(), activities).map_err(|e| anyhow::anyhow!("{e}"))?;
    if std::env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_V2").as_deref() == Ok("true") {
        let can_admit_v2: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(session_user,
                'pipeline.qdrant_projection_v2_admissions','INSERT')",
        )
        .fetch_one(&pool)
        .await
        .context("V2 SQL schema and least-privileged Activity LOGIN must be ready")?;
        anyhow::ensure!(
            !can_admit_v2,
            "Qdrant V2 Activity cannot hold operator INSERT privilege"
        );
        let mut command_opts =
            WorkerOptions::new(qdrant_contracts::QDRANT_OWNER_COMMAND_QUEUE_V2).build();
        owner_v2::register_command(&mut command_opts)?;
        let mut command_worker_v2 = Worker::new(&runtime, client.clone(), command_opts)
            .map_err(|err| anyhow::anyhow!("Qdrant V2 command Worker: {err}"))?;
        let activities_v2 = WorkerOptions::new(qdrant_contracts::QDRANT_OWNER_ACTIVITY_QUEUE_V2)
            .register_activities(owner_v2::QdrantV2Activities {
                pool: Arc::new(pool),
            })
            .build();
        let mut activity_worker_v2 = Worker::new(&runtime, client, activities_v2)
            .map_err(|err| anyhow::anyhow!("Qdrant V2 Activity Worker: {err}"))?;
        tokio::try_join!(
            command_worker.run(),
            activity_worker.run(),
            command_worker_v2.run(),
            activity_worker_v2.run(),
        )?;
    } else {
        tokio::try_join!(command_worker.run(), activity_worker.run())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_only_read_only_storage_failure_never_identity_or_provider_failure() {
        let lost_connection = AdmissionProbeError::Storage(sqlx::Error::Io(std::io::Error::other(
            "PostgreSQL connection reset before admission SELECT",
        )));
        match admission_probe_activity_error(lost_connection) {
            ActivityError::Application(failure) => assert!(!failure.is_non_retryable()),
            other => panic!("expected retryable SQL transport failure, got {other:?}"),
        }

        let forged_identity = AdmissionProbeError::Denied(anyhow::anyhow!(
            "server-origin child identity differs from immutable admission"
        ));
        match admission_probe_activity_error(forged_identity) {
            ActivityError::Application(failure) => assert!(failure.is_non_retryable()),
            other => panic!("expected permanently rejected identity, got {other:?}"),
        }

        match terminal_activity_error("unknown outcome after Qdrant provider RPC") {
            ActivityError::Application(failure) => assert!(failure.is_non_retryable()),
            other => panic!("expected fenced provider ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn admission_retry_sqlstates_are_explicitly_bounded() {
        for sqlstate in [
            "08006", "08001", "40001", "40003", "40P01", "55P03", "53300", "57P01",
        ] {
            assert!(sqlstate_allows_admission_probe_retry(sqlstate));
        }
        for sqlstate in [
            "AV001", "42501", "42P01", "42703", "23505", "22001", "XX000",
        ] {
            assert!(
                !sqlstate_allows_admission_probe_retry(sqlstate),
                "permanent SQLSTATE {sqlstate} must not be retried"
            );
        }
        let no_permission = classify_admission_sql_error(sqlx::Error::RowNotFound);
        match admission_probe_activity_error(no_permission) {
            ActivityError::Application(failure) => assert!(failure.is_non_retryable()),
            other => panic!("expected terminal failed read, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn closed_postgres_pool_is_retryable_without_a_provider_attempt() {
        let pool = PgPool::connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("parse PostgreSQL URL without opening a network connection");
        pool.close().await;
        let error = sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .expect_err("closed PostgreSQL pool cannot perform read-only query");
        match admission_probe_activity_error(classify_admission_sql_error(error)) {
            ActivityError::Application(failure) => assert!(
                !failure.is_non_retryable(),
                "transient PostgreSQL pool shutdown must remain retryable"
            ),
            other => panic!("expected retryable PostgreSQL transport error, got {other:?}"),
        }
    }
}
