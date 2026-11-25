//! V4 operator-only parent/child Temporal provenance before PostgreSQL admission.
//!
//! Inputs are locators, NEVER bearer proof: independently query the exact-run
//! histories from a configured Temporal service and bind server-observed
//! parent->child initiation/start, child->parent startup, and actual payloads.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    ArtifactBinding, ArtifactId, ArtifactSchema, EffectChildInvocationV4, ExecutionPlan,
    OperationInstanceId, RunScope,
};
use postgres_store::truth_review_effect_v3::{
    contract_fingerprint, ReconcileTruthReviewsInput, V4TemporalAttestation, CANDIDATE_SET_HASH_SQL,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowDescribeOptions, WorkflowExecutionInfo,
    WorkflowFetchHistoryOptions, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use truth_domain::review_effect_contract::{
    operation_contract, TruthReviewScopeArtifactV1, INPUT_PORT,
};
use uuid::Uuid;

const MAX_V4_ADMISSION_SECONDS: u32 = 300;
const PARENT_TYPE: &str = "AlegriaAdmittedEffectDagV2Workflow";
const PARENT_QUEUE: &str = "alegria-effect-dag-v4";
const CHILD_TYPE: &str = "PgTruthReviewReconcileV4Workflow";
const CHILD_QUEUE: &str = "alegria-pg-command-v3";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V4IssueRequest {
    admission_id: Uuid,
    canonical_run_id: Uuid,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    child_workflow_id: String,
    child_workflow_run_id: String,
    expires_in_seconds: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentRunInputV4 {
    plan: ExecutionPlan,
    admitted: BTreeMap<ArtifactId, ParentArtifactValue>,
    owner_commands: BTreeMap<OperationInstanceId, ParentOwnerCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentArtifactValue {
    schema: ArtifactSchema,
    scope: RunScope,
    provenance: String,
    payload: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum ParentOwnerCommand {
    PgTruthReviewReconcileV1(ReconcileTruthReviewsInput),
}

#[derive(Debug)]
struct ServerAttestedChild {
    invocation: EffectChildInvocationV4,
    exact_invocation_bytes: Vec<u8>,
    parent_child_started_event_id: i64,
    parent_initiated_event_id: i64,
    command: ReconcileTruthReviewsInput,
}

fn first_started_payload(
    history: &History,
    expected_type: &str,
    expected_queue: &str,
) -> Result<Vec<u8>> {
    let first = history
        .events
        .first()
        .context("Temporal history has no start event")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(start)) = &first.attributes else {
        bail!("Temporal exact-run history first event is not WorkflowExecutionStarted");
    };
    ensure!(
        start.workflow_type.as_ref().map(|t| t.name.as_str()) == Some(expected_type),
        "Temporal Workflow type does not match V4 native owner"
    );
    ensure!(
        start.task_queue.as_ref().map(|q| q.name.as_str()) == Some(expected_queue),
        "Temporal Workflow started on unexpected Task Queue"
    );
    let inputs = &start
        .input
        .as_ref()
        .context("Temporal Workflow start has no input")?
        .payloads;
    ensure!(
        inputs.len() == 1,
        "V4 child and parent must each have one typed Temporal start payload"
    );
    ensure!(
        inputs[0].data.len() <= 2 * 1024 * 1024,
        "V4 Temporal start payload exceeds the operator proof budget"
    );
    Ok(inputs[0].data.to_vec())
}

fn prove_parent_plan_and_command(
    parent_bytes: &[u8],
    invocation: &EffectChildInvocationV4,
    expected_canonical_run: Uuid,
) -> Result<ReconcileTruthReviewsInput> {
    invocation
        .verify_integrity()
        .context("child actual input/dispatch hash was forged")?;
    let parent: ParentRunInputV4 = serde_json::from_slice(parent_bytes)
        .context("parent did not start with typed EffectDagRunV1")?;
    ensure!(
        parent.plan.hash == parent.plan.canonical_hash()?,
        "parent sealed DAG hash is invalid"
    );
    ensure!(
        parent.plan.hash == invocation.checked_dispatch.plan_hash
            && parent.plan.scope == invocation.checked_dispatch.scope
            && parent.plan.scope.run_id == expected_canonical_run.to_string(),
        "child checked dispatch does not belong to canonical parent plan"
    );
    let contract = operation_contract();
    let checked = &invocation.checked_dispatch;
    let expected_fingerprint = contract.fingerprint()?;
    ensure!(
        checked.operation == contract.reference
            && checked.owner == contract.owner
            && checked.effects == contract.effects
            && checked.idempotency == contract.idempotency,
        "V4 owner dispatch is not the fixed Truth-review operation contract"
    );
    ensure!(
        parent
            .plan
            .pinned_contracts
            .iter()
            .any(|p| p.reference == checked.operation && p.fingerprint == expected_fingerprint),
        "parent plan did not pin this exact owner contract"
    );
    let node = parent
        .plan
        .nodes
        .iter()
        .find(|n| n.instance == checked.instance)
        .context("checked V4 node not present in parent DAG")?;
    ensure!(
        node.operation == checked.operation
            && node.permitted_effects == checked.effects
            && parent.plan.allowed_effects.is_superset(&checked.effects),
        "parent DAG did not authorize this exact node and effect set"
    );
    let Some(ArtifactBinding::External { artifact }) = node
        .inputs
        .get(&operation_protocol::PortId::from(INPUT_PORT))
    else {
        bail!("parent V4 effect input is not an externally bound artifact");
    };
    ensure!(
        artifact == &invocation.bound_input.artifact_id,
        "child bound artifact is not the parent node's exact input"
    );
    let parent_artifact = parent
        .admitted
        .get(artifact)
        .context("parent History did not admit this source artifact")?;
    ensure!(
        parent_artifact.schema == invocation.bound_input.schema
            && parent_artifact.scope == invocation.bound_input.scope
            && parent_artifact.provenance == invocation.bound_input.provenance
            && parent_artifact.payload == invocation.bound_input.payload,
        "child effect artifact bytes/provenance differ from parent History"
    );
    let Some(ParentOwnerCommand::PgTruthReviewReconcileV1(parent_command)) =
        parent.owner_commands.get(&checked.instance)
    else {
        bail!("parent History omitted typed Truth-review owner command");
    };
    let command: ReconcileTruthReviewsInput =
        serde_json::from_slice(&invocation.owner_command_bytes)
            .context("child owner command is not typed Truth-review JSON")?;
    ensure!(
        &command == parent_command
            && command.run_id == expected_canonical_run
            && command.scope == checked.scope
            && command.operation_instance == checked.instance.as_str()
            && command.plan_hash == checked.plan_hash.as_str(),
        "child owner command differs from canonical parent History"
    );
    let scope: TruthReviewScopeArtifactV1 = serde_json::from_slice(&invocation.bound_input.payload)
        .context("child bound input is not Truth-review scope artifact")?;
    ensure!(
        scope.run_id == command.run_id && scope.context_key == command.context_key,
        "child bound artifact does not describe its typed canonical Truth run"
    );
    Ok(command)
}

fn prove_reciprocal_temporal_histories(
    parent: &History,
    child: &History,
    request: &V4IssueRequest,
) -> Result<ServerAttestedChild> {
    let parent_input = first_started_payload(parent, PARENT_TYPE, PARENT_QUEUE)?;
    let child_input = first_started_payload(child, CHILD_TYPE, CHILD_QUEUE)?;
    let invocation: EffectChildInvocationV4 = serde_json::from_slice(&child_input)
        .context("child Workflow History contains invalid V4 invocation bytes")?;
    ensure!(serde_json::to_vec(&invocation)? == child_input,
        "V4 child History payload is not canonical typed JSON; SQL executor would hash different bytes");
    ensure!(
        request.child_workflow_id
            == format!(
                "alegria-pg-effect-v4-{}",
                invocation.checked_dispatch.execution_key,
            ),
        "V4 child Workflow ID does not follow the deterministic checked execution key identity"
    );
    ensure!(
        invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id,
        "child input declared different Temporal parent than independent parent locator"
    );
    let child_start = child.events.first().context("missing child start")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(child_attributes)) =
        &child_start.attributes
    else {
        bail!("child did not start as WorkflowExecutionStarted");
    };
    let actual_parent = child_attributes
        .parent_workflow_execution
        .as_ref()
        .context("child Temporal start has no server-attested parent")?;
    ensure!(
        actual_parent.workflow_id == request.parent_workflow_id
            && actual_parent.run_id == request.parent_workflow_run_id,
        "child server-attested parent identity mismatches requested parent execution"
    );
    let mut unique_match = None;
    for initiated in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs)) =
            &initiated.attributes
        else {
            continue;
        };
        if attrs.workflow_id != request.child_workflow_id {
            continue;
        }
        ensure!(
            attrs.workflow_type.as_ref().map(|t| t.name.as_str()) == Some(CHILD_TYPE)
                && attrs.task_queue.as_ref().map(|q| q.name.as_str()) == Some(CHILD_QUEUE),
            "parent initiated wrong child Workflow Type or Queue"
        );
        let payloads = &attrs
            .input
            .as_ref()
            .context("parent child-initiation event has no input")?
            .payloads;
        ensure!(
            payloads.len() == 1 && payloads[0].data.as_slice() == child_input.as_slice(),
            "parent child-initiation payload differs from child's actual History payload"
        );
        for started in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(child_started)) =
                &started.attributes
            else {
                continue;
            };
            if child_started.initiated_event_id != initiated.event_id {
                continue;
            }
            let actual_child = child_started
                .workflow_execution
                .as_ref()
                .context("parent ChildStarted lacked server child execution")?;
            ensure!(
                actual_child.workflow_id == request.child_workflow_id
                    && actual_child.run_id == request.child_workflow_run_id
                    && child_started
                        .workflow_type
                        .as_ref()
                        .map(|t| t.name.as_str())
                        == Some(CHILD_TYPE),
                "parent ChildStarted was a different server child execution"
            );
            ensure!(
                child_attributes.parent_initiated_event_id == initiated.event_id,
                "child parent_initiated_event_id does not match parent initiation"
            );
            ensure!(
                unique_match
                    .replace((initiated.event_id, started.event_id))
                    .is_none(),
                "ambiguous duplicate child-start match in parent Temporal History"
            );
        }
    }
    let (parent_initiated_event_id, parent_child_started_event_id) = unique_match
        .context("no reciprocal server-generated parent ChildStarted/child Started event pair")?;
    let command =
        prove_parent_plan_and_command(&parent_input, &invocation, request.canonical_run_id)?;
    ensure!(
        command.admission_id == request.admission_id,
        "operator admission UUID differs from actual child History's typed command"
    );
    Ok(ServerAttestedChild {
        invocation,
        exact_invocation_bytes: child_input,
        parent_child_started_event_id,
        parent_initiated_event_id,
        command,
    })
}

