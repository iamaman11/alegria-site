//! Independent operator authorization for Qdrant's shared native owner wire.
//! The request contains *locators only*: every effect requires reciprocal
//! Temporal SERVER Histories, concrete Qdrant command verification, an exact
//! trusted route/build, and a separately committed V3 PostgreSQL admission.
//! Produced inputs cannot pass until their origin has independent proof.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id,
    owner_dispatch::{
        OperationDagRunV6, OwnerInvocationV1, OPERATION_DAG_PARENT_QUEUE,
        OPERATION_DAG_PARENT_TYPE, OWNER_OPERATION_CHILD_TYPE_V1,
    },
    ArtifactBinding, Operation, PortId,
};
use qdrant_contracts::{
    verify_external_qdrant_owner_invocation, QdrantKeywordProjectionCommandV1,
    QdrantKeywordProjectionContractV2, QDRANT_EFFECT_INPUT_PORT_V2,
    QDRANT_GENERIC_ADMISSION_SIGNAL_V1, QDRANT_GENERIC_OWNER_QUEUE_V1,
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
struct IssueRequestV3 {
    admission_id: Uuid,
    canonical_run_id: Uuid,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    child_workflow_id: String,
    child_workflow_run_id: String,
    expires_in_seconds: u32,
}

struct AttestedV3 {
    invocation: OwnerInvocationV1,
    command: QdrantKeywordProjectionCommandV1,
    parent_bytes: Vec<u8>,
    child_bytes: Vec<u8>,
    parent_child_started_event_id: i64,
}

fn verify_trusted_temporal_url(candidate: &str) -> Result<()> {
    // A prefix test is insufficient: `http://127.0.0.1:pw@remote.example`
    // has a remote host despite its loopback-looking prefix. Parse the URL
    // and reject credentials, alternate paths and every non-loopback host.
    let parsed =
        url::Url::parse(candidate).context("generic Qdrant operator Temporal URL is malformed")?;
    ensure!(
        parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
            && parsed.port().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "generic Qdrant operator requires exact loopback Temporal URL without userinfo or redirects"
    );
    Ok(())
}

fn attest_pair_v3(
    parent: &History,
    child: &History,
    request: &IssueRequestV3,
    trusted_build_id: &str,
) -> Result<AttestedV3> {
    let parent_bytes = start_payload(
        parent,
        OPERATION_DAG_PARENT_TYPE,
        OPERATION_DAG_PARENT_QUEUE,
    )?;
    let child_bytes = start_payload(
        child,
        OWNER_OPERATION_CHILD_TYPE_V1,
        QDRANT_GENERIC_OWNER_QUEUE_V1,
    )?;
    let run: OperationDagRunV6 = serde_json::from_slice(&parent_bytes)
        .context("generic DAG parent server History contains an invalid typed run")?;
    ensure!(
        run.schema_revision == 6
            && run.plan.hash == run.plan.canonical_hash()?
            && run.plan.scope.run_id == request.canonical_run_id.to_string(),
        "generic DAG parent History has an invalid plan hash or run identity"
    );
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_bytes)
        .context("generic Qdrant child server History has an invalid owner invocation")?;
    let command = verify_external_qdrant_owner_invocation(&invocation)
        .context("generic Qdrant child command is foreign, malformed, or Produced without proof")?;
    let contract = QdrantKeywordProjectionContractV2::contract();
    ensure!(
        command.admission_id == request.admission_id
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id
            && invocation.checked_dispatch.scope == run.plan.scope
            && invocation.checked_dispatch.plan_hash == run.plan.hash
            && invocation.checked_dispatch.operation == contract.reference
            && invocation.checked_dispatch.owner == contract.owner
            && invocation.checked_dispatch.effects == contract.effects
            && request.child_workflow_id
                == format!(
                    "alegria-owner-operation-v1-{}",
                    invocation.checked_dispatch.execution_key
                ),
        "Qdrant child is not bound to exact checked run, owner and immutable parent"
    );
    let contract_hash = contract.fingerprint()?;
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .filter(|p| { p.reference == contract.reference && p.fingerprint == contract_hash })
            .count()
            == 1,
        "concrete Qdrant operation contract/fingerprint is not pinned in parent plan"
    );
    let routes = run
        .pinned_routes
        .iter()
        .filter(|route| route.operation == contract.reference && route.owner == contract.owner)
        .collect::<Vec<_>>();
    ensure!(
        routes.len() == 1,
        "exactly one Qdrant route must be pinned in parent History"
    );
    let route = routes[0];
    ensure!(
        route.contract_hash == contract_hash
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && route.task_queue == QDRANT_GENERIC_OWNER_QUEUE_V1
            && route.build_id == trusted_build_id,
        "Qdrant route does not match independently approved owner build, type and queue"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == invocation.checked_dispatch.instance)
        .context("checked Qdrant operation instance missing from original DAG")?;
    ensure!(
        node.operation == contract.reference && node.permitted_effects == contract.effects,
        "DAG node does not authorize exact Qdrant operation/effect"
    );
    let wire = run
        .owner_commands
        .get(&node.instance)
        .context("parent History has no own typed Qdrant command bytes")?;
    ensure!(
        wire.operation == contract.reference
            && wire.owner == contract.owner
            && wire.contract_hash == contract_hash
            && wire.command_bytes == invocation.command_bytes,
        "child command bytes do not match pinned parent Qdrant command"
    );
    let input_port = PortId::from(QDRANT_EFFECT_INPUT_PORT_V2);
    let source = node
        .inputs
        .get(&input_port)
        .context("Qdrant DAG input port is missing")?;
    let input_id = match source {
        ArtifactBinding::External { artifact } => artifact,
        ArtifactBinding::Produced { .. } => bail!(
            "Produced Qdrant artifact requires independent upstream child History/receipt proof"
        ),
    };
    let bound = invocation
        .bound_inputs
        .get(&input_port)
        .context("Qdrant child omitted the declared typed command input")?;
    let admitted = run
        .admitted
        .get(input_id)
        .context("Qdrant external input is absent from immutable parent admission")?;
    ensure!(
        bound.artifact_id == *input_id
            && bound.source == *source
            && bound.produced_evidence.is_none()
            && bound.artifact == *admitted
            && bound.artifact.payload == invocation.command_bytes,
        "Qdrant input artifact ID, scope, provenance or bytes differ from parent History"
    );
    ensure!(
        serde_json::to_vec(&invocation)? == child_bytes,
        "child start History differs from canonical generic owner invocation bytes"
    );
    let child_start = child
        .events
        .first()
        .context("Qdrant child server History empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &child_start.attributes
    else {
        bail!("Qdrant child first event was not server WorkflowExecutionStarted");
    };
    let recorded_parent = attrs
        .parent_workflow_execution
        .as_ref()
        .context("Qdrant child server History has no parent identity")?;
    ensure!(
        recorded_parent.workflow_id == request.parent_workflow_id
            && recorded_parent.run_id == request.parent_workflow_run_id,
        "child server History parent identity is foreign"
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
                == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                && init.task_queue.as_ref().map(|v| v.name.as_str())
                    == Some(QDRANT_GENERIC_OWNER_QUEUE_V1),
            "parent initiated an untrusted Qdrant child type or queue"
        );
        let payloads = &init
            .input
            .as_ref()
            .context("parent child initiation omitted typed input")?
            .payloads;
        ensure!(
            payloads.len() == 1 && payloads[0].data.as_slice() == child_bytes.as_slice(),
            "parent child initiation bytes differ from child server History"
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
            let actual_child = actual
                .workflow_execution
                .as_ref()
                .context("parent ChildStarted omitted child execution")?;
            ensure!(
                actual_child.workflow_id == request.child_workflow_id
                    && actual_child.run_id == request.child_workflow_run_id
                    && actual.workflow_type.as_ref().map(|v| v.name.as_str())
                        == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                    && attrs.parent_initiated_event_id == initiated.event_id,
                "reciprocal generic Qdrant child server History identity mismatch"
            );
            ensure!(
                matching_started.replace(started.event_id).is_none(),
                "multiple ChildStarted events cannot authorize one Qdrant side effect"
            );
        }
    }
    Ok(AttestedV3 {
        invocation,
        command,
        parent_bytes,
        child_bytes,
        parent_child_started_event_id: matching_started
            .context("no reciprocal Qdrant child server ChildStarted event")?,
    })
}

