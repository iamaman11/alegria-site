//! Opt-in V2 Qdrant owner for independently attested mixed-owner DAG runs.
//!
//! V1 Histories, task queues and operator-issued SQL rows remain frozen.
//! V2 never consumes a V1 admission or retries an ambiguous provider write.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use operation_protocol::{IdempotencyPolicy, Operation};
use qdrant_contracts::{
    QdrantChildOutcomeV1, QdrantKeywordChildInvocationV2, QdrantKeywordChildResultV2,
    QdrantKeywordProjectionContractV2, QDRANT_CHILD_SCHEMA_REVISION_V2,
    QDRANT_OWNER_ACTIVITY_QUEUE_V2, QDRANT_OWNER_CHILD_TYPE_V2, QDRANT_OWNER_COMMAND_QUEUE_V2,
};
use qdrant_effects_core::{execute_admitted_qdrant_projection_v2, QdrantProjectionOutcomeV1};
use sqlx::{PgPool, Row};
use temporalio_client::WorkflowHistory;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, ApplicationFailure, SyncWorkflowContext, WorkerOptions,
    WorkflowContext, WorkflowContextView, WorkflowResult, WorkflowTermination,
};

use super::{admission_probe_activity_error, terminal_activity_error, AdmissionProbeError};

fn admission_probe_options() -> ActivityOptions {
    let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(15));
    options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
        schedule_to_close: Duration::from_secs(300),
        start_to_close: Duration::from_secs(15),
    };
    options.task_queue = Some(QDRANT_OWNER_ACTIVITY_QUEUE_V2.to_owned());
    options
}

fn materialize_options() -> ActivityOptions {
    let mut options = ActivityOptions::schedule_to_close_timeout(Duration::from_secs(300));
    options.task_queue = Some(QDRANT_OWNER_ACTIVITY_QUEUE_V2.to_owned());
    options
}

#[workflow]
pub(crate) struct QdrantKeywordProjectionV2Workflow {
    immutable_invocation: QdrantKeywordChildInvocationV2,
    admission_signals_received: u64,
}

#[workflow_methods]
impl QdrantKeywordProjectionV2Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: QdrantKeywordChildInvocationV2) -> Self {
        Self {
            immutable_invocation: invocation,
            admission_signals_received: 0,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<QdrantKeywordChildResultV2> {
        let invocation = ctx.state(|s| s.immutable_invocation.clone());
        invocation.verify_integrity().map_err(|err| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(format!("invalid typed Qdrant V2 invocation: {err}")),
            ))
        })?;
        let mut observed = 0;
        loop {
            let previous = observed;
            ctx.wait_condition(move |s| s.admission_signals_received > previous)
                .await?;
            observed = ctx.state(|s| s.admission_signals_received);
            let admitted: bool = ctx
                .start_activity(
                    QdrantV2Activities::check_operator_admission_v2,
                    invocation.clone(),
                    admission_probe_options(),
                )
                .await?;
            if admitted {
                break;
            }
        }
        let result: QdrantKeywordChildResultV2 = ctx.start_activity(
            QdrantV2Activities::materialize_admitted_keyword_v2,
            invocation,
            materialize_options(),
        )
        .await?;
        Ok(result)
    }

    #[signal(name = "operator_admission_ready_qdrant_v2")]
    fn operator_admission_ready_qdrant_v2(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.admission_signals_received = self.admission_signals_received.saturating_add(1);
    }
}

pub(crate) struct QdrantV2Activities {
    pub(crate) pool: Arc<PgPool>,
}

#[activities]
impl QdrantV2Activities {
    #[activity]
    pub(crate) async fn check_operator_admission_v2(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: QdrantKeywordChildInvocationV2,
    ) -> Result<bool, ActivityError> {
        // Only a read-only probe may retry after an outage. The forged Signal
        // has no authority and never changes SQL admission state.
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.matches_server_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)
    }

    #[activity]
    pub(crate) async fn materialize_admitted_keyword_v2(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: QdrantKeywordChildInvocationV2,
    ) -> Result<QdrantKeywordChildResultV2, ActivityError> {
        if !self
            .matches_server_admission(&ctx, &invocation)
            .await
            .map_err(admission_probe_activity_error)?
        {
            return Err(terminal_activity_error(
                "Qdrant V2 owner has no independently issued exact DAG operator admission",
            ));
        }
        let outcome = execute_admitted_qdrant_projection_v2(
            &self.pool,
            invocation.command.admission_id,
            &invocation.checked_dispatch.execution_key,
        )
        .await
        .map_err(terminal_activity_error)?;
        Ok(QdrantKeywordChildResultV2 {
            schema_revision: QDRANT_CHILD_SCHEMA_REVISION_V2,
            admission_id: invocation.command.admission_id,
            dispatch_execution_key: invocation.checked_dispatch.execution_key,
            outcome: match outcome {
                QdrantProjectionOutcomeV1::Applied => QdrantChildOutcomeV1::Applied,
                QdrantProjectionOutcomeV1::Reconciled => QdrantChildOutcomeV1::Reconciled,
                QdrantProjectionOutcomeV1::AlreadyConfirmed => {
                    QdrantChildOutcomeV1::AlreadyConfirmed
                }
            },
        })
    }
}

