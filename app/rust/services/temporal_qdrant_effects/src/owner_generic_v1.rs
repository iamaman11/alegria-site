//! Shared native Temporal owner wire, implemented independently by Qdrant.
//! The common Workflow Type does not imply a common Worker, credential or
//! provider runtime. Only externally admitted Qdrant commands may execute;
//! upstream Produced inputs fail closed until independent provenance proof.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{ensure, Context, Result};
use operation_protocol::owner_dispatch::{
    owner_child_workflow_id, owner_child_workflow_id_v2, OwnerInvocationV1, OwnerResultV1,
    OWNER_OPERATION_CHILD_TYPE_V1,
};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, Operation, OwnerValidationRequestV1,
    OwnerValidationResultV1, PortId,
};
use qdrant_contracts::{
    confirmed_qdrant_owner_result, verify_qdrant_owner_invocation, QdrantChildOutcomeV1,
    QdrantKeywordProjectionCommandV1, QdrantKeywordProjectionContractV2,
    QdrantProducedVectorCommandV1, QdrantProducedVectorProjectionContractV1,
    QDRANT_EFFECT_INPUT_PORT_V2, QDRANT_GENERIC_ACTIVITY_QUEUE_V1, QDRANT_GENERIC_OWNER_QUEUE_V1,
    QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1,
};
use qdrant_effects_core::{
    execute_admitted_qdrant_projection_v3, execute_admitted_qdrant_projection_v4,
    QdrantProjectionOutcomeV1,
};
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
        verify_qdrant_owner_invocation(&invocation).map_err(|error| {
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

pub(super) fn validate_qdrant_owner_pre_effect_v1(
    request: &OwnerValidationRequestV1,
) -> Result<()> {
    request.validate_shape()?;
    let external_contract = QdrantKeywordProjectionContractV2::contract();
    let produced_contract = QdrantProducedVectorProjectionContractV1::contract();
    let contract = if request.operation == external_contract.reference {
        external_contract
    } else if request.operation == produced_contract.reference {
        produced_contract
    } else {
        anyhow::bail!("Qdrant ValidateChild refuses foreign operation");
    };
    ensure!(
        request.owner == contract.owner
            && request.contract_hash == contract.fingerprint()?
            && request.input_schemas == contract.inputs,
        "Qdrant ValidateChild contract/owner/input schema differs from deployed code"
    );
    if request.operation == QdrantKeywordProjectionContractV2::contract().reference {
        let port = PortId::from(QDRANT_EFFECT_INPUT_PORT_V2);
        ensure!(
            request.produced_input_ports.is_empty() && request.external_inputs.len() == 1,
            "Qdrant External ValidateChild cannot contain Produced or hidden inputs"
        );
        let input = request
            .external_inputs
            .get(&port)
            .context("Qdrant ValidateChild missing immutable External input")?;
        ensure!(
            input.payload == request.command_bytes,
            "Qdrant ValidateChild command differs from exact admitted External bytes"
        );
        let command: QdrantKeywordProjectionCommandV1 =
            serde_json::from_slice(&request.command_bytes)?;
        command.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            command.tenant_id == request.scope.tenant_id,
            "Qdrant ValidateChild command belongs to foreign tenant"
        );
    } else {
        ensure!(
            request.external_inputs.is_empty()
                && request.produced_input_ports.len() == 1
                && request
                    .produced_input_ports
                    .contains(&PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1)),
            "Qdrant Produced ValidateChild must reference one future typed vector"
        );
        let command: QdrantProducedVectorCommandV1 =
            serde_json::from_slice(&request.command_bytes)?;
        ensure!(
            command.schema_revision == 1 && !command.admission_id.is_nil(),
            "Qdrant Produced ValidateChild rejects invalid static admission command"
        );
    }
    Ok(())
}

/// Pure same-owner preflight. Never executes a provider Activity or SQL.
#[workflow]
struct AlegriaOwnerValidateV1Workflow {
    request: OwnerValidationRequestV1,
}

