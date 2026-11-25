//! Shared native Temporal owner wire, implemented independently by Qdrant.
//! The common Workflow Type does not imply a common Worker, credential or
//! provider runtime. Only externally admitted Qdrant commands may execute;
//! upstream Produced inputs fail closed until independent provenance proof.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{ensure, Context, Result};
use operation_protocol::owner_dispatch::{
    OwnerInvocationV1, OwnerResultV1, OWNER_OPERATION_CHILD_TYPE_V1,
};
use qdrant_contracts::{
    confirmed_qdrant_owner_result, verify_external_qdrant_owner_invocation, QdrantChildOutcomeV1,
    QDRANT_GENERIC_ACTIVITY_QUEUE_V1, QDRANT_GENERIC_OWNER_QUEUE_V1,
};
use qdrant_effects_core::{execute_admitted_qdrant_projection_v3, QdrantProjectionOutcomeV1};
use sqlx::{PgPool, Row};
use temporalio_client::WorkflowHistory;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, ApplicationFailure, Runtime, SyncWorkflowContext,
    Worker, WorkerOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
    WorkflowTermination,
};

use super::{admission_probe_activity_error, terminal_activity_error, AdmissionProbeError};

fn probe_options() -> ActivityOptions {
    let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(15));
    options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
        schedule_to_close: Duration::from_secs(300),
        start_to_close: Duration::from_secs(15),
    };
    options.task_queue = Some(QDRANT_GENERIC_ACTIVITY_QUEUE_V1.to_owned());
    options
}

fn effect_options() -> ActivityOptions {
    let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
    options.task_queue = Some(QDRANT_GENERIC_ACTIVITY_QUEUE_V1.to_owned());
    options
}

#[workflow]
struct AlegriaOwnerOperationV1Workflow {
    immutable_invocation: OwnerInvocationV1,
    wakeup_count: u64,
}

#[workflow_methods]
impl AlegriaOwnerOperationV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: OwnerInvocationV1) -> Self {
        Self {
            immutable_invocation: invocation,
            wakeup_count: 0,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerResultV1> {
        let invocation = ctx.state(|s| s.immutable_invocation.clone());
        verify_external_qdrant_owner_invocation(&invocation).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(format!(
                    "Qdrant owner rejected foreign/Produced input: {error}"
                )),
            ))
        })?;
        let mut observed = 0;
        loop {
            let previous = observed;
            ctx.wait_condition(move |s| s.wakeup_count > previous)
                .await?;
            observed = ctx.state(|s| s.wakeup_count);
            let admitted: bool = ctx
                .start_activity(
                    QdrantGenericActivities::check_operator_admission_generic_v1,
                    invocation.clone(),
                    probe_options(),
                )
                .await?;
            if admitted {
                break;
            }
        }
        let receipt: OwnerResultV1 = ctx
            .start_activity(
                QdrantGenericActivities::materialize_admitted_qdrant_generic_v1,
                invocation,
                effect_options(),
            )
            .await?;
        Ok(receipt)
    }

    #[signal(name = "operator_admission_ready_qdrant_generic_v1")]
    fn operator_admission_ready_qdrant_generic_v1(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.wakeup_count = self.wakeup_count.saturating_add(1);
    }
}

pub(crate) struct QdrantGenericActivities {
    pool: Arc<PgPool>,
}

#[activities]
impl QdrantGenericActivities {
    #[activity]
    pub(crate) async fn check_operator_admission_generic_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<bool, ActivityError> {
        // Forged/early Temporal signals are data, not authorization. Only a
        // read-only DB admission probe can retry after transient outages.
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.matches_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)
    }

    #[activity]
    pub(crate) async fn materialize_admitted_qdrant_generic_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<OwnerResultV1, ActivityError> {
        if !self
            .matches_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)?
        {
            return Err(terminal_activity_error(
                "generic Qdrant effect has no independently History-attested SQL admission",
            ));
        }
        let command = verify_external_qdrant_owner_invocation(&invocation)
            .map_err(terminal_activity_error)?;
        let outcome = execute_admitted_qdrant_projection_v3(
            &self.pool,
            command.admission_id,
            &invocation.checked_dispatch.execution_key,
        )
        .await
        .map_err(terminal_activity_error)?;
        confirmed_qdrant_owner_result(
            &invocation,
            &command,
            match outcome {
                QdrantProjectionOutcomeV1::Applied => QdrantChildOutcomeV1::Applied,
                QdrantProjectionOutcomeV1::Reconciled => QdrantChildOutcomeV1::Reconciled,
                QdrantProjectionOutcomeV1::AlreadyConfirmed => {
                    QdrantChildOutcomeV1::AlreadyConfirmed
                }
            },
        )
        .map_err(terminal_activity_error)
    }
}

