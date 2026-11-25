//! Independently attested PostgreSQL owner for the stable native Temporal
//! OwnerInvocationV1 ABI. Neither a manifest nor a signed-looking child input
//! is an effect capability: both reciprocal server Histories and the actual
//! authoritative PG source/tenant are proven before a schema-5 admission.
//!
//! This is intentionally a narrow site-build Truth-review pilot: the
//! canonical SEO run governance remains enforced, and Produced-to-PG inputs
//! fail closed until independently attested upstream receipts exist.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id, ArtifactBinding, OperationDagRunV6, OwnerInvocationV1,
    PortId, OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_TYPE, OWNER_OPERATION_CHILD_TYPE_V1,
};
use postgres_store::truth_review_effect_v3::{
    contract_fingerprint, validate_pg_owner_invocation_v1, ReconcileTruthReviewsInput,
    V4TemporalAttestation, PG_OWNER_TASK_QUEUE_V1,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowExecutionInfo, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use truth_domain::review_effect_contract::{operation_contract, INPUT_PORT};
use uuid::Uuid;

use super::operator_v4::{
    fetch_exact_history, first_started_payload, locked_source_hash, sha256_pg,
};

const MAX_ADMISSION_SECONDS: u32 = 300;
const GENERIC_CHILD_ID_PREFIX: &str = "alegria-owner-operation-v1-";

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
    invocation: OwnerInvocationV1,
    exact_invocation_bytes: Vec<u8>,
    parent_start_bytes: Vec<u8>,
    child_started_event_id: i64,
    command: ReconcileTruthReviewsInput,
}

fn prove_plan_source_and_own_command(
    parent_bytes: &[u8],
    invocation: &OwnerInvocationV1,
    canonical_run: Uuid,
    trusted_build_id: &str,
) -> Result<ReconcileTruthReviewsInput> {
    let command = validate_pg_owner_invocation_v1(invocation)
        .context("PG typed command, owner dispatch or external artifact invalid")?;
    let run: OperationDagRunV6 = serde_json::from_slice(parent_bytes)
        .context("parent did not start with typed provider-neutral OperationDagRunV6")?;
    ensure!(
        run.schema_revision == 6 && serde_json::to_vec(&run)? == parent_bytes,
        "parent V6 History is not exact canonical versioned DAG input"
    );
    ensure!(
        run.plan.hash == run.plan.canonical_hash()?,
        "parent immutable DAG plan hash invalid"
    );
    let checked = &invocation.checked_dispatch;
    let pg_contract = operation_contract();
    let fingerprint = pg_contract.fingerprint()?;
    ensure!(
        checked.plan_hash == run.plan.hash
            && checked.scope == run.plan.scope
            && checked.scope.run_id == canonical_run.to_string()
            && checked.operation == pg_contract.reference
            && checked.owner == pg_contract.owner
            && checked.effects == pg_contract.effects
            && checked.idempotency == pg_contract.idempotency,
        "PG child dispatch is not a pinned exact operation in canonical plan"
    );
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .any(|p| p.reference == checked.operation && p.fingerprint == fingerprint),
        "canonical DAG omitted exact PG operation contract fingerprint"
    );
    let owner_wire = run
        .owner_commands
        .get(&checked.instance)
        .context("parent History omitted this exact PG owner command")?;
    ensure!(
        owner_wire.operation == checked.operation
            && owner_wire.owner == checked.owner
            && owner_wire.contract_hash == fingerprint
            && owner_wire.command_bytes == invocation.command_bytes,
        "PG parent owner command bytes/type/contract do not match real child History"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == checked.instance)
        .context("parent DAG lacks this checked PG instance")?;
    ensure!(
        node.operation == checked.operation
            && node.permitted_effects == checked.effects
            && run.plan.allowed_effects.is_superset(&checked.effects),
        "PG node/effect set is not permitted by immutable plan"
    );
    ensure!(
        node.inputs.len() == invocation.bound_inputs.len()
            && node.inputs.iter().all(|(port, binding)| invocation
                .bound_inputs
                .get(port)
                .is_some_and(|bound| &bound.source == binding)),
        "bound child input source differs from exact parent node's binding"
    );
    let port = PortId::from(INPUT_PORT);
    let bound = invocation
        .bound_inputs
        .get(&port)
        .context("PG owner missing exact Truth-review input port")?;
    let ArtifactBinding::External { artifact } = &bound.source else {
        bail!("Produced-to-PG requires independent upstream owner receipt; unsupported");
    };
    ensure!(
        artifact == &bound.artifact_id,
        "PG child's external artifact ID is not the sealed parent binding"
    );
    let parent_artifact = run
        .admitted
        .get(artifact)
        .context("PG external input not present in parent's admitted snapshot")?;
    ensure!(
        parent_artifact == &bound.artifact,
        "PG artifact bytes/schema/scope/provenance mismatch immutable parent History"
    );
    let route = run
        .pinned_routes
        .iter()
        .find(|route| route.operation == checked.operation && route.owner == checked.owner)
        .context("parent lacks PG owner pinned route")?;
    ensure!(
        route.contract_hash == fingerprint
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && route.task_queue == PG_OWNER_TASK_QUEUE_V1
            && route.build_id == trusted_build_id
            && run
                .pinned_routes
                .iter()
                .filter(|r| r.operation == checked.operation)
                .count()
                == 1,
        "PG route in parent History differs from operator-owned type/queue/build ID"
    );
    ensure!(
        command.run_id == canonical_run
            && command.operation_instance == checked.instance.as_str()
            && command.plan_hash == checked.plan_hash.as_str()
            && command.scope == checked.scope,
        "PG typed command is not bound to canonical run/checked instance"
    );
    Ok(command)
}

