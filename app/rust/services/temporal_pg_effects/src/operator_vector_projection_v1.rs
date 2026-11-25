//! Independent PG projection issuer. No downstream provider command, SDK or
//! network is used: the PostgreSQL owner only admits its own real source read
//! after reciprocal native parent/child server History attestation.

use std::{env, fs, path::Path};

use anyhow::{ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id, ArtifactBinding, Operation, OperationDagRunV6,
    OwnerInvocationV1, PortId, OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_TYPE,
    OWNER_OPERATION_CHILD_TYPE_V1,
};
use postgres_store::truth_review_effect_v3::PG_OWNER_TASK_QUEUE_V1;
use postgres_store::vector_projection_owner_v1::{
    validate_pg_vector_projection_invocation_v1, PgReadVectorProjectionV1,
    PgVectorProjectionCommandV1, INPUT_PORT,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowExecutionInfo, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use uuid::Uuid;

use super::operator_v4::{fetch_exact_history, first_started_payload, sha256_pg};

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

struct VerifiedHistory {
    invocation: OwnerInvocationV1,
    command: PgVectorProjectionCommandV1,
    parent_start_bytes: Vec<u8>,
    child_start_bytes: Vec<u8>,
}

fn verify_history_bytes(
    parent: &History,
    child: &History,
    request: &IssueRequest,
    trusted_build_id: &str,
) -> Result<VerifiedHistory> {
    let parent_start_bytes = first_started_payload(
        parent,
        OPERATION_DAG_PARENT_TYPE,
        OPERATION_DAG_PARENT_QUEUE,
    )?;
    let child_start_bytes =
        first_started_payload(child, OWNER_OPERATION_CHILD_TYPE_V1, PG_OWNER_TASK_QUEUE_V1)?;
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_start_bytes)
        .context("PG projection child start is not a typed OwnerInvocationV1")?;
    ensure!(
        serde_json::to_vec(&invocation)? == child_start_bytes
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id
            && request.child_workflow_id
                == operation_protocol::owner_child_workflow_id(
                    &invocation.checked_dispatch.execution_key
                ),
        "PG projection child start bytes/parent/deterministic ID mismatch"
    );
    let command = validate_pg_vector_projection_invocation_v1(&invocation)
        .context("PG projection typed owner command does not match checked dispatch")?;
    ensure!(
        command.admission_id == request.admission_id && command.run_id == request.canonical_run_id,
        "PG projection admission/canonical run not bound to typed command"
    );

    let child_started = child
        .events
        .first()
        .context("empty PG vector child History")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(child_start)) =
        &child_started.attributes
    else {
        anyhow::bail!("PG vector child History lacks actual WorkflowExecutionStarted")
    };
    let actual_parent = child_start
        .parent_workflow_execution
        .as_ref()
        .context("PG projection child has no server-attested parent")?;
    ensure!(
        actual_parent.workflow_id == request.parent_workflow_id
            && actual_parent.run_id == request.parent_workflow_run_id,
        "PG projection Child server parent identity mismatch"
    );
    let mut reciprocal = 0_usize;
    for initiated in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(start)) =
            &initiated.attributes
        else {
            continue;
        };
        if start.workflow_id != request.child_workflow_id {
            continue;
        }
        ensure!(
            start.workflow_type.as_ref().map(|v| v.name.as_str())
                == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                && start.task_queue.as_ref().map(|v| v.name.as_str())
                    == Some(PG_OWNER_TASK_QUEUE_V1)
                && start.input.as_ref().is_some_and(|v| v.payloads.len() == 1
                    && v.payloads[0].data.as_slice() == child_start_bytes.as_slice()),
            "PG projection parent scheduled different child workflow/queue/bytes"
        );
        for event in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(started)) =
                &event.attributes
            else {
                continue;
            };
            if started.initiated_event_id != initiated.event_id {
                continue;
            }
            let observed = started
                .workflow_execution
                .as_ref()
                .context("PG parent ChildStarted has no server execution")?;
            ensure!(
                observed.workflow_id == request.child_workflow_id
                    && observed.run_id == request.child_workflow_run_id
                    && child_start.parent_initiated_event_id == initiated.event_id
                    && started.workflow_type.as_ref().map(|v| v.name.as_str())
                        == Some(OWNER_OPERATION_CHILD_TYPE_V1),
                "PG projection parent/child reciprocal server identity mismatch"
            );
            reciprocal += 1;
        }
    }
    ensure!(
        reciprocal == 1,
        "PG projection needs one reciprocal native ChildStarted"
    );

    let run: OperationDagRunV6 = serde_json::from_slice(&parent_start_bytes)?;
    ensure!(
        run.schema_revision == 6
            && serde_json::to_vec(&run)? == parent_start_bytes
            && run.plan.hash == run.plan.canonical_hash()?,
        "PG vector parent lacks canonical versioned DAG start"
    );
    let checked = &invocation.checked_dispatch;
    let contract = PgReadVectorProjectionV1::contract();
    let contract_hash = contract.fingerprint()?;
    ensure!(
        checked.operation == contract.reference
            && checked.owner == contract.owner
            && checked.effects == contract.effects
            && checked.idempotency == contract.idempotency
            && checked.plan_hash == run.plan.hash
            && checked.scope == run.plan.scope
            && checked.scope.run_id == request.canonical_run_id.to_string(),
        "PG producer command is not immutable parent plan dispatch"
    );
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .any(|p| p.reference == contract.reference && p.fingerprint == contract_hash),
        "PG producer exact contract fingerprint absent in parent History"
    );
    let owner_wire = run
        .owner_commands
        .get(&checked.instance)
        .context("PG vector command not present in parent start")?;
    ensure!(
        owner_wire.operation == checked.operation
            && owner_wire.owner == checked.owner
            && owner_wire.contract_hash == contract_hash
            && owner_wire.command_bytes == invocation.command_bytes,
        "PG vector owner command differs from immutable parent start"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|n| n.instance == checked.instance)
        .context("PG vector operation instance missing from parent DAG")?;
    ensure!(
        node.operation == checked.operation
            && node.permitted_effects == checked.effects
            && run.plan.allowed_effects.is_superset(&checked.effects)
            && node.inputs.len() == invocation.bound_inputs.len()
            && node.inputs.iter().all(|(port, binding)| invocation
                .bound_inputs
                .get(port)
                .is_some_and(|bound| &bound.source == binding)),
        "PG vector source binding/effect violates immutable parent plan"
    );
    let bound = invocation
        .bound_inputs
        .get(&PortId(INPUT_PORT.into()))
        .context("PG vector external lookup not bound")?;
    let ArtifactBinding::External { artifact } = &bound.source else {
        anyhow::bail!("PG vector producer cannot consume unproven Produced data")
    };
    ensure!(
        artifact == &bound.artifact_id && run.admitted.get(artifact) == Some(&bound.artifact),
        "PG vector bound lookup bytes differ from immutable parent input"
    );
    let route = run
        .pinned_routes
        .iter()
        .find(|r| r.operation == checked.operation && r.owner == checked.owner)
        .context("parent did not pin PG vector producer route")?;
    ensure!(
        route.contract_hash == contract_hash
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && route.task_queue == PG_OWNER_TASK_QUEUE_V1
            && route.build_id == trusted_build_id
            && run
                .pinned_routes
                .iter()
                .filter(|r| r.operation == checked.operation)
                .count()
                == 1,
        "PG vector route is not trusted worker Build ID/type/queue"
    );
    Ok(VerifiedHistory {
        invocation,
        command,
        parent_start_bytes,
        child_start_bytes,
    })
}

