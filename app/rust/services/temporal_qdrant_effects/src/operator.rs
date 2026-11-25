//! Independently authenticated operator proof for the opt-in native Qdrant
//! child. Request IDs are LOCATORS, never self-asserted Temporal authority.
//! Only reciprocal server-sourced parent/child Histories can issue SQL admission.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    QdrantKeywordChildInvocationV1, QdrantProjectionRunV1,
    QDRANT_DAG_PARENT_QUEUE_V1, QDRANT_DAG_PARENT_TYPE_V1,
    QDRANT_OWNER_CHILD_TYPE_V1, QDRANT_OWNER_COMMAND_QUEUE_V1,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowDescribeOptions, WorkflowExecutionInfo,
    WorkflowFetchHistoryOptions, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{
    history_event::Attributes, History,
};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequest {
    admission_id: Uuid,
    canonical_run_id: Uuid,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    child_workflow_id: String,
    child_workflow_run_id: String,
    expires_in_seconds: u32,
}

struct Attested {
    invocation: QdrantKeywordChildInvocationV1,
    parent_input_bytes: Vec<u8>,
    command_bytes: Vec<u8>,
    parent_child_started_event_id: i64,
}

fn start_payload(history: &History, workflow_type: &str, queue: &str) -> Result<Vec<u8>> {
    let first = history.events.first().context("exact-run Temporal History has no start")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(start)) = &first.attributes else {
        bail!("first Temporal event was not WorkflowExecutionStarted");
    };
    ensure!(start.workflow_type.as_ref().map(|v| v.name.as_str()) == Some(workflow_type)
        && start.task_queue.as_ref().map(|v| v.name.as_str()) == Some(queue),
        "Temporal started on wrong native Workflow Type or Task Queue");
    let input = &start.input.as_ref().context("missing native Workflow input")?.payloads;
    ensure!(input.len() == 1 && input[0].data.len() <= 2 * 1024 * 1024,
        "Temporal native Workflow requires one bounded typed input");
    Ok(input[0].data.to_vec())
}

fn attest_pair(parent: &History, child: &History, request: &IssueRequest) -> Result<Attested> {
    let parent_bytes = start_payload(parent, QDRANT_DAG_PARENT_TYPE_V1,
        QDRANT_DAG_PARENT_QUEUE_V1)?;
    let child_bytes = start_payload(child, QDRANT_OWNER_CHILD_TYPE_V1,
        QDRANT_OWNER_COMMAND_QUEUE_V1)?;
    let run: QdrantProjectionRunV1 = serde_json::from_slice(&parent_bytes)
        .context("parent History did not start with QdrantProjectionRunV1")?;
    run.preflight().map_err(anyhow::Error::msg)?;
    ensure!(run.canonical_run_id == request.canonical_run_id,
        "parent History run differs from requested canonical run");
    let invocation: QdrantKeywordChildInvocationV1 = serde_json::from_slice(&child_bytes)
        .context("child History did not start with typed Qdrant invocation")?;
    invocation.validate().map_err(anyhow::Error::msg)?;
    ensure!(invocation.canonical_run_id == request.canonical_run_id
        && invocation.parent.workflow_id == request.parent_workflow_id
        && invocation.parent.run_id == request.parent_workflow_run_id
        && invocation.command.admission_id == request.admission_id
        && request.child_workflow_id == format!("alegria-qdrant-effect-v1-{}", request.admission_id),
        "child immutable identity/parent/run/admission is invalid");
    ensure!(
        run.commands.iter().filter(|command| *command == &invocation.command).count() == 1
            && invocation.command.tenant_id == run.tenant_id
            && invocation.command.context_key == run.context_key,
        "child command is not an exactly admitted parent History operation"
    );
    // Production SDK serializes strictly typed Workflow args; reserializing
    // must reproduce the bytes the Activity later binds to SQL. Do not insert
    // arbitrary caller-supplied JSON that differs from native child History.
    ensure!(serde_json::to_vec(&invocation)? == child_bytes,
        "child History input is not canonical serialized invocation");
    let child_started = child.events.first().context("child History empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &child_started.attributes
    else { bail!("child did not start with native Temporal start event"); };
    let parent_execution = attrs.parent_workflow_execution.as_ref()
        .context("child start has no server-origin parent execution")?;
    ensure!(parent_execution.workflow_id == request.parent_workflow_id
        && parent_execution.run_id == request.parent_workflow_run_id,
        "child server-origin parent differs from operator target");

    let mut child_started_id = None;
    for initiated in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(init)) = &initiated.attributes
        else { continue; };
        if init.workflow_id != request.child_workflow_id { continue; }
        ensure!(init.workflow_type.as_ref().map(|v|v.name.as_str()) == Some(QDRANT_OWNER_CHILD_TYPE_V1)
            && init.task_queue.as_ref().map(|v|v.name.as_str()) == Some(QDRANT_OWNER_COMMAND_QUEUE_V1),
            "parent requested unexpected Qdrant child Type/Queue");
        let init_payloads = &init.input.as_ref().context("parent initiated child without input")?.payloads;
        ensure!(init_payloads.len() == 1 && init_payloads[0].data.as_slice() == child_bytes.as_slice(),
            "parent child-initiation bytes differ from server child History");
        for started in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(start)) = &started.attributes
            else { continue; };
            if start.initiated_event_id != initiated.event_id { continue; }
            let actual = start.workflow_execution.as_ref()
                .context("parent ChildStarted omitted server child identity")?;
            ensure!(actual.workflow_id == request.child_workflow_id
                && actual.run_id == request.child_workflow_run_id
                && start.workflow_type.as_ref().map(|v|v.name.as_str()) == Some(QDRANT_OWNER_CHILD_TYPE_V1)
                && attrs.parent_initiated_event_id == initiated.event_id,
                "reciprocal native Temporal child start does not match both Histories");
            ensure!(child_started_id.replace(started.event_id).is_none(),
                "duplicate parent ChildStarted would make Temporal admission ambiguous");
        }
    }
    let event_id = child_started_id.context("missing reciprocal Temporal ChildStarted proof")?;
    Ok(Attested {
        parent_input_bytes: parent_bytes,
        command_bytes: serde_json::to_vec(&invocation.command)?,
        invocation,
        parent_child_started_event_id: event_id,
    })
}