async fn operator_pool_v3() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_OPERATOR_V1").as_deref() == Ok("true"),
        "generic Qdrant operator requires distinct explicit opt-in"
    );
    let filename = env::var("QDRANT_GENERIC_OPERATOR_DATABASE_URL_FILE")
        .context("generic Qdrant operator requires an independent DSN secret file")?;
    let path = Path::new(&filename);
    ensure!(
        path.is_absolute(),
        "generic Qdrant operator secret path must be absolute"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_file(),
        "generic Qdrant operator secret must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "generic Qdrant operator secret requires 0600/0400 permissions"
        );
    }
    let url = fs::read_to_string(path)?
        .trim_end_matches(|ch| ch == '\r' || ch == '\n')
        .to_owned();
    ensure!(
        !url.is_empty() && url.trim() == url,
        "malformed generic Qdrant operator DSN"
    );
    let pool = PgPool::connect(&url).await?;
    let role = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','INSERT') AS v3_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','INSERT') AS v2_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','INSERT') AS v1_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','SELECT') AS v2_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','SELECT') AS v1_read,
                has_table_privilege(session_user,'pipeline.operation_dag_run_governance_v1','INSERT') AS governance_issue,
                has_function_privilege(session_user,
                    'pipeline.operation_dag_governance_matches_v1(uuid,text,text,text,text,text,text,boolean)','EXECUTE') AS governed,
                has_function_privilege(session_user,
                    'pipeline.verify_qdrant_operator_resignal_v3(uuid,uuid,text,text,text,text,text,text,text,bigint,bytea,bytea,bytea,text)','EXECUTE') AS resignal_allowed,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS v3_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS v3_confirm,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_confirm,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_confirm
           FROM pg_roles r WHERE r.rolname=session_user",
    ).fetch_one(&pool).await?;
    ensure!(
        !role.try_get::<bool, _>("rolsuper")?
            && !role.try_get::<bool, _>("rolbypassrls")?
            && role.try_get::<bool, _>("v3_insert")?
            && !role.try_get::<bool, _>("v2_insert")?
            && !role.try_get::<bool, _>("v1_insert")?
            && !role.try_get::<bool, _>("v2_read")?
            && !role.try_get::<bool, _>("v1_read")?
            && !role.try_get::<bool, _>("governance_issue")?
            && role.try_get::<bool, _>("governed")?
            && role.try_get::<bool, _>("resignal_allowed")?
            && !role.try_get::<bool, _>("v3_begin")?
            && !role.try_get::<bool, _>("v3_confirm")?
            && !role.try_get::<bool, _>("v2_begin")?
            && !role.try_get::<bool, _>("v2_confirm")?
            && !role.try_get::<bool, _>("v1_begin")?
            && !role.try_get::<bool, _>("v1_confirm")?
            && role.try_get::<Option<String>, _>("tenant")?.as_deref()
                == Some(env::var("QDRANT_GENERIC_OPERATOR_TENANT_ID")?.as_str()),
        "generic Qdrant operator must have only tenant-bound V3 admission authority"
    );
    Ok(pool)
}