async fn attest_from_server(request: &IssueRequest, build_id: &str) -> Result<VerifiedHistory> {
    let url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")
        .context("PG projection operator must configure independent Temporal URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")
        .context("PG projection operator must configure exact namespace")?;
    super::operator_dag_governance::validate_local_temporal_endpoint(&url)?;
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "invalid Temporal namespace"
    );
    let client = temporal_gateway::connect_client(
        &url,
        format!("alegria-pg-projection-issuer-{}", std::process::id()),
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
    verify_history_bytes(&parent, &child, request, build_id)
}

async fn insert_admission(
    pool: &PgPool,
    request: &IssueRequest,
    attested: &VerifiedHistory,
    tenant: &str,
) -> Result<()> {
    super::operator::enforce_authoritative_pg_tenant(pool, tenant).await?;
    ensure!(
        attested.command.scope.tenant_id == tenant,
        "PG vector source tenant does not match authenticated operator LOGIN"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
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
    let run = sqlx::query(
        "SELECT context_key,workflow_type,status FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("PG vector producer canonical Truth source run missing")?;
    ensure!(
        run.get::<String, _>("context_key") == attested.command.context_key
            && run.get::<String, _>("workflow_type") == "seo_site_build"
            && matches!(
                run.get::<String, _>("status").as_str(),
                "verifying" | "pending_hitl"
            ),
        "PG source is not an authorized active canonical business run"
    );
    let parent_sha = sha256_pg(&mut tx, &attested.parent_start_bytes).await?;
    let governed: bool = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_governance_matches_v1(
            $1,$2,$3,$4,$5,$6,$7,TRUE)",
    )
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&attested.command.context_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(&attested.invocation.checked_dispatch.plan_hash.0)
    .bind(&parent_sha)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        governed,
        "PG projection generic parent governance missing/expired"
    );
    let source = sqlx::query(
        "SELECT vector FROM pipeline.vector_projection_sources_v1
         WHERE run_id=$1 AND tenant_id=$2 AND context_key=$3 AND keyword_key=$4",
    )
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&attested.command.context_key)
    .bind(&attested.command.keyword_key)
    .fetch_optional(&mut *tx)
    .await?
    .context("PG vector precomputed authoritative source row missing")?;
    let vector: serde_json::Value = source.try_get("vector")?;
    let source_hash: String = sqlx::query_scalar(
        "SELECT encode(public.digest(convert_to(
            $1::text || '|' || $2::text || '|' || $3::text || '|' ||
            $4::text || '|' || $5::jsonb::text,'UTF8'),'sha256'),'hex')",
    )
    .bind(tenant)
    .bind(request.canonical_run_id.to_string())
    .bind(&attested.command.context_key)
    .bind(&attested.command.keyword_key)
    .bind(vector)
    .fetch_one(&mut *tx)
    .await?;
    let invocation_sha = sha256_pg(&mut tx, &attested.child_start_bytes).await?;
    let checked = &attested.invocation.checked_dispatch;
    let fingerprint = PgReadVectorProjectionV1::contract().fingerprint()?;
    let inserted = sqlx::query(
        "INSERT INTO pipeline.pg_vector_projection_admissions_v1
            (admission_id,run_id,tenant_id,context_key,keyword_key,operation_instance,
             plan_hash,contract_fingerprint,dispatch_execution_key,
             parent_workflow_id,parent_workflow_run_id,child_workflow_id,
             child_workflow_run_id,invocation_sha256,generic_parent_start_sha256,
             source_hash,issued_by,expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                current_user,clock_timestamp()+($17::integer*interval '1 second')
         WHERE pipeline.operation_dag_governance_matches_v1($2,$3,$4,$10,$11,$7,$15,TRUE)",
    )
    .bind(request.admission_id)
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&attested.command.context_key)
    .bind(&attested.command.keyword_key)
    .bind(&attested.command.operation_instance)
    .bind(&attested.command.plan_hash)
    .bind(fingerprint.as_str())
    .bind(&checked.execution_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(&request.child_workflow_id)
    .bind(&request.child_workflow_run_id)
    .bind(invocation_sha)
    .bind(parent_sha)
    .bind(source_hash)
    .bind(request.expires_in_seconds as i32)
    .execute(&mut *tx)
    .await?;
    ensure!(
        inserted.rows_affected() == 1,
        "PG vector source admission must insert exactly one row"
    );
    tx.commit().await?;
    println!(
        "{}",
        serde_json::json!({
            "operation": "issue_pg_vector_projection_v1",
            "admission_id": request.admission_id,
            "status": "issued",
            "checked_execution_key": checked.execution_key,
            "child_workflow_run_id": request.child_workflow_run_id,
        })
    );
    Ok(())
}

