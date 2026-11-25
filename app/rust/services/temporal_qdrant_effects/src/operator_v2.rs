//! Independent operator admission for the versioned mixed-owner Qdrant V2
//! child. Every authorization requires two live Temporal server Histories.
//! V1 Qdrant-only parent and its SQL ledger cannot authorize this route.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::owner_dispatch::{
    OperationDagRunV6, OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_TYPE,
};
use operation_protocol::{ArtifactBinding, Operation, PortId};
use qdrant_contracts::{
    QdrantKeywordChildInvocationV2, QdrantKeywordProjectionCommandV1,
    QdrantKeywordProjectionContractV2, QDRANT_EFFECT_INPUT_PORT_V2, QDRANT_OWNER_CHILD_TYPE_V2,
    QDRANT_OWNER_COMMAND_QUEUE_V2,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowExecutionInfo, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use uuid::Uuid;

use super::operator::{exact_history, start_payload};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequestV2 {
    admission_id: Uuid,
    canonical_run_id: Uuid,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    child_workflow_id: String,
    child_workflow_run_id: String,
    expires_in_seconds: u32,
}

struct AttestedV2 {
    invocation: QdrantKeywordChildInvocationV2,
    parent_bytes: Vec<u8>,
    child_bytes: Vec<u8>,
    command_bytes: Vec<u8>,
    parent_child_started_event_id: i64,
}

fn attest_pair_v2(
    parent: &History,
    child: &History,
    request: &IssueRequestV2,
    trusted_build_id: &str,
) -> Result<AttestedV2> {
    let parent_bytes = start_payload(
        parent,
        OPERATION_DAG_PARENT_TYPE,
        OPERATION_DAG_PARENT_QUEUE,
    )?;
    let child_bytes = start_payload(
        child,
        QDRANT_OWNER_CHILD_TYPE_V2,
        QDRANT_OWNER_COMMAND_QUEUE_V2,
    )?;
    let run: OperationDagRunV6 = serde_json::from_slice(&parent_bytes)
        .context("mixed-owner V6 parent History has an invalid typed input")?;
    ensure!(run.schema_revision == 6, "incorrect parent DAG revision");
    ensure!(
        run.plan.hash == run.plan.canonical_hash()?
            && run.plan.scope.run_id == request.canonical_run_id.to_string(),
        "parent DAG is not immutable or belongs to another canonical run"
    );

    let invocation: QdrantKeywordChildInvocationV2 = serde_json::from_slice(&child_bytes)
        .context("Qdrant V2 child History has an invalid typed input")?;
    invocation.verify_integrity()?;
    let expected = QdrantKeywordProjectionContractV2::contract();
    ensure!(
        invocation.canonical_run_id == request.canonical_run_id
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id
            && invocation.command.admission_id == request.admission_id
            && invocation.checked_dispatch.operation == expected.reference
            && invocation.checked_dispatch.owner == expected.owner
            && invocation.checked_dispatch.effects == expected.effects
            && invocation.checked_dispatch.scope == run.plan.scope
            && invocation.checked_dispatch.plan_hash == run.plan.hash
            && request.child_workflow_id
                == format!("alegria-qdrant-effect-v2-{}", request.admission_id),
        "Qdrant V2 child is not bound to exact checked DAG operation/tenant/parent"
    );
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .filter(|p| {
                p.reference == expected.reference
                    && expected
                        .fingerprint()
                        .is_ok_and(|hash| p.fingerprint == hash)
            })
            .count()
            == 1,
        "Qdrant V2 contract revision/fingerprint not pinned in original parent plan"
    );
    let matching_routes = run
        .pinned_routes
        .iter()
        .filter(|route| route.operation == expected.reference && route.owner == expected.owner)
        .collect::<Vec<_>>();
    ensure!(
        matching_routes.len() == 1,
        "Qdrant owner route must be pinned exactly once"
    );
    let route = matching_routes[0];
    ensure!(
        route.contract_hash == expected.fingerprint()?
            && route.workflow_type == QDRANT_OWNER_CHILD_TYPE_V2
            && route.task_queue == QDRANT_OWNER_COMMAND_QUEUE_V2
            && route.build_id == trusted_build_id,
        "Qdrant owner route in parent History does not match approved typed child"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == invocation.checked_dispatch.instance)
        .context("checked Qdrant instance missing from parent DAG")?;
    ensure!(
        node.operation == expected.reference && node.permitted_effects == expected.effects,
        "Qdrant DAG node does not permit exactly the checked owner effect"
    );
    let wire = run
        .owner_commands
        .get(&node.instance)
        .context("parent DAG Qdrant operation has no owner-specific command")?;
    ensure!(
        wire.operation == expected.reference
            && wire.owner == expected.owner
            && wire.contract_hash == expected.fingerprint()?,
        "Qdrant owner command in parent History is not the pinned contract"
    );
    let command: QdrantKeywordProjectionCommandV1 = serde_json::from_slice(&wire.command_bytes)
        .context("parent Qdrant command bytes do not deserialize to typed owner input")?;
    ensure!(
        command == invocation.command,
        "Qdrant child command differs from parent History"
    );
    let input = match node.inputs.get(&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2)) {
        Some(ArtifactBinding::External { artifact }) => artifact,
        _ => bail!("Qdrant DAG does not bind an external immutable command artifact"),
    };
    let artifact = run
        .admitted
        .get(input)
        .context("Qdrant parent admitted input missing")?;
    ensure!(
        input == &invocation.bound_input.artifact_id
            && artifact.schema == invocation.bound_input.schema
            && artifact.scope == invocation.bound_input.scope
            && artifact.provenance == invocation.bound_input.provenance
            && artifact.payload == invocation.bound_input.payload,
        "child input bytes and provenance differ from parent History admitted artifact"
    );
    ensure!(
        serde_json::to_vec(&invocation)? == child_bytes,
        "child History input is not canonical typed V2 serialization"
    );

    let child_start = child.events.first().context("Qdrant child History empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &child_start.attributes
    else {
        bail!("child first event was not Temporal WorkflowExecutionStarted");
    };
    let child_parent = attrs
        .parent_workflow_execution
        .as_ref()
        .context("child server start omitted parent Workflow execution")?;
    ensure!(
        child_parent.workflow_id == request.parent_workflow_id
            && child_parent.run_id == request.parent_workflow_run_id,
        "server-origin child parent identity differs from expected V6 DAG"
    );

    let mut matching_started = None;
    for initiated in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(init)) =
            &initiated.attributes
        else {
            continue;
        };
        if init.workflow_id != request.child_workflow_id {
            continue;
        }
        ensure!(
            init.workflow_type.as_ref().map(|v| v.name.as_str())
                == Some(QDRANT_OWNER_CHILD_TYPE_V2)
                && init.task_queue.as_ref().map(|v| v.name.as_str())
                    == Some(QDRANT_OWNER_COMMAND_QUEUE_V2),
            "parent requested an unexpected child Workflow Type/Queue"
        );
        let payloads = &init
            .input
            .as_ref()
            .context("parent initiation has no typed input")?
            .payloads;
        ensure!(
            payloads.len() == 1 && payloads[0].data.as_slice() == child_bytes.as_slice(),
            "parent initiation bytes differ from Qdrant V2 child server History"
        );
        for started in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(actual)) =
                &started.attributes
            else {
                continue;
            };
            if actual.initiated_event_id != initiated.event_id {
                continue;
            }
            let child_id = actual
                .workflow_execution
                .as_ref()
                .context("parent ChildStarted missing child identity")?;
            ensure!(
                child_id.workflow_id == request.child_workflow_id
                    && child_id.run_id == request.child_workflow_run_id
                    && actual.workflow_type.as_ref().map(|t| t.name.as_str())
                        == Some(QDRANT_OWNER_CHILD_TYPE_V2)
                    && attrs.parent_initiated_event_id == initiated.event_id,
                "Qdrant V2 reciprocal parent/child history identity invalid"
            );
            ensure!(
                matching_started.replace(started.event_id).is_none(),
                "duplicate Qdrant V2 ChildStarted events cannot authorize an effect"
            );
        }
    }
    Ok(AttestedV2 {
        invocation,
        parent_bytes,
        child_bytes,
        command_bytes: wire.command_bytes.clone(),
        parent_child_started_event_id: matching_started.context("no reciprocal V2 ChildStarted")?,
    })
}