async fn fetch_exact_history(
    client: &temporalio_client::Client,
    namespace: &str,
    workflow_id: &str,
    run_id: &str,
) -> Result<History> {
    let info = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(workflow_id.to_owned())
        .run_id(run_id.to_owned())
        .build();
    let handle = info.bind_untyped(client.clone());
    // Describe establishes this server execution exists; History is still
    // mandatory because Describe alone cannot prove its native child link,
    // checked payload or exact parent initiated-event correspondence.
    handle
        .describe(WorkflowDescribeOptions::builder().build())
        .await
        .context("Temporal Describe exact-run failed")?;
    let events = handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events()
        .await
        .context("Temporal exact-run server History fetch failed")?;
    ensure!(
        events.len() <= 10_000,
        "Temporal operator History exceeds the V4 event proof budget"
    );
    Ok(History::from(events))
}

async fn attest_temporal_child(request: &V4IssueRequest) -> Result<ServerAttestedChild> {
    let temporal_url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")
        .context("V4 issuer requires independently configured Temporal server URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")
        .context("V4 issuer requires its exact Temporal namespace")?;
    ensure!(
        !namespace.trim().is_empty() && namespace.trim() == namespace,
        "Temporal namespace is blank or contains whitespace"
    );
    // Without a separately authenticated TLS client configuration, only an
    // operator-side localhost control endpoint is trusted for development.
    // Production remote TLS/mTLS credential provisioning remains a NO-GO.
    ensure!(
        temporal_url.starts_with("http://127.0.0.1:")
            || temporal_url.starts_with("http://localhost:"),
        "V4 operator currently requires a trusted LOCAL Temporal endpoint; remote auth is not provisioned"
    );
    let client = temporal_gateway::connect_client(
        &temporal_url,
        format!("alegria-pg-v4-operator-proof-{}", std::process::id()),
        &namespace,
    )
    .await
    .context("connect independently configured Temporal proof server")?;
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
    prove_reciprocal_temporal_histories(&parent, &child, request)
}