#[workflow_methods]
impl AlegriaOwnerValidateV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, request: OwnerValidationRequestV1) -> Self {
        Self { request }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerValidationResultV1> {
        let request = ctx.state(|state| state.request.clone());
        validate_qdrant_owner_pre_effect_v1(&request).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(format!("Qdrant ValidateOnly denied plan: {error}")),
            ))
        })?;
        OwnerValidationResultV1::accepted(&request).map_err(|error| {
            WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
                std::io::Error::other(format!("Qdrant ValidateOnly result: {error}")),
            ))
        })
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
        let command =
            verify_qdrant_owner_invocation(&invocation).map_err(terminal_activity_error)?;
        let run_v2 = invocation.parent.workflow_id
            == operation_dag_parent_workflow_id_v2(&invocation.checked_dispatch.scope.run_id);
        let outcome = if run_v2 {
            execute_admitted_qdrant_projection_v4(
                &self.pool,
                command.admission_id,
                &invocation.checked_dispatch.execution_key,
            )
            .await
        } else {
            execute_admitted_qdrant_projection_v3(
                &self.pool,
                command.admission_id,
                &invocation.checked_dispatch.execution_key,
            )
            .await
        }
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
        let command = verify_qdrant_owner_invocation(invocation)
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
                        == if invocation.parent.workflow_id
                            == operation_dag_parent_workflow_id_v2(
                                &invocation.checked_dispatch.scope.run_id
                            )
                        {
                            owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key)
                        } else {
                            owner_child_workflow_id(&invocation.checked_dispatch.execution_key)
                        },
                "Activity is not executing for its fixed Qdrant generic child"
            );
            Ok((id, run_id))
        })()
        .map_err(AdmissionProbeError::Denied)?;
        let run_v2 = invocation.parent.workflow_id
            == operation_dag_parent_workflow_id_v2(&invocation.checked_dispatch.scope.run_id);
        let read_sql = if run_v2 {
            "SELECT child_workflow_id,child_workflow_run_id,parent_workflow_id,
                    parent_workflow_run_id,run_id,tenant_id,context_key,keyword_key,
                    operation_instance,plan_hash,dispatch_execution_key,command_bytes
               FROM pipeline.qdrant_projection_v4_admissions WHERE admission_id=$1"
        } else {
            "SELECT child_workflow_id,child_workflow_run_id,parent_workflow_id,
                    parent_workflow_run_id,run_id,tenant_id,context_key,keyword_key,
                    operation_instance,plan_hash,dispatch_execution_key,command_bytes
               FROM pipeline.qdrant_projection_v3_admissions WHERE admission_id=$1"
        };
        let row = sqlx::query(read_sql)
            .bind(command.admission_id)
            .fetch_optional(&*self.pool)
            .await
            .map_err(super::classify_admission_sql_error)?;
        let Some(row) = row else {
            return Ok(false);
        };
        let pinned_effect_bytes = if invocation.checked_dispatch.operation
            == QdrantProducedVectorProjectionContractV1::contract().reference
        {
            serde_json::to_vec(&command)
                .map_err(|error| AdmissionProbeError::Denied(anyhow::Error::from(error)))?
        } else {
            invocation.command_bytes.clone()
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
                    && row.try_get::<Vec<u8>, _>("command_bytes")? == pinned_effect_bytes,
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
    let v3_enabled =
        std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V3").as_deref() == Ok("1");
    let v4_enabled =
        std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V4").as_deref() == Ok("1");
    let row = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,
                pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','SELECT') AS v3_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','INSERT') AS v3_insert,
                COALESCE(has_table_privilege(session_user,
                    to_regclass('pipeline.qdrant_projection_v4_admissions'),'SELECT'),FALSE) AS v4_read,
                COALESCE(has_table_privilege(session_user,
                    to_regclass('pipeline.qdrant_projection_v4_admissions'),'INSERT'),FALSE) AS v4_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','INSERT') AS v2_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','INSERT') AS v1_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','SELECT') AS v2_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','SELECT') AS v1_read,
                has_table_privilege(session_user,'pipeline.vector_projection_sources_v1','SELECT') AS producer_source_read,
                has_table_privilege(session_user,'pipeline.operation_artifact_receipts_v1','SELECT') AS cross_owner_receipt_read,
                has_function_privilege(session_user,
                    'pipeline.operation_artifact_receipt_matches_v1(uuid,text,uuid,text,text,text,text,text,text,text,text,text,text,jsonb)','EXECUTE') AS cross_owner_cert_verify,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.operation_artifact_receipt_matches_v2(uuid,text,uuid,text,text,text,text,text,text,text,text,text,text,jsonb)'),
                    'EXECUTE'),FALSE) AS cross_owner_cert_verify_v2,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS begin_allowed,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS confirm_allowed,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.begin_qdrant_projection_v4(uuid,bytea,text)'),
                    'EXECUTE'),FALSE) AS v4_begin,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.confirm_qdrant_projection_v4(uuid,bytea,text)'),
                    'EXECUTE'),FALSE) AS v4_confirm,
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
            && (!v3_enabled || row.try_get::<bool, _>("v3_read")?)
            && (!v4_enabled || row.try_get::<bool, _>("v4_read")?)
            && !row.try_get::<bool, _>("v3_insert")?
            && !row.try_get::<bool, _>("v4_insert")?
            && !row.try_get::<bool, _>("v2_insert")?
            && !row.try_get::<bool, _>("v1_insert")?
            && !row.try_get::<bool, _>("v2_read")?
            && !row.try_get::<bool, _>("v1_read")?
            && !row.try_get::<bool, _>("producer_source_read")?
            && !row.try_get::<bool, _>("cross_owner_receipt_read")?
            && !row.try_get::<bool, _>("cross_owner_cert_verify")?
            && !row.try_get::<bool, _>("cross_owner_cert_verify_v2")?
            && !row.try_get::<bool, _>("v2_begin")?
            && !row.try_get::<bool, _>("v2_confirm")?
            && !row.try_get::<bool, _>("v1_begin")?
            && !row.try_get::<bool, _>("v1_confirm")?
            && (!v3_enabled
                || (row.try_get::<bool, _>("begin_allowed")?
                    && row.try_get::<bool, _>("confirm_allowed")?))
            && (!v4_enabled
                || (row.try_get::<bool, _>("v4_begin")?
                    && row.try_get::<bool, _>("v4_confirm")?))
            && row.try_get::<Option<String>, _>("tenant")?.as_deref()
                == Some(std::env::var("QDRANT_GENERIC_OWNER_TENANT_ID")?.as_str()),
        "generic Qdrant Activity must have tenant-bound read/execute, no admission authority"
    );
    Ok(())
}