async fn attested_request(
    file: &Path,
) -> Result<(
    IssueRequestV3,
    String,
    temporalio_client::Client,
    AttestedV3,
    PgPool,
)> {
    let request: IssueRequestV3 = serde_json::from_slice(&fs::read(file)?)
        .context("V3 operator request must contain only strict server run locators")?;
    ensure!(
        !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && (1..=300).contains(&request.expires_in_seconds)
            && request.parent_workflow_id
                == operation_dag_parent_workflow_id(&request.canonical_run_id.to_string())
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok(),
        "invalid generic Qdrant operator run locators or TTL"
    );
    let temporal_url = env::var("QDRANT_GENERIC_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("QDRANT_GENERIC_OPERATOR_TEMPORAL_NAMESPACE")?;
    verify_trusted_temporal_url(&temporal_url)?;
    ensure!(
        !namespace.trim().is_empty() && namespace.trim() == namespace,
        "invalid generic Qdrant Temporal namespace"
    );
    let trusted_build_id = env::var("QDRANT_GENERIC_OPERATOR_TRUSTED_BUILD_ID")
        .context("generic Qdrant operator requires exact trusted deployed owner build ID")?;
    ensure!(
        !trusted_build_id.trim().is_empty() && trusted_build_id.trim() == trusted_build_id,
        "invalid trusted Qdrant owner build ID"
    );
    let client = temporal_gateway::connect_client(
        &temporal_url,
        format!("alegria-qdrant-generic-operator:{}", std::process::id()),
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
    let attested = attest_pair_v3(&parent, &child, &request, &trusted_build_id)?;
    ensure!(
        attested.command.tenant_id == env::var("QDRANT_GENERIC_OPERATOR_TENANT_ID")?,
        "generic Qdrant operator is not the admitted command tenant"
    );
    let pool = operator_pool_v3().await?;
    Ok((request, namespace, client, attested, pool))
}

async fn signal_after_commit(
    client: temporalio_client::Client,
    namespace: &str,
    request: &IssueRequestV3,
) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let info = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build();
    info.bind_untyped(client)
        .signal(
            UntypedSignal::<UntypedWorkflow>::new(QDRANT_GENERIC_ADMISSION_SIGNAL_V1),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("signal exact generic Qdrant child after committed SQL admission")
}

pub(crate) async fn issue_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let command = &proof.command;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-run-status|{}", request.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    // Lock in the same order as the governance issuer/revoker and the V3
    // first-start SQL function. Revocation must not race between the active
    // governance predicate and commitment of a new Qdrant admission.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!(
            "operation-dag-canonical|{}",
            request.canonical_run_id
        ))
        .execute(&mut *tx)
        .await?;
    let canonical = sqlx::query(
        "SELECT workflow_type,status,context_key,tenant_id
           FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_one(&mut *tx)
    .await
    .context("generic Qdrant canonical execution run disappeared before admission")?;
    ensure!(
        canonical.try_get::<String, _>("workflow_type")? == "seo_site_build"
            && matches!(
                canonical.try_get::<String, _>("status")?.as_str(),
                "verifying" | "pending_hitl"
            )
            && canonical.try_get::<String, _>("tenant_id")? == command.tenant_id
            && canonical.try_get::<String, _>("context_key")? == command.context_key,
        "generic Qdrant canonical source run terminal or foreign"
    );
    // A generic DAG is a distinct Temporal execution from seo_site_build.
    // Only an independently operator-issued governance row, attested against
    // the actual generic parent server start History, can bind these runs.
    // No invented equality with the SEO Workflow Run ID is permissible.
    let governed: bool = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_governance_matches_v1(
            $1,$2,$3,$4,$5,$6,
            encode(public.digest($7::bytea,'sha256'),'hex'),TRUE)",
    )
    .bind(request.canonical_run_id)
    .bind(&command.tenant_id)
    .bind(&command.context_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
    .bind(&proof.parent_bytes)
    .fetch_one(&mut *tx)
    .await
    .context("generic Qdrant operator requires separately attested active DAG governance")?;
    ensure!(
        governed,
        "Qdrant V3 cannot issue admission without immutable generic DAG governance"
    );
    let inserted = sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v3_admissions
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
           AND er.workflow_type='seo_site_build'
           AND er.status IN ('verifying','pending_hitl')
           AND pipeline.operation_dag_governance_matches_v1(
               $2,$3,$4,$6,$7,$12,
               encode(public.digest($14::bytea,'sha256'),'hex'),TRUE)",
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
    .bind(&proof.invocation.command_bytes)
    .bind(request.expires_in_seconds as i32)
    .execute(&mut *tx)
    .await
    .context("generic Qdrant V3 admission insert denied by tenant/run/History fence")?;
    ensure!(
        inserted.rows_affected() == 1,
        "V3 Qdrant must insert exactly one operator admission"
    );
    tx.commit().await?;
    signal_after_commit(client, &namespace, &request)
        .await
        .context("generic Qdrant SQL admission committed, signal failed; use exact resignal")
}