async fn attested_request(
    path: &Path,
) -> Result<(
    IssueRequestV2,
    String,
    temporalio_client::Client,
    AttestedV2,
    sqlx::PgPool,
)> {
    let request: IssueRequestV2 = serde_json::from_slice(&fs::read(path)?)
        .context("V2 operator request must contain only strict Temporal run locators")?;
    ensure!(
        !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && (1..=300).contains(&request.expires_in_seconds)
            && !request.parent_workflow_id.trim().is_empty()
            && request.parent_workflow_id.trim() == request.parent_workflow_id
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok(),
        "invalid V2 admission locators / TTL"
    );
    let temporal_url = env::var("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")?;
    ensure!(
        temporal_url.starts_with("http://127.0.0.1:")
            || temporal_url.starts_with("http://localhost:"),
        "Qdrant V2 operator requires explicitly trusted local Temporal connection"
    );
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "invalid V2 Temporal namespace"
    );
    let client = temporal_gateway::connect_client(
        &temporal_url,
        format!("alegria-qdrant-v2-operator:{}", std::process::id()),
        &namespace,
    )
    .await?;
    let parent = exact_history(
        &client,
        &namespace,
        &request.parent_workflow_id,
        &request.parent_workflow_run_id,
    )
    .await?;
    let child = exact_history(
        &client,
        &namespace,
        &request.child_workflow_id,
        &request.child_workflow_run_id,
    )
    .await?;
    let trusted_build_id = env::var("QDRANT_EFFECT_OPERATOR_TRUSTED_BUILD_ID_V2")
        .context("Qdrant V2 operator requires trusted static owner build ID")?;
    ensure!(
        !trusted_build_id.trim().is_empty() && trusted_build_id.trim() == trusted_build_id,
        "invalid trusted owner build ID"
    );
    let attested = attest_pair_v2(&parent, &child, &request, &trusted_build_id)?;
    let pool = operator_pool_v2().await?;
    ensure!(
        attested.invocation.command.tenant_id == env::var("QDRANT_EFFECT_OPERATOR_TENANT_ID")?,
        "Qdrant V2 operator tenant does not match original typed command"
    );
    Ok((request, namespace, client, attested, pool))
}