async fn exact_history(
    client: &temporalio_client::Client, namespace: &str, workflow_id: &str, run_id: &str,
) -> Result<History> {
    let info = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned()).workflow_id(workflow_id.to_owned())
        .run_id(run_id.to_owned()).build();
    let handle = info.bind_untyped(client.clone());
    handle.describe(WorkflowDescribeOptions::builder().build()).await
        .context("Temporal exact-run Describe failed")?;
    let events = handle.fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.context("Temporal independent exact-run History fetch failed")?;
    ensure!(events.len() <= 10_000, "Temporal Qdrant admission History exceeds event budget");
    Ok(History::from(events))
}

async fn operator_pool() -> Result<PgPool> {
    ensure!(env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1").as_deref() == Ok("true"),
        "Qdrant independent operator admission requires explicit opt-in");
    let path = env::var("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE")
        .context("separate Qdrant operator database secret path required")?;
    let file = Path::new(&path);
    ensure!(file.is_absolute(), "operator DSN secret must use absolute path");
    let meta = fs::symlink_metadata(file)?;
    ensure!(meta.file_type().is_file(), "operator DSN secret must be regular non-symlink file");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        ensure!(meta.permissions().mode() & 0o077 == 0,
            "operator DSN secret requires 0600/0400 mode");
    }
    let url = fs::read_to_string(file)?.trim_end_matches(|v|v == '\n' || v == '\r').to_owned();
    ensure!(!url.is_empty() && url.trim() == url, "operator DB URL unsafe");
    let pool = PgPool::connect(&url).await?;
    let role = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,
                  'pipeline.qdrant_projection_v1_admissions','INSERT') AS can_issue
         FROM pg_roles r WHERE r.rolname=session_user",
    ).fetch_one(&pool).await?;
    let tenant = env::var("QDRANT_EFFECT_OPERATOR_TENANT_ID")?;
    ensure!(!role.try_get::<bool,_>("rolsuper")?
        && !role.try_get::<bool,_>("rolbypassrls")?
        && role.try_get::<bool,_>("can_issue")?
        && role.try_get::<Option<String>,_>("tenant")?.as_deref() == Some(tenant.as_str()),
        "Qdrant operator login must have exact DBA-bound tenant and INSERT admission privilege");
    Ok(pool)
}