impl QdrantGenericActivities {
    async fn matches_admission(
        &self,
        ctx: &ActivityContext,
        invocation: &OwnerInvocationV1,
    ) -> std::result::Result<bool, AdmissionProbeError> {
        let command = verify_external_qdrant_owner_invocation(invocation)
            .map_err(|error| AdmissionProbeError::Denied(anyhow::anyhow!(error)))?;
        let info = ctx.info();
        let (id, run_id) = (|| -> Result<(&str, &str)> {
            let id = info
                .workflow_id
                .as_deref()
                .context("missing generic Qdrant child ID")?;
            let run_id = info
                .workflow_run_id
                .as_deref()
                .context("missing generic Qdrant child Run ID")?;
            ensure!(
                info.workflow_type.as_deref() == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                    && info.task_queue == QDRANT_GENERIC_ACTIVITY_QUEUE_V1
                    && id
                        == format!(
                            "alegria-owner-operation-v1-{}",
                            invocation.checked_dispatch.execution_key
                        ),
                "Activity is not executing for its fixed Qdrant generic child"
            );
            Ok((id, run_id))
        })()
        .map_err(AdmissionProbeError::Denied)?;
        let row = sqlx::query(
            "SELECT child_workflow_id,child_workflow_run_id,parent_workflow_id,
                    parent_workflow_run_id,run_id,tenant_id,context_key,keyword_key,
                    operation_instance,plan_hash,dispatch_execution_key,command_bytes
               FROM pipeline.qdrant_projection_v3_admissions WHERE admission_id=$1",
        )
        .bind(command.admission_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(super::classify_admission_sql_error)?;
        let Some(row) = row else {
            return Ok(false);
        };
        (|| -> Result<()> {
            ensure!(
                row.try_get::<String, _>("child_workflow_id")? == id
                    && row.try_get::<String, _>("child_workflow_run_id")? == run_id
                    && row.try_get::<String, _>("parent_workflow_id")?
                        == invocation.parent.workflow_id
                    && row.try_get::<String, _>("parent_workflow_run_id")?
                        == invocation.parent.run_id
                    && row.try_get::<String, _>("tenant_id")? == command.tenant_id
                    && row.try_get::<String, _>("context_key")? == command.context_key
                    && row.try_get::<String, _>("keyword_key")? == command.keyword_key
                    && row.try_get::<uuid::Uuid, _>("run_id")?.to_string()
                        == invocation.checked_dispatch.scope.run_id
                    && row.try_get::<String, _>("operation_instance")?
                        == invocation.checked_dispatch.instance.as_str()
                    && row.try_get::<String, _>("plan_hash")?
                        == invocation.checked_dispatch.plan_hash.as_str()
                    && row.try_get::<String, _>("dispatch_execution_key")?
                        == invocation.checked_dispatch.execution_key
                    && row.try_get::<Vec<u8>, _>("command_bytes")? == invocation.command_bytes,
                "V3 SQL admission does not pin exact typed command, parent, child and checked DAG"
            );
            Ok(())
        })()
        .map_err(AdmissionProbeError::Denied)?;
        Ok(true)
    }
}

fn database_url_from_file() -> Result<String> {
    let filename = std::env::var("QDRANT_GENERIC_OWNER_DATABASE_URL_FILE")
        .context("generic Qdrant owner requires separate dedicated DB secret")?;
    let path = Path::new(&filename);
    ensure!(
        path.is_absolute(),
        "generic Qdrant DB secret path must be absolute"
    );
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_file(),
        "generic Qdrant DB secret must not be symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "generic Qdrant secret requires 0600/0400"
        );
    }
    let value = std::fs::read_to_string(path)?
        .trim_end_matches(|ch| ch == '\r' || ch == '\n')
        .to_owned();
    ensure!(
        !value.is_empty() && value.trim() == value,
        "malformed generic Qdrant DSN"
    );
    Ok(value)
}