async fn operator_pool_v2() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V2").as_deref() == Ok("true"),
        "V2 Qdrant operator requires separate explicit opt-in"
    );
    let path = env::var("QDRANT_EFFECT_OPERATOR_V2_DATABASE_URL_FILE")
        .context("independent V2 Qdrant operator DSN secret file required")?;
    let file = Path::new(&path);
    ensure!(file.is_absolute(), "V2 operator DSN path must be absolute");
    let metadata = fs::symlink_metadata(file)?;
    ensure!(
        metadata.file_type().is_file(),
        "V2 operator DSN must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "V2 operator secret requires owner-only 0600/0400 permissions"
        );
    }
    let url = fs::read_to_string(file)?
        .trim_end_matches(|c| c == '\r' || c == '\n')
        .to_owned();
    ensure!(
        !url.is_empty() && url.trim() == url,
        "V2 operator DSN has unsafe whitespace"
    );
    let pool = PgPool::connect(&url).await?;
    let role = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,
                  'pipeline.qdrant_projection_v2_admissions','INSERT') AS can_issue_v2,
                has_table_privilege(session_user,
                  'pipeline.qdrant_projection_v1_admissions','INSERT') AS can_issue_v1
           FROM pg_roles r WHERE r.rolname=session_user",
    )
    .fetch_one(&pool)
    .await?;
    let tenant = env::var("QDRANT_EFFECT_OPERATOR_TENANT_ID")?;
    ensure!(
        !role.try_get::<bool, _>("rolsuper")?
            && !role.try_get::<bool, _>("rolbypassrls")?
            && role.try_get::<bool, _>("can_issue_v2")?
            && !role.try_get::<bool, _>("can_issue_v1")?
            && role.try_get::<Option<String>, _>("tenant")?.as_deref() == Some(tenant.as_str()),
        "V2 operator must be tenant-bound with only V2 INSERT, no V1 admission privilege"
    );
    Ok(pool)
}