fn prove_temporal_histories(
    parent: &History,
    child: &History,
    request: &IssueRequest,
    trusted_build_id: &str,
) -> Result<Attested> {
    let parent_input = first_started_payload(
        parent,
        OPERATION_DAG_PARENT_TYPE,
        OPERATION_DAG_PARENT_QUEUE,
    )?;
    let child_input =
        first_started_payload(child, OWNER_OPERATION_CHILD_TYPE_V1, PG_OWNER_TASK_QUEUE_V1)?;
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_input)
        .context("child History did not start with typed native OwnerInvocationV1")?;
    ensure!(
        serde_json::to_vec(&invocation)? == child_input
            && request.child_workflow_id
                == format!(
                    "{GENERIC_CHILD_ID_PREFIX}{}",
                    invocation.checked_dispatch.execution_key
                )
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id,
        "child's canonical start bytes, deterministic ID or parent identity mismatch"
    );
    let first = child.events.first().context("child History is empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(child_start)) = &first.attributes
    else {
        bail!("child History lacks server WorkflowExecutionStarted")
    };
    let actual_parent = child_start
        .parent_workflow_execution
        .as_ref()
        .context("child start has no server-attested parent")?;
    ensure!(
        actual_parent.workflow_id == request.parent_workflow_id
            && actual_parent.run_id == request.parent_workflow_run_id,
        "child server-attested parent Run ID does not match independent locator"
    );
    let mut matched = None;
    for event in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(initiated)) =
            &event.attributes
        else {
            continue;
        };
        if initiated.workflow_id != request.child_workflow_id {
            continue;
        }
        ensure!(
            initiated.workflow_type.as_ref().map(|v| v.name.as_str())
                == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                && initiated.task_queue.as_ref().map(|v| v.name.as_str())
                    == Some(PG_OWNER_TASK_QUEUE_V1)
                && initiated
                    .input
                    .as_ref()
                    .is_some_and(|v| v.payloads.len() == 1
                        && v.payloads[0].data.as_slice() == child_input.as_slice()),
            "parent server Initiated event scheduled different native PG child/input/queue"
        );
        for started_event in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(started)) =
                &started_event.attributes
            else {
                continue;
            };
            if started.initiated_event_id != event.event_id {
                continue;
            }
            let actual_child = started
                .workflow_execution
                .as_ref()
                .context("server ChildStarted lacks actual child execution")?;
            ensure!(
                actual_child.workflow_id == request.child_workflow_id
                    && actual_child.run_id == request.child_workflow_run_id
                    && started.workflow_type.as_ref().map(|v| v.name.as_str())
                        == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                    && child_start.parent_initiated_event_id == event.event_id,
                "parent and child server histories are not reciprocal exact executions"
            );
            ensure!(
                matched.replace(started_event.event_id).is_none(),
                "duplicate ambiguous child start attestation"
            );
        }
    }
    let child_started_event_id = matched
        .context("parent Temporal History did not attest unique reciprocal child-start event")?;
    let command = prove_plan_source_and_own_command(
        &parent_input,
        &invocation,
        request.canonical_run_id,
        trusted_build_id,
    )?;
    ensure!(
        command.admission_id == request.admission_id,
        "operator admission UUID differs from server-attested PG command"
    );
    Ok(Attested {
        invocation,
        exact_invocation_bytes: child_input,
        parent_start_bytes: parent_input,
        child_started_event_id,
        command,
    })
}