async fn assert_generic_worker_role(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,
                pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','SELECT') AS v3_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','INSERT') AS v3_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','INSERT') AS v2_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','INSERT') AS v1_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','SELECT') AS v2_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','SELECT') AS v1_read,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS begin_allowed,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS confirm_allowed,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_confirm,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_confirm
           FROM pg_roles r WHERE r.rolname=session_user",
    ).fetch_one(pool).await?;
    ensure!(
        !row.try_get::<bool, _>("rolsuper")?
            && !row.try_get::<bool, _>("rolbypassrls")?
            && row.try_get::<bool, _>("v3_read")?
            && !row.try_get::<bool, _>("v3_insert")?
            && !row.try_get::<bool, _>("v2_insert")?
            && !row.try_get::<bool, _>("v1_insert")?
            && !row.try_get::<bool, _>("v2_read")?
            && !row.try_get::<bool, _>("v1_read")?
            && !row.try_get::<bool, _>("v2_begin")?
            && !row.try_get::<bool, _>("v2_confirm")?
            && !row.try_get::<bool, _>("v1_begin")?
            && !row.try_get::<bool, _>("v1_confirm")?
            && row.try_get::<bool, _>("begin_allowed")?
            && row.try_get::<bool, _>("confirm_allowed")?
            && row.try_get::<Option<String>, _>("tenant")?.as_deref()
                == Some(std::env::var("QDRANT_GENERIC_OWNER_TENANT_ID")?.as_str()),
        "generic Qdrant Activity must have tenant-bound read/execute, no admission authority"
    );
    Ok(())
}

pub(crate) async fn run_worker() -> Result<()> {
    ensure!(
        std::env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1").as_deref() == Ok("true")
            && std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V3").as_deref() == Ok("1"),
        "generic Qdrant owner requires two explicit independent opt-ins"
    );
    ensure!(
        std::env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_V1").as_deref() != Ok("true")
            && std::env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_V2").as_deref() != Ok("true"),
        "generic Qdrant owner must run with separate credentials and queues from legacy workers"
    );
    let pool = PgPool::connect(&database_url_from_file()?).await?;
    assert_generic_worker_role(&pool).await?;
    let runtime = Runtime::from_current_tokio(
        RuntimeOptions::builder()
            .build()
            .map_err(|error| anyhow::anyhow!("{error}"))?,
    )?;
    let url = std::env::var("TEMPORAL_URL")?;
    let namespace = std::env::var("TEMPORAL_NAMESPACE")?;
    ensure!(
        !url.trim().is_empty() && !namespace.trim().is_empty(),
        "explicit Temporal endpoint required"
    );
    let client = temporal_gateway::connect_client(
        &url,
        format!("alegria-qdrant-generic-owner:{}", std::process::id()),
        &namespace,
    )
    .await?;
    let mut command = WorkerOptions::new(QDRANT_GENERIC_OWNER_QUEUE_V1).build();
    command.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    let mut command_worker = Worker::new(&runtime, client.clone(), command)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let activities = WorkerOptions::new(QDRANT_GENERIC_ACTIVITY_QUEUE_V1)
        .register_activities(QdrantGenericActivities {
            pool: Arc::new(pool),
        })
        .build();
    let mut activity_worker =
        Worker::new(&runtime, client, activities).map_err(|error| anyhow::anyhow!("{error}"))?;
    tokio::try_join!(command_worker.run(), activity_worker.run())?;
    Ok(())
}

pub(crate) async fn replay_history(path: &Path) -> Result<()> {
    let history = WorkflowHistory::from_json(&std::fs::read(path)?)
        .map_err(|error| anyhow::anyhow!("invalid real Qdrant generic History: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".into())
        .task_queue(QDRANT_GENERIC_OWNER_QUEUE_V1.into())
        .build();
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}