async fn signal_after_commit(
    client: temporalio_client::Client,
    namespace: &str,
    request: &IssueRequestV2,
) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let info = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build();
    info.bind_untyped(client)
        .signal(
            UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_qdrant_v2"),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("signal exact Qdrant V2 child after committed SQL admission")
}

pub(crate) async fn issue_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let command = &proof.invocation.command;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-run-status|{}", request.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    let canonical = sqlx::query(
        "SELECT workflow_run_id,workflow_type,status,context_key,tenant_id
           FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_one(&mut *tx)
    .await
    .context("canonical mixed-DAG execution disappeared before Qdrant admission")?;
    ensure!(
        canonical.try_get::<String, _>("workflow_run_id")? == request.parent_workflow_run_id
            && canonical.try_get::<String, _>("workflow_type")? == "seo_site_build"
            && matches!(
                canonical.try_get::<String, _>("status")?.as_str(),
                "verifying" | "pending_hitl"
            )
            && canonical.try_get::<String, _>("context_key")? == command.context_key
            && canonical.try_get::<String, _>("tenant_id")? == command.tenant_id,
        "canonical mixed-DAG run terminal or changed tenant/context before admission"
    );
    let inserted = sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v2_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,operation_instance,plan_hash,dispatch_execution_key,
            parent_sha256,child_sha256,command_sha256,command_bytes,expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,
           encode(public.digest($14::bytea,'sha256'),'hex'),
           encode(public.digest($15::bytea,'sha256'),'hex'),
           encode(public.digest($16::bytea,'sha256'),'hex'),$16,
           clock_timestamp()+($17::integer*interval '1 second')
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
    .bind(proof.parent_child_started_event_id)
    .bind(proof.invocation.checked_dispatch.instance.as_str())
    .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
    .bind(&proof.invocation.checked_dispatch.execution_key)
    .bind(&proof.parent_bytes)
    .bind(&proof.child_bytes)
    .bind(&proof.command_bytes)
    .bind(request.expires_in_seconds as i32)
    .execute(&mut *tx)
    .await
    .context("V2 Qdrant admission insert denied by tenant/run/immutability fence")?;
    ensure!(
        inserted.rows_affected() == 1,
        "operator must insert exactly one V2 admission"
    );
    tx.commit().await?;
    signal_after_commit(client, &namespace, &request)
        .await
        .context("Qdrant V2 admission committed but wakeup failed; use exact history resignal")
}

pub(crate) async fn resignal_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let command = &proof.invocation.command;
    let matched: bool = sqlx::query_scalar(
        "SELECT pipeline.verify_qdrant_operator_resignal_v2(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
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
    .bind(proof.parent_child_started_event_id)
    .bind(&proof.parent_bytes)
    .bind(&proof.child_bytes)
    .bind(&proof.command_bytes)
    .bind(&proof.invocation.checked_dispatch.execution_key)
    .fetch_one(&pool)
    .await
    .context(
        "Qdrant V2 resignal SQL denied absent/expired/revoked/terminal/mismatched admission",
    )?;
    ensure!(
        matched,
        "Qdrant V2 exact immutable SQL admission verification failed"
    );
    signal_after_commit(client, &namespace, &request).await
}