async fn signal_after_commit(request: &V4IssueRequest) -> Result<()> {
    use temporalio_common::data_converters::RawValue;

    let temporal_url = env::var("PG_EFFECT_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE")?;
    let client = temporal_gateway::connect_client(
        &temporal_url,
        format!("alegria-pg-v4-operator-signal-{}", std::process::id()),
        &namespace,
    )
    .await?;
    let child_handle = WorkflowExecutionInfo::builder()
        .namespace(namespace)
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build()
        .bind_untyped(client);
    child_handle
        .signal(
            UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_v4"),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("send V4 child admission wake-up AFTER committed PostgreSQL record")
}

async fn locked_source_hash(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    context_key: &str,
) -> Result<String> {
    // Mirror the V3 executor's admission/source locking. A section's parent
    // page can change before its initial page lock is acquired; the exact
    // post-lock relation re-read prevents hashing a newly *unlocked* page.
    const REFS: &str = "SELECT DISTINCT s.id AS section_id, s.page_id
        FROM extracted.rule_candidates c JOIN raw.sections s
          ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = $1 ORDER BY s.id, s.page_id";
    let initial = sqlx::query(REFS)
        .bind(context_key)
        .fetch_all(&mut **tx)
        .await?;
    let mut pairs = BTreeSet::new();
    let mut pages = BTreeSet::new();
    let mut sections = BTreeSet::new();
    for row in initial {
        let section_id = row.get::<i64, _>("section_id");
        let page_id = row.get::<i64, _>("page_id");
        pairs.insert((section_id, page_id));
        pages.insert(page_id);
        sections.insert(section_id);
    }
    for page_id in pages {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|pages|{page_id}"))
            .execute(&mut **tx)
            .await?;
    }
    for section_id in sections {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|sections|{section_id}"))
            .execute(&mut **tx)
            .await?;
    }
    let post = sqlx::query(REFS)
        .bind(context_key)
        .fetch_all(&mut **tx)
        .await?;
    let observed: BTreeSet<(i64, i64)> = post
        .into_iter()
        .map(|row| {
            (
                row.get::<i64, _>("section_id"),
                row.get::<i64, _>("page_id"),
            )
        })
        .collect();
    ensure!(
        observed == pairs,
        "V4 operator raw page/section associations changed before source locks"
    );
    sqlx::query_scalar(CANDIDATE_SET_HASH_SQL)
        .bind(context_key)
        .fetch_one(&mut **tx)
        .await
        .context("compute authoritative PostgreSQL Truth source fingerprint")
}

async fn sha256_pg(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, bytes: &[u8]) -> Result<String> {
    sqlx::query_scalar("SELECT encode(digest($1::bytea, 'sha256'), 'hex')")
        .bind(bytes)
        .fetch_one(&mut **tx)
        .await
        .context("derive authoritative SHA-256 source/History bytes")
}

async fn insert_attested_admission(
    pool: &PgPool,
    role: &str,
    request: &V4IssueRequest,
    attested: &ServerAttestedChild,
    tenant: &str,
) -> Result<()> {
    ensure!(
        tenant == attested.command.scope.tenant_id && !tenant.trim().is_empty(),
        "V4 issuer configured tenant policy differs from server History scope"
    );
    ensure!(
        attested.invocation.checked_dispatch.scope == attested.command.scope,
        "V4 parent dispatch tenant/run differs from exact child typed command"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{}", attested.command.context_key))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-run-status|{}", request.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    let run = sqlx::query(
        "SELECT context_key,workflow_run_id,workflow_type,status
         FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("V4 operator canonical parent pipeline run not found")?;
    ensure!(
        run.get::<String, _>("context_key") == attested.command.context_key
            && run.get::<String, _>("workflow_run_id") == request.parent_workflow_run_id
            && run.get::<String, _>("workflow_type") == "seo_site_build"
            && matches!(
                run.get::<String, _>("status").as_str(),
                "verifying" | "pending_hitl"
            ),
        "V4 server-attested parent does not match live canonical PostgreSQL run"
    );
    let source_hash = locked_source_hash(&mut tx, &attested.command.context_key).await?;
    let attestation = V4TemporalAttestation {
        parent_workflow_id: request.parent_workflow_id.clone(),
        parent_workflow_run_id: request.parent_workflow_run_id.clone(),
        child_workflow_id: request.child_workflow_id.clone(),
        child_workflow_run_id: request.child_workflow_run_id.clone(),
        parent_child_started_event_id: attested.parent_child_started_event_id,
    };
    let attestation_bytes = serde_json::to_vec(&attestation)?;
    let bound_bytes = serde_json::to_vec(&attested.invocation.bound_input)?;
    let invocation_sha256 = sha256_pg(&mut tx, &attested.exact_invocation_bytes).await?;
    let bound_sha256 = sha256_pg(&mut tx, &bound_bytes).await?;
    let command_sha256 = sha256_pg(&mut tx, &attested.invocation.owner_command_bytes).await?;
    let attestation_sha256 = sha256_pg(&mut tx, &attestation_bytes).await?;
    let checked = &attested.invocation.checked_dispatch;
    let inserted: Uuid = sqlx::query_scalar(
        "INSERT INTO pipeline.pg_effect_v3_admissions
           (admission_id,run_id,tenant_id,context_key,workflow_id,
            workflow_execution_run_id,operation_instance,plan_hash,
            contract_fingerprint,candidate_set_hash,issued_by,expires_at,
            admission_schema_revision,parent_workflow_id,parent_workflow_run_id,
            dispatch_execution_key,dispatch_input_hash,invocation_sha256,
            bound_input_sha256,owner_command_sha256,temporal_attestation_sha256,
            temporal_child_started_event_id)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,current_user,
                clock_timestamp() + ($11::integer * interval '1 second'),
                4,$12,$13,$14,$15,$16,$17,$18,$19,$20
         FROM pipeline.execution_runs er
         WHERE er.run_id=$2 AND er.context_key=$4 AND
               er.workflow_run_id=$13 AND er.workflow_type='seo_site_build'
           AND er.status IN ('verifying','pending_hitl')
         RETURNING admission_id",
    )
    .bind(request.admission_id)
    .bind(request.canonical_run_id)
    .bind(&checked.scope.tenant_id)
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
    .bind(attested.parent_child_started_event_id)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        inserted == request.admission_id,
        "V4 admission SQL returned an unexpected identity"
    );
    tx.commit().await?;
    println!(
        "{}",
        serde_json::json!({
            "operation": "issue_admission_v4",
            "admission_id": inserted,
            "status": "issued",
            "operator_role": role,
            "parent_workflow_run_id": request.parent_workflow_run_id,
            "child_workflow_run_id": request.child_workflow_run_id,
            "checked_execution_key": checked.execution_key,
        })
    );
    Ok(())
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes =
        fs::read(path).with_context(|| format!("read V4 operator request {}", path.display()))?;
    let request: V4IssueRequest =
        serde_json::from_slice(&bytes).context("decode strict locator-only V4 operator request")?;
    ensure!(
        request.expires_in_seconds >= 1 && request.expires_in_seconds <= MAX_V4_ADMISSION_SECONDS,
        "V4 admission expiry must be 1..=300 seconds"
    );
    ensure!(
        !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok()
            && request.parent_workflow_id == request.canonical_run_id.to_string()
            && !request.child_workflow_id.trim().is_empty(),
        "V4 request UUID or server Temporal Run ID is invalid"
    );
    // Not an authorization oracle: identifiers in the file are only lookups.
    // Every claimed parent/child identity and command is re-established from
    // server Histories. A syntactically valid forged manifest is insufficient.
    let attested = attest_temporal_child(&request).await?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("V4 issuer requires configured operator tenant policy")?;
    let (pool, role) =
        super::operator::operator_pool_with_enable_flag("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V4")
            .await?;
    insert_attested_admission(&pool, &role, &request, &attested, &tenant).await?;
    // Signaling is only a wake-up: the Activity later consumes the pinned PG
    // admission under the database source/receipt fence. A lost Signal ACK
    // cannot roll back admission; the operator must review the emitted row.
    signal_after_commit(&request).await
        .context("V4 admission COMMITTED but Temporal child signal failed; recover via verified manual signal")
}

enum RevocationOutcomeV4 {
    Busy,
    Committed(&'static str),
}

async fn revoke_once_v4(
    pool: &PgPool,
    admission_id: Uuid,
    tenant: &str,
) -> Result<RevocationOutcomeV4> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    let acquired: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("pg-v3-admission|{admission_id}"))
            .fetch_one(&mut *tx)
            .await?;
    if !acquired {
        return Ok(RevocationOutcomeV4::Busy);
    }
    let receipt_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pipeline.pg_effect_v3_receipts r
          JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id = r.admission_id
          WHERE r.admission_id=$1 AND a.tenant_id=$2 AND a.admission_schema_revision=4)",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        !receipt_exists,
        "V4 effect already committed; revocation cannot undo it"
    );
    let changed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE pipeline.pg_effect_v3_admissions
         SET revoked_at=clock_timestamp()
         WHERE admission_id=$1 AND tenant_id=$2 AND admission_schema_revision=4
           AND revoked_at IS NULL
           AND NOT EXISTS(SELECT 1 FROM pipeline.pg_effect_v3_receipts r
                          WHERE r.admission_id=$1)
         RETURNING admission_id",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_optional(&mut *tx)
    .await?;
    if changed.is_some() {
        tx.commit().await?;
        return Ok(RevocationOutcomeV4::Committed("revoked"));
    }
    let existing: Option<bool> = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM pipeline.pg_effect_v3_admissions
         WHERE admission_id=$1 AND tenant_id=$2 AND admission_schema_revision=4",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_optional(&mut *tx)
    .await?;
    match existing {
        Some(true) => Ok(RevocationOutcomeV4::Committed("already_revoked")),
        Some(false) => bail!("V4 admission could not be revoked (possible committed receipt)"),
        None => bail!("V4 admission absent for this operator tenant/revision"),
    }
}