pub(crate) async fn resignal_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let command = &proof.command;
    let matched: bool = sqlx::query_scalar(
        "SELECT pipeline.verify_qdrant_operator_resignal_v3(
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
    .bind(&proof.invocation.command_bytes)
    .bind(&proof.invocation.checked_dispatch.execution_key)
    .fetch_one(&pool)
    .await
    .context("generic Qdrant V3 SQL rejected absent/expired/foreign/changed admission")?;
    ensure!(
        matched,
        "generic Qdrant V3 immutable SQL History admission mismatch"
    );
    signal_after_commit(client, &namespace, &request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_owner_operator_requires_exact_loopback_temporal_origin() {
        assert!(verify_trusted_temporal_url("http://127.0.0.1:7233").is_ok());
        assert!(verify_trusted_temporal_url("http://localhost:7233/").is_ok());
        for forged in [
            "http://127.0.0.1:pw@evil.example:7233",
            "http://localhost:pw@evil.example:7233",
            "https://localhost:7233",
            "http://127.0.0.1:7233/other",
            "http://localhost:7233/?fallback=https://evil.example",
            "http://localhost:7233/#evil",
            "http://0.0.0.0:7233",
            "http://127.0.0.1",
        ] {
            assert!(
                verify_trusted_temporal_url(forged).is_err(),
                "untrusted Temporal URL incorrectly accepted: {forged}"
            );
        }
    }
}