async fn attest_from_server(request: &IssueRequest, build_id: &str) -> Result<Attested> {
    let url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")
        .context("PG owner requires independently configured Temporal operator URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")
        .context("PG owner requires exact Temporal namespace")?;
    super::operator_dag_governance::validate_local_temporal_endpoint(&url)?;
    ensure!(
        !namespace.trim().is_empty() && namespace == namespace.trim(),
        "PG owner issuer requires nonempty exact Temporal namespace"
    );
    let client = crate::connect_client(
        &url,
        format!("alegria-pg-native-operator-{}", std::process::id()),
        &namespace,
    )
    .await?;
    let parent = fetch_exact_history(
        &client,
        &namespace,
        &request.parent_workflow_id,
        &request.parent_workflow_run_id,
    )
    .await?;
    let child = fetch_exact_history(
        &client,
        &namespace,
        &request.child_workflow_id,
        &request.child_workflow_run_id,
    )
    .await?;
    prove_temporal_histories(&parent, &child, request, build_id)
}

async fn insert_admission(
    pool: &PgPool,
    request: &IssueRequest,
    attested: &Attested,
    tenant: &str,
) -> Result<()> {
    super::operator::enforce_authoritative_pg_tenant(pool, tenant).await?;
    ensure!(
        attested.command.scope.tenant_id == tenant && !tenant.trim().is_empty(),
        "server-attested PG command does not match authenticated operator tenant"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-context|{}", attested.command.context_key))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-run-status|{}", request.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!(
            "operation-dag-canonical|{}",
            request.canonical_run_id
        ))
        .execute(&mut *tx)
        .await?;
    // The canonical SEO Truth source is a DIFFERENT Temporal execution from
    // the new, separately operator-admitted native generic DAG parent.
    let run = sqlx::query(
        "SELECT context_key,workflow_run_id,workflow_type,status
         FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("PG owner operator found no authoritative canonical Truth run")?;
    ensure!(
        run.get::<String, _>("context_key") == attested.command.context_key
            && run.get::<String, _>("workflow_type") == "seo_site_build"
            && matches!(
                run.get::<String, _>("status").as_str(),
                "verifying" | "pending_hitl"
            ),
        "generic PG owner is not an approved tenant/site-build Truth-run experiment"
    );
    let source_hash = locked_source_hash(&mut tx, &attested.command.context_key).await?;
    let tenant_ready: bool =
        sqlx::query_scalar("SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)")
            .bind(tenant)
            .bind(&attested.command.context_key)
            .bind(request.canonical_run_id)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(
        tenant_ready,
        "source and HITL rows lack authoritative PG tenant ownership"
    );
    let attestation = V4TemporalAttestation {
        parent_workflow_id: request.parent_workflow_id.clone(),
        parent_workflow_run_id: request.parent_workflow_run_id.clone(),
        child_workflow_id: request.child_workflow_id.clone(),
        child_workflow_run_id: request.child_workflow_run_id.clone(),
        parent_child_started_event_id: attested.child_started_event_id,
    };
    let checked = &attested.invocation.checked_dispatch;
    let port = PortId::from(INPUT_PORT);
    let bound = attested
        .invocation
        .bound_inputs
        .get(&port)
        .context("PG owner admission lacked its checked bound input")?;
    let bound_bytes = serde_json::to_vec(bound)?;
    let invocation_sha256 = sha256_pg(&mut tx, &attested.exact_invocation_bytes).await?;
    let bound_sha256 = sha256_pg(&mut tx, &bound_bytes).await?;
    let command_sha256 = sha256_pg(&mut tx, &attested.invocation.command_bytes).await?;
    let attestation_sha256 = sha256_pg(&mut tx, &serde_json::to_vec(&attestation)?).await?;
    let parent_start_sha256 = sha256_pg(&mut tx, &attested.parent_start_bytes).await?;
    let governed: bool = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_governance_matches_v1(
            $1,$2,$3,$4,$5,$6,$7,TRUE)",
    )
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&attested.command.context_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(checked.plan_hash.as_str())
    .bind(&parent_start_sha256)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        governed,
        "independently issued native DAG run governance absent/expired/mismatched"
    );
    let inserted: Uuid = sqlx::query_scalar(
        "INSERT INTO pipeline.pg_effect_v3_admissions
           (admission_id,run_id,tenant_id,context_key,workflow_id,
            workflow_execution_run_id,operation_instance,plan_hash,
            contract_fingerprint,candidate_set_hash,issued_by,expires_at,
            admission_schema_revision,parent_workflow_id,parent_workflow_run_id,
            dispatch_execution_key,dispatch_input_hash,invocation_sha256,
            bound_input_sha256,owner_command_sha256,temporal_attestation_sha256,
            temporal_child_started_event_id,generic_parent_start_sha256)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,current_user,
                clock_timestamp() + ($11::integer * interval '1 second'),
                5,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21
         FROM pipeline.execution_runs er
         WHERE er.run_id=$2 AND er.context_key=$4
           AND er.workflow_type='seo_site_build'
           AND er.status IN ('verifying','pending_hitl')
           AND pipeline.operation_dag_governance_matches_v1(
               er.run_id,$3,$4,$12,$13,$8,$21,TRUE)
         RETURNING admission_id",
    )
    .bind(request.admission_id)
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&attested.command.context_key)
    .bind(&request.child_workflow_id)
    .bind(&request.child_workflow_run_id)
    .bind(checked.instance.as_str())
    .bind(checked.plan_hash.as_str())
    .bind(contract_fingerprint()?)
    .bind(source_hash)
    .bind(request.expires_in_seconds as i32)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(&checked.execution_key)
    .bind(&checked.input_hash)
    .bind(invocation_sha256)
    .bind(bound_sha256)
    .bind(command_sha256)
    .bind(attestation_sha256)
    .bind(attested.child_started_event_id)
    .bind(parent_start_sha256)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        inserted == request.admission_id,
        "PG rev5 admission identity mismatch"
    );
    tx.commit().await?;
    println!(
        "{}",
        serde_json::json!({
            "operation": "issue_pg_owner_v1",
            "admission_id": inserted,
            "status": "issued",
            "checked_execution_key": checked.execution_key,
            "child_workflow_run_id": request.child_workflow_run_id
        })
    );
    Ok(())
}