pub async fn revoke_by_id(id: &str) -> Result<()> {
    let admission_id = Uuid::parse_str(id).context("V4 operator revoke requires UUID")?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("V4 operator revoke requires configured tenant policy")?;
    ensure!(
        !tenant.trim().is_empty() && tenant.trim() == tenant,
        "V4 operator tenant policy is invalid"
    );
    let (pool, role) =
        super::operator::operator_pool_with_enable_flag("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V4")
            .await?;
    for attempt in 0..=3 {
        match revoke_once_v4(&pool, admission_id, &tenant).await {
            Ok(RevocationOutcomeV4::Committed(status)) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "operation": "revoke_admission_v4",
                        "admission_id": admission_id,
                        "operator_role": role,
                        "status": status,
                    })
                );
                return Ok(());
            }
            Ok(RevocationOutcomeV4::Busy) if attempt < 3 => {
                tokio::time::sleep(Duration::from_millis(25u64 << attempt)).await;
            }
            Ok(RevocationOutcomeV4::Busy) => {
                bail!("V4 effect in flight; revocation BUSY, no outcome confirmed");
            }
            Err(error) => {
                if let Some(sqlx::Error::Database(database)) = error.downcast_ref::<sqlx::Error>() {
                    if database.code().as_deref() == Some("55P03") && attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(25u64 << attempt)).await;
                        continue;
                    }
                }
                return Err(error);
            }
        }
    }
    bail!("V4 revocation retry budget exhausted without confirmed outcome")
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::{Payload, Payloads, WorkflowType},
        history::v1::{HistoryEvent, WorkflowExecutionStartedEventAttributes},
        taskqueue::v1::TaskQueue,
    };

    fn synthetic_temporal_start(workflow_type: &str, queue: &str) -> History {
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
                            data: b"verified-server-history-input".to_vec().into(),
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
    fn history_start_payload_requires_exact_registered_type_and_queue() {
        let valid = synthetic_temporal_start(CHILD_TYPE, CHILD_QUEUE);
        assert_eq!(
            first_started_payload(&valid, CHILD_TYPE, CHILD_QUEUE).unwrap(),
            b"verified-server-history-input"
        );
        assert!(
            first_started_payload(&valid, "PgTruthReviewReconcileV3Workflow", CHILD_QUEUE).is_err()
        );
        assert!(first_started_payload(&valid, CHILD_TYPE, "alegria-pipeline").is_err());
    }

    #[test]
    fn forged_parent_claim_cannot_substitute_for_server_parent_child_events() {
        let parent = synthetic_temporal_start(PARENT_TYPE, PARENT_QUEUE);
        let child = synthetic_temporal_start(CHILD_TYPE, CHILD_QUEUE);
        let request = V4IssueRequest {
            admission_id: Uuid::new_v4(),
            canonical_run_id: Uuid::new_v4(),
            parent_workflow_id: "forged-parent".into(),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            child_workflow_id: "forged-child".into(),
            child_workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 30,
        };
        assert!(prove_reciprocal_temporal_histories(&parent, &child, &request).is_err());
    }
}