impl QdrantV2Activities {
    async fn matches_server_admission(
        &self,
        ctx: &ActivityContext,
        invocation: &QdrantKeywordChildInvocationV2,
    ) -> std::result::Result<bool, AdmissionProbeError> {
        let info = ctx.info();
        let (actual_id, actual_run_id) = (|| -> Result<(&str, &str)> {
            let id = info
                .workflow_id
                .as_deref()
                .context("missing V2 child Workflow ID")?;
            let run_id = info
                .workflow_run_id
                .as_deref()
                .context("missing V2 child Run ID")?;
            anyhow::ensure!(
                info.workflow_type.as_deref() == Some(QDRANT_OWNER_CHILD_TYPE_V2)
                    && info.task_queue == QDRANT_OWNER_ACTIVITY_QUEUE_V2
                    && id
                        == format!(
                            "alegria-qdrant-effect-v2-{}",
                            invocation.command.admission_id
                        ),
                "Qdrant V2 Activity is outside its pinned child Workflow Type/Queue"
            );
            invocation.verify_integrity()?;
            let expected = QdrantKeywordProjectionContractV2::contract();
            anyhow::ensure!(
                invocation.checked_dispatch.operation == expected.reference
                    && invocation.checked_dispatch.owner == expected.owner
                    && invocation.checked_dispatch.effects == expected.effects
                    && invocation.checked_dispatch.idempotency == IdempotencyPolicy::Idempotent,
                "Qdrant V2 cannot execute a foreign owner, operation revision, or effect set"
            );
            Ok((id, run_id))
        })()
        .map_err(AdmissionProbeError::Denied)?;
        let row = sqlx::query(
            "SELECT child_workflow_id,child_workflow_run_id,parent_workflow_id,
                    parent_workflow_run_id,run_id,tenant_id,context_key,keyword_key,
                    operation_instance,plan_hash,dispatch_execution_key,command_bytes
               FROM pipeline.qdrant_projection_v2_admissions WHERE admission_id=$1",
        )
        .bind(invocation.command.admission_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(super::classify_admission_sql_error)?;
        let Some(row) = row else {
            return Ok(false);
        };
        (|| -> Result<()> {
            anyhow::ensure!(
                row.try_get::<String, _>("child_workflow_id")? == actual_id
                    && row.try_get::<String, _>("child_workflow_run_id")? == actual_run_id
                    && row.try_get::<String, _>("parent_workflow_id")? == invocation.parent.workflow_id
                    && row.try_get::<String, _>("parent_workflow_run_id")? == invocation.parent.run_id
                    && row.try_get::<uuid::Uuid, _>("run_id")? == invocation.canonical_run_id
                    && row.try_get::<String, _>("tenant_id")? == invocation.command.tenant_id
                    && row.try_get::<String, _>("context_key")? == invocation.command.context_key
                    && row.try_get::<String, _>("keyword_key")? == invocation.command.keyword_key
                    && row.try_get::<String, _>("operation_instance")? == invocation.checked_dispatch.instance.0
                    && row.try_get::<String, _>("plan_hash")? == invocation.checked_dispatch.plan_hash.0
                    && row.try_get::<String, _>("dispatch_execution_key")? == invocation.checked_dispatch.execution_key
                    && row.try_get::<Vec<u8>, _>("command_bytes")? == serde_json::to_vec(&invocation.command)?,
                "V2 admission does not pin actual child, parent, tenant, command and checked DAG dispatch"
            );
            Ok(())
        })().map_err(AdmissionProbeError::Denied)?;
        Ok(true)
    }
}

pub(crate) fn register_command(opts: &mut WorkerOptions) -> Result<()> {
    opts.register_workflow::<QdrantKeywordProjectionV2Workflow>()?;
    Ok(())
}

pub(crate) async fn replay_history(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|err| anyhow::anyhow!("decode real Qdrant V2 History: {err}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_owned())
        .task_queue(QDRANT_OWNER_COMMAND_QUEUE_V2.to_owned())
        .build();
    options.register_workflow::<QdrantKeywordProjectionV2Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|err| anyhow::anyhow!("construct Qdrant V2 SDK replayer: {err}"))?
        .replay_workflow(history)
        .await
        .map_err(|err| anyhow::anyhow!("Qdrant V2 SDK History replay failed: {err}"))?;
    Ok(())
}