async fn signal_after_commit(request: &IssueRequest) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")?;
    let client = crate::connect_client(
        &url,
        format!("alegria-pg-owner-signal-{}", std::process::id()),
        &namespace,
    )
    .await?;
    WorkflowExecutionInfo::builder()
        .namespace(namespace)
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build()
        .bind_untyped(client)
        .signal(
            UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_v5"),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("signal PG native child AFTER committed revision-5 admission")
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes =
        fs::read(path).with_context(|| format!("read PG owner locator {}", path.display()))?;
    let request: IssueRequest = serde_json::from_slice(&bytes)
        .context("decode strict locator-only PG owner admission request")?;
    ensure!(
        (1..=MAX_ADMISSION_SECONDS).contains(&request.expires_in_seconds)
            && !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok()
            && request.parent_workflow_id
                == operation_dag_parent_workflow_id(&request.canonical_run_id.to_string())
            && request
                .child_workflow_id
                .starts_with(GENERIC_CHILD_ID_PREFIX),
        "PG owner issue request contains invalid canonical IDs/expiry"
    );
    let build_id = env::var("PG_OWNER_EXPECTED_BUILD_ID")
        .context("PG owner operator must pin explicit trusted build ID")?;
    ensure!(
        !build_id.trim().is_empty() && build_id.trim() == build_id,
        "untrusted blank PG owner build ID"
    );
    let attested = attest_from_server(&request, &build_id).await?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("PG owner operator must configure authoritative tenant")?;
    let (pool, _) =
        super::operator::operator_pool_with_enable_flag("ALEGRIA_ENABLE_PG_OWNER_OPERATOR_V1")
            .await?;
    insert_admission(&pool, &request, &attested, &tenant).await?;
    signal_after_commit(&request)
        .await
        .context("PG rev5 admission committed but Signal failed; manual review required")
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::{Payload, Payloads, WorkflowType},
        history::v1::{HistoryEvent, WorkflowExecutionStartedEventAttributes},
        taskqueue::v1::TaskQueue,
    };

    fn started(workflow_type: &str, queue: &str) -> History {
        History::from(vec![HistoryEvent {
            event_id: 1,
            attributes: Some(Attributes::WorkflowExecutionStartedEventAttributes(
                WorkflowExecutionStartedEventAttributes {
                    workflow_type: Some(WorkflowType {
                        name: workflow_type.into(),
                    }),
                    task_queue: Some(TaskQueue {
                        name: queue.into(),
                        ..Default::default()
                    }),
                    input: Some(Payloads {
                        payloads: vec![Payload {
                            data: b"forged-self-consistent-input".to_vec().into(),
                            ..Default::default()
                        }],
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }])
    }

    #[test]
    fn generic_child_prefix_is_not_a_legacy_pg_workflow() {
        assert_ne!(
            OWNER_OPERATION_CHILD_TYPE_V1,
            "PgTruthReviewReconcileV4Workflow"
        );
        assert_ne!(PG_OWNER_TASK_QUEUE_V1, "alegria-pg-command-v3");
        assert!(GENERIC_CHILD_ID_PREFIX.starts_with("alegria-owner-operation-v1-"));
    }

    #[test]
    fn child_without_server_parent_history_link_must_fail_closed() {
        let id = Uuid::new_v4();
        let request = IssueRequest {
            admission_id: id,
            canonical_run_id: id,
            parent_workflow_id: id.to_string(),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            child_workflow_id: format!("{GENERIC_CHILD_ID_PREFIX}{}", "a".repeat(64)),
            child_workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 60,
        };
        assert!(prove_temporal_histories(
            &started(OPERATION_DAG_PARENT_TYPE, OPERATION_DAG_PARENT_QUEUE),
            &started(OWNER_OPERATION_CHILD_TYPE_V1, PG_OWNER_TASK_QUEUE_V1),
            &request,
            "trusted-build-id"
        )
        .is_err());
    }
}