async fn signal_after_commit(request: &IssueRequest) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")?;
    let client = temporal_gateway::connect_client(
        &url,
        format!("alegria-pg-vector-signal-{}", std::process::id()),
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
        .context("PG vector source admission committed but signal failed; reconcile manually")
}

/// The projection issuer has a different SQL authority than the historical
/// Truth V3 issuer. Never reuse the latter's role validator: it permits the
/// V3 admission table and necessarily rejects this separate owner admission.
/// This role may INSERT only its own immutable projection admission and read
/// only the PG source + canonical governance required to issue that admission.
async fn projection_issuer_pool() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_PG_OWNER_OPERATOR_V1").as_deref() == Ok("true"),
        "PG vector source operator requires explicit owner-only enable flag"
    );
    let url = env::var("PG_EFFECT_OPERATOR_DATABASE_URL")
        .context("PG vector operator requires separately provisioned operator DSN")?;
    ensure!(
        !url.is_empty() && url.trim() == url,
        "PG vector operator DSN is empty or has surrounding whitespace"
    );
    let parsed = url::Url::parse(&url).context("PG vector operator DSN is malformed")?;
    ensure!(
        matches!(parsed.scheme(), "postgres" | "postgresql")
            && !parsed.username().is_empty()
            && parsed.host_str().is_some()
            && !parsed.path().is_empty()
            && parsed.path() != "/"
            && parsed.fragment().is_none(),
        "PG vector operator DSN must identify one explicit PostgreSQL login and database"
    );
    for key in [
        "PG_EFFECT_WORKER_DATABASE_URL",
        "OPERATION_DAG_GOVERNANCE_DATABASE_URL",
        "PG_DAG_GOVERNANCE_OPERATOR_DATABASE_URL",
    ] {
        if let Ok(other) = env::var(key) {
            ensure!(
                url != other,
                "PG vector operator must not reuse {key} credentials"
            );
        }
    }
    let pool = PgPool::connect(&url)
        .await
        .context("connect isolated PG vector operator")?;
    let identity = sqlx::query(
        "SELECT current_user::text AS role_name,
                session_user::text AS login_role,
                r.rolcanlogin AS can_login,
                (r.rolsuper OR r.rolcreaterole OR r.rolcreatedb
                 OR r.rolreplication OR r.rolbypassrls) AS elevated,
                EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m
                        WHERE m.member = r.oid) AS has_memberships
         FROM pg_catalog.pg_roles r WHERE r.rolname = current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        identity.get::<String, _>("role_name") == identity.get::<String, _>("login_role")
            && identity.get::<bool, _>("can_login")
            && !identity.get::<bool, _>("elevated")
            && !identity.get::<bool, _>("has_memberships"),
        "PG vector operator requires a restricted standalone LOGIN with no SET ROLE memberships"
    );
    let can_issue: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege(current_user,'pipeline','USAGE')
           AND has_table_privilege(current_user,'pipeline.pg_vector_projection_admissions_v1','INSERT')
           AND has_table_privilege(current_user,'pipeline.vector_projection_sources_v1','SELECT')
           AND has_table_privilege(current_user,'pipeline.execution_runs','SELECT')
           AND has_function_privilege(current_user,
                'pipeline.operation_dag_governance_matches_v1(uuid,text,text,text,text,text,text,boolean)',
                'EXECUTE')",
    ).fetch_one(&pool).await?;
    ensure!(
        can_issue,
        "PG vector operator lacks exact source-read, governance and own-admission capabilities"
    );
    let excess_table: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (
             (has_any_column_privilege(current_user,c.oid,'INSERT')
              AND NOT (n.nspname = 'pipeline'
                       AND c.relname = 'pg_vector_projection_admissions_v1'))
             OR has_any_column_privilege(current_user,c.oid,'UPDATE')
             OR has_table_privilege(current_user,c.oid,'DELETE')
             OR has_table_privilege(current_user,c.oid,'TRUNCATE')
           )
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        excess_table.is_none(),
        "PG vector operator has forbidden foreign write authority: {excess_table:?}"
    );
    let excess_sequence: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind = 'S'
           AND (has_sequence_privilege(current_user,c.oid,'USAGE')
             OR has_sequence_privilege(current_user,c.oid,'UPDATE'))
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        excess_sequence.is_none(),
        "PG vector operator has forbidden sequence mutation authority: {excess_sequence:?}"
    );
    let excess_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname FROM pg_catalog.pg_namespace n
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND has_schema_privilege(current_user,n.oid,'CREATE')
         ORDER BY n.nspname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        excess_schema.is_none(),
        "PG vector operator has forbidden schema CREATE authority: {excess_schema:?}"
    );
    Ok(pool)
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("read PG vector projection locator {}", path.display()))?;
    let request: IssueRequest = serde_json::from_slice(&bytes)
        .context("decode strict PG vector projection locator-only request")?;
    ensure!(
        (1..=300).contains(&request.expires_in_seconds)
            && !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok()
            && request.parent_workflow_id
                == operation_dag_parent_workflow_id(&request.canonical_run_id.to_string())
            && request
                .child_workflow_id
                .starts_with("alegria-owner-operation-v1-"),
        "invalid PG projection operator locator/expiry"
    );
    let build_id = env::var("PG_OWNER_EXPECTED_BUILD_ID")
        .context("PG projection operator needs trusted deployed source SHA")?;
    ensure!(
        build_id.len() == 40 && build_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "PG projection owner Build ID must be exact 40-hex source SHA"
    );
    let attested = attest_from_server(&request, &build_id).await?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("PG projection operator must pin authenticated tenant")?;
    let pool = projection_issuer_pool().await?;
    insert_admission(&pool, &request, &attested, &tenant).await?;
    signal_after_commit(&request).await
}