pub(crate) async fn run_worker() -> Result<()> {
    ensure!(
        std::env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1").as_deref() == Ok("true")
            && (std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V3").as_deref() == Ok("1")
                || std::env::var("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V4").as_deref()
                    == Ok("1")),
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
    command.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
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
        .namespace("default".to_owned())
        .task_queue(QDRANT_GENERIC_OWNER_QUEUE_V1.to_owned())
        .build();
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

#[cfg(test)]
mod owner_validate_tests {
    use super::*;
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, OperationInstanceId, PlanHash, RunScope,
        TemporalExecutionIdentityV4,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use uuid::Uuid;

    fn valid_produced_preflight() -> OwnerValidationRequestV1 {
        let contract = QdrantProducedVectorProjectionContractV1::contract();
        let contract_hash = contract.fingerprint().unwrap();
        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: Uuid::new_v4().to_string(),
        };
        OwnerValidationRequestV1 {
            schema_revision: 1,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: operation_dag_parent_workflow_id_v2(&scope.run_id),
                run_id: Uuid::new_v4().to_string(),
            },
            plan_hash: PlanHash("b".repeat(64)),
            scope,
            instance: OperationInstanceId::from("qdrant-new-projection"),
            operation: contract.reference,
            owner: contract.owner,
            contract_hash,
            command_bytes: serde_json::to_vec(&QdrantProducedVectorCommandV1 {
                schema_revision: 1,
                admission_id: Uuid::new_v4(),
            })
            .unwrap(),
            input_schemas: contract.inputs,
            external_inputs: BTreeMap::new(),
            produced_input_ports: BTreeSet::from([PortId::from(
                QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1,
            )]),
        }
    }

    #[test]
    fn validates_only_own_concrete_produced_command_before_effect() {
        let original = valid_produced_preflight();
        assert!(validate_qdrant_owner_pre_effect_v1(&original).is_ok());
        let mut forged = valid_produced_preflight();
        forged.command_bytes = b"{\"schema_revision\":1,\"admission_id\":\"bad\"}".to_vec();
        assert!(validate_qdrant_owner_pre_effect_v1(&forged).is_err());
        let mut foreign = valid_produced_preflight();
        foreign.owner = operation_protocol::OperationOwnerId::from("postgres_store");
        assert!(validate_qdrant_owner_pre_effect_v1(&foreign).is_err());
        let mut unknown = valid_produced_preflight();
        unknown.produced_input_ports.clear();
        assert!(validate_qdrant_owner_pre_effect_v1(&unknown).is_err());
    }
}