async fn signal_after_commit(client: temporalio_client::Client,
    namespace: &str, request: &IssueRequest) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let handle = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone()).build()
        .bind_untyped(client);
    handle.signal(
        UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_qdrant_v1"),
        RawValue::default(), WorkflowSignalOptions::default(),
    ).await.context("signal admitted Qdrant child after committed SQL row")
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes = fs::read(path)?;
    let request: IssueRequest = serde_json::from_slice(&bytes)
        .context("operator request must be strict locator-only JSON")?;
    ensure!(!request.admission_id.is_nil() && !request.canonical_run_id.is_nil()
        && request.expires_in_seconds >= 1 && request.expires_in_seconds <= 300
        && request.parent_workflow_id == request.canonical_run_id.to_string()
        && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
        && Uuid::parse_str(&request.child_workflow_run_id).is_ok(),
        "Qdrant operator locator/TTL invalid");
    let temporal_url = env::var("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")?;
    ensure!(temporal_url.starts_with("http://127.0.0.1:")
        || temporal_url.starts_with("http://localhost:"),
        "Qdrant operator requires trusted localhost Temporal; remote TLS auth unproven");
    ensure!(!namespace.is_empty() && namespace.trim() == namespace,
        "Qdrant Temporal namespace invalid");
    let client = temporal_gateway::connect_client(&temporal_url,
        format!("alegria-qdrant-operator-v1:{}",std::process::id()),&namespace).await?;
    let parent = exact_history(&client,&namespace,&request.parent_workflow_id,
        &request.parent_workflow_run_id).await?;
    let child = exact_history(&client,&namespace,&request.child_workflow_id,
        &request.child_workflow_run_id).await?;
    let attested = attest_pair(&parent,&child,&request)?;
    let pool = operator_pool().await?;
    let tenant = env::var("QDRANT_EFFECT_OPERATOR_TENANT_ID")?;
    ensure!(attested.invocation.command.tenant_id == tenant,
        "operator tenant differs from server History's typed command");
    let command = &attested.invocation.command;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'").execute(&mut *tx).await?;
    // Shares the canonical run-status transition lock used by the PostgreSQL
    // Truth owner and run lifecycle triggers. Source run may become terminal
    // between Temporal History proof and SQL issuance; recheck *under lock*.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-run-status|{}", request.canonical_run_id))
        .execute(&mut *tx).await?;
    let canonical = sqlx::query(
        "SELECT workflow_run_id,workflow_type,status,context_key,tenant_id
         FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_one(&mut *tx).await
    .context("canonical Qdrant run disappeared during operator issuance")?;
    ensure!(canonical.try_get::<String,_>("workflow_run_id")? == request.parent_workflow_run_id
        && canonical.try_get::<String,_>("workflow_type")? == "seo_site_build"
        && matches!(canonical.try_get::<String,_>("status")?.as_str(),
            "verifying" | "pending_hitl")
        && canonical.try_get::<String,_>("context_key")? == command.context_key
        && canonical.try_get::<String,_>("tenant_id")? == command.tenant_id,
        "canonical Qdrant run terminated or changed identity before operator admission");
    let parent_plan_sha256: String = sqlx::query_scalar(
        "SELECT encode(public.digest($1::bytea,'sha256'),'hex')",
    )
    .bind(&attested.parent_input_bytes)
    .fetch_one(&mut *tx).await?;
    // The operator LOGIN has INSERT but deliberately no SELECT on admissions.
    // `INSERT ... RETURNING` would implicitly require a broader SELECT grant.
    let inserted = sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,operation_instance,plan_sha256,command_sha256,
            command_bytes,expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,
                encode(public.digest($13::bytea,'sha256'),'hex'),$13,
                clock_timestamp() + ($14::integer * interval '1 second')
         FROM pipeline.execution_runs er
         WHERE er.run_id=$2 AND er.tenant_id=$3 AND er.context_key=$4
           AND er.workflow_run_id=$7 AND er.workflow_type='seo_site_build'
           AND er.status IN ('verifying','pending_hitl')",
    )
    .bind(request.admission_id)
    .bind(request.canonical_run_id)
    .bind(&command.tenant_id)
    .bind(&command.context_key)
    .bind(&command.keyword_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(&request.child_workflow_id)
    .bind(&request.child_workflow_run_id)
    .bind(attested.parent_child_started_event_id)
    .bind(format!("keyword-{}",request.admission_id))
    // Bind exact server parent History start bytes, not a claimed plan hash.
    .bind(parent_plan_sha256)
    .bind(&attested.command_bytes)
    .bind(request.expires_in_seconds as i32)
    .execute(&mut *tx).await
    .context("native Qdrant operator admission insert refused canonical run/tenant")?;
    ensure!(inserted.rows_affected() == 1,
        "Qdrant operator admission did not insert exactly one authorized row");
    tx.commit().await?;
    signal_after_commit(client,&namespace,&request).await
        .context("Qdrant admission committed but child wakeup signal failed; manually recover only")
}
