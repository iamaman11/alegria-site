//! Separate DataForSEO STUB issuer. This is an operator, NEVER an owner Activity.
//!
//! A caller's request contains only Temporal execution locators. Authority is
//! independently derived from exact server Histories, tenant-governed SQL
//! business-run rows and a distinct, restricted PostgreSQL LOGIN. The role
//! may insert immutable *stub-only* admission, then signal the existing owner.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use dataforseo_contracts::{
    validate_dataforseo_owner_request, verify_dataforseo_owner_invocation,
    DataForSeoGoogleOrganicSearchInputV1, DataForSeoGoogleOrganicSearchV1,
    DATAFORSEO_OPERATOR_SIGNAL_V1, DATAFORSEO_OWNER_QUEUE_V1, INPUT_PORT,
};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2, owner_content_hash,
    owner_validation_child_workflow_id_v1, ArtifactBinding, Operation, OperationDagRunV6,
    OwnerInvocationV1, OwnerValidationRequestV1, OwnerValidationResultV1, PortId,
    TemporalExecutionIdentityV4, OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    OWNER_OPERATION_CHILD_TYPE_V1, OWNER_VALIDATION_CHILD_TYPE_V1,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, UntypedSignal, UntypedWorkflow,
    WorkflowDescribeOptions, WorkflowExecutionInfo, WorkflowFetchHistoryOptions,
    WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssueRequest {
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
    input: DataForSeoGoogleOrganicSearchInputV1,
    parent_start_bytes: Vec<u8>,
    child_start_bytes: Vec<u8>,
    child_started_event_id: i64,
}

fn validate_locator(request: &IssueRequest, build_id: &str) -> Result<()> {
    ensure!(
        !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && (1..=300).contains(&request.expires_in_seconds)
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok()
            && request.parent_workflow_id
                == operation_dag_parent_workflow_id_v2(&request.canonical_run_id.to_string())
            && request
                .child_workflow_id
                .starts_with("alegria-owner-operation-v2-")
            && build_id.len() == 40
            && build_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid DataForSEO exact-run locator, expiry or deployed build ID"
    );
    Ok(())
}

fn first_started_payload(history: &History, workflow_type: &str, queue: &str) -> Result<Vec<u8>> {
    let start = history
        .events
        .first()
        .context("empty DataForSEO Temporal server History")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &start.attributes else {
        bail!("DataForSEO History does not begin with server WorkflowExecutionStarted")
    };
    ensure!(
        attrs
            .workflow_type
            .as_ref()
            .map(|value| value.name.as_str())
            == Some(workflow_type)
            && attrs.task_queue.as_ref().map(|value| value.name.as_str()) == Some(queue),
        "DataForSEO Temporal Workflow Type or Task Queue mismatch"
    );
    let payloads = attrs
        .input
        .as_ref()
        .context("Temporal Workflow has no start input")?;
    ensure!(
        payloads.payloads.len() == 1
            && !payloads.payloads[0].data.is_empty()
            && payloads.payloads[0].data.len() <= 2 * 1024 * 1024,
        "DataForSEO Workflow start must have one bounded typed payload"
    );
    Ok(payloads.payloads[0].data.clone())
}

fn started_child<'a>(
    parent: &'a History,
    workflow_id: &str,
    child_type: &str,
    task_queue: &str,
    input: &[u8],
) -> Result<(
    &'a temporalio_common::protos::temporal::api::history::v1::HistoryEvent,
    &'a temporalio_common::protos::temporal::api::history::v1::HistoryEvent,
)> {
    let matches = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs))
                if attrs.workflow_id == workflow_id =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        matches.len() == 1,
        "exactly one ChildInitiated event required"
    );
    let (initiated, attrs) = matches[0];
    ensure!(
        attrs
            .workflow_type
            .as_ref()
            .map(|value| value.name.as_str())
            == Some(child_type)
            && attrs.task_queue.as_ref().map(|value| value.name.as_str()) == Some(task_queue)
            && attrs.input.as_ref().is_some_and(
                |value| value.payloads.len() == 1 && value.payloads[0].data.as_slice() == input
            ),
        "ChildInitiated queue/type/exact bytes differ from independently observed child"
    );
    let starts = parent
        .events
        .iter()
        .filter(|event| {
            matches!(&event.attributes,
            Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(value))
            if value.initiated_event_id == initiated.event_id)
        })
        .collect::<Vec<_>>();
    ensure!(starts.len() == 1, "exactly one ChildStarted event required");
    Ok((initiated, starts[0]))
}

fn prove_effect_pair(
    request: &IssueRequest,
    trusted_build_id: &str,
    parent: &History,
    child: &History,
) -> Result<VerifiedHistory> {
    validate_locator(request, trusted_build_id)?;
    let parent_bytes = first_started_payload(
        parent,
        OPERATION_DAG_PARENT_TYPE_V2,
        OPERATION_DAG_PARENT_QUEUE_V2,
    )?;
    let child_bytes = first_started_payload(
        child,
        OWNER_OPERATION_CHILD_TYPE_V1,
        DATAFORSEO_OWNER_QUEUE_V1,
    )?;
    let run: OperationDagRunV6 = serde_json::from_slice(&parent_bytes)?;
    ensure!(
        run.schema_revision == 6
            && serde_json::to_vec(&run)? == parent_bytes
            && run.plan.hash == run.plan.canonical_hash()?
            && run.plan.scope.run_id == request.canonical_run_id.to_string(),
        "parent server History does not contain exact canonical V2 DAG"
    );
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_bytes)?;
    let input = verify_dataforseo_owner_invocation(&invocation)?;
    let dispatch = &invocation.checked_dispatch;
    let contract = DataForSeoGoogleOrganicSearchV1::contract();
    let contract_hash = contract.fingerprint()?;
    ensure!(
        serde_json::to_vec(&invocation)? == child_bytes
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id
            && request.child_workflow_id == owner_child_workflow_id_v2(&dispatch.execution_key)
            && dispatch.operation == contract.reference
            && dispatch.owner == contract.owner
            && dispatch.effects == contract.effects
            && dispatch.idempotency == contract.idempotency
            && dispatch.plan_hash == run.plan.hash
            && dispatch.scope == run.plan.scope
            && input.scope == run.plan.scope,
        "DataForSEO exact child command differs from parent scope/contract/identity"
    );
    let (initiated, started) = started_child(
        parent,
        &request.child_workflow_id,
        OWNER_OPERATION_CHILD_TYPE_V1,
        DATAFORSEO_OWNER_QUEUE_V1,
        &child_bytes,
    )?;
    let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(started_attrs)) =
        &started.attributes
    else {
        bail!("missing actual effect ChildStarted")
    };
    let server_child = started_attrs
        .workflow_execution
        .as_ref()
        .context("effect ChildStarted missing server execution")?;
    let own_start = child.events.first().context("empty effect child History")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(own_attrs)) =
        &own_start.attributes
    else {
        bail!("effect child History missing server start")
    };
    let server_parent = own_attrs
        .parent_workflow_execution
        .as_ref()
        .context("effect child has no real parent")?;
    ensure!(
        server_child.workflow_id == request.child_workflow_id
            && server_child.run_id == request.child_workflow_run_id
            && started_attrs
                .workflow_type
                .as_ref()
                .map(|value| value.name.as_str())
                == Some(OWNER_OPERATION_CHILD_TYPE_V1)
            && own_attrs.parent_initiated_event_id == initiated.event_id
            && server_parent.workflow_id == request.parent_workflow_id
            && server_parent.run_id == request.parent_workflow_run_id,
        "parent/effect-child reciprocal server History identities mismatch"
    );
    let owner_wire = run
        .owner_commands
        .get(&dispatch.instance)
        .context("effect command absent from immutable parent start")?;
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == dispatch.instance)
        .context("effect node absent from immutable parent DAG")?;
    let bound = invocation
        .bound_inputs
        .get(&PortId::from(INPUT_PORT))
        .context("DataForSEO typed external query missing")?;
    let ArtifactBinding::External { artifact } = &bound.source else {
        bail!("DataForSEO owner may consume only independent External query")
    };
    let route = run
        .pinned_routes
        .iter()
        .find(|route| route.operation == dispatch.operation && route.owner == dispatch.owner)
        .context("DataForSEO operation missing pinned owner route")?;
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .any(|pinned| pinned.reference == contract.reference
                && pinned.fingerprint == contract_hash)
            && owner_wire.operation == dispatch.operation
            && owner_wire.owner == dispatch.owner
            && owner_wire.contract_hash == contract_hash
            && owner_wire.command_bytes == invocation.command_bytes
            && node.operation == dispatch.operation
            && node.permitted_effects == dispatch.effects
            && run.plan.allowed_effects.is_superset(&dispatch.effects)
            && node.inputs.len() == invocation.bound_inputs.len()
            && node.inputs.iter().all(|(port, binding)| invocation
                .bound_inputs
                .get(port)
                .is_some_and(|bound| &bound.source == binding))
            && artifact == &bound.artifact_id
            && run.admitted.get(artifact) == Some(&bound.artifact)
            && route.contract_hash == contract_hash
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && route.task_queue == DATAFORSEO_OWNER_QUEUE_V1
            && route.build_id == trusted_build_id
            && run
                .pinned_routes
                .iter()
                .filter(|route| route.operation == dispatch.operation)
                .count()
                == 1,
        "DataForSEO route, contract, source bytes or owner command differs from parent"
    );
    Ok(VerifiedHistory {
        invocation,
        input,
        parent_start_bytes: parent_bytes,
        child_start_bytes: child_bytes,
        child_started_event_id: started.event_id,
    })
}

/// Prove the WHOLE ValidateOnly wave, not only DataForSEO's own preflight.
/// Every validation has to complete before the effect Child even starts.
/// Child own server History is fetched independently, and both Temporal
/// views must contain byte-identical typed requests and accepted results.
async fn prove_all_validations(
    client: &Client,
    namespace: &str,
    request: &IssueRequest,
    parent: &History,
    proof: &VerifiedHistory,
) -> Result<()> {
    let dag: OperationDagRunV6 = serde_json::from_slice(&proof.parent_start_bytes)?;
    let parent_identity = TemporalExecutionIdentityV4 {
        workflow_id: request.parent_workflow_id.clone(),
        run_id: request.parent_workflow_run_id.clone(),
    };
    for node in &dag.plan.nodes {
        let owner_wire = dag
            .owner_commands
            .get(&node.instance)
            .context("ValidateOnly lacks immutable owner command wire")?;
        let route = dag
            .pinned_routes
            .iter()
            .find(|route| route.operation == node.operation && route.owner == owner_wire.owner)
            .context("ValidateOnly lacks parent-pinned owner route")?;
        let id =
            owner_validation_child_workflow_id_v1(&parent_identity, &dag.plan.hash, &node.instance);
        let initiated = parent
            .events
            .iter()
            .filter(|event| {
                matches!(&event.attributes,
                Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(value))
                    if value.workflow_id == id)
            })
            .collect::<Vec<_>>();
        ensure!(
            initiated.len() == 1,
            "V2 requires exactly one actual ValidateOnly Child per operation"
        );
        let initiated = initiated[0];
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(init)) =
            &initiated.attributes
        else {
            bail!("missing ValidateOnly initiated attributes")
        };
        ensure!(
            init.workflow_type.as_ref().map(|value| value.name.as_str())
                == Some(OWNER_VALIDATION_CHILD_TYPE_V1)
                && init.task_queue.as_ref().map(|value| value.name.as_str())
                    == Some(route.task_queue.as_str()),
            "ValidateOnly child dispatched to foreign type or owner queue"
        );
        let raw_request = init
            .input
            .as_ref()
            .and_then(|value| {
                (value.payloads.len() == 1).then_some(value.payloads[0].data.as_slice())
            })
            .context("ValidateOnly ChildInitiated lacks exact command")?;
        let typed: OwnerValidationRequestV1 = serde_json::from_slice(raw_request)?;
        typed.verify_against_run(&dag)?;
        ensure!(
            serde_json::to_vec(&typed)? == raw_request
                && typed.parent == parent_identity
                && typed.instance == node.instance,
            "ValidateOnly request differs from canonical parent command"
        );
        let started = parent
            .events
            .iter()
            .filter(|event| {
                matches!(&event.attributes,
                Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(value))
                    if value.initiated_event_id == initiated.event_id)
            })
            .collect::<Vec<_>>();
        ensure!(started.len() == 1, "ValidateOnly must start exactly once");
        let started = started[0];
        let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(started_attrs)) =
            &started.attributes
        else {
            bail!("missing ValidateOnly start attributes")
        };
        let execution = started_attrs
            .workflow_execution
            .as_ref()
            .context("ValidateOnly server start lacks real Run ID")?;
        ensure!(
            execution.workflow_id == id
                && started_attrs
                    .workflow_type
                    .as_ref()
                    .map(|value| value.name.as_str())
                    == Some(OWNER_VALIDATION_CHILD_TYPE_V1),
            "ValidateOnly ChildStarted server identity mismatch"
        );
        let completed = parent
            .events
            .iter()
            .filter(|event| {
                matches!(&event.attributes,
                Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(value))
                    if value.initiated_event_id == initiated.event_id
                       && value.started_event_id == started.event_id)
            })
            .collect::<Vec<_>>();
        ensure!(
            completed.len() == 1 && completed[0].event_id < proof.child_started_event_id,
            "all ValidateOnly Children must finish BEFORE effect child starts"
        );
        let Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(completion)) =
            &completed[0].attributes
        else {
            bail!("missing ValidateOnly completion attributes")
        };
        let parent_result = completion
            .result
            .as_ref()
            .and_then(|value| {
                (value.payloads.len() == 1).then_some(value.payloads[0].data.as_slice())
            })
            .context("ValidateOnly parent completion lacks typed result")?;
        let child_history = fetch_exact_history(client, namespace, &id, &execution.run_id).await?;
        let own_bytes = first_started_payload(
            &child_history,
            OWNER_VALIDATION_CHILD_TYPE_V1,
            &route.task_queue,
        )?;
        ensure!(
            own_bytes == raw_request,
            "ValidateOnly child own History start differs from parent initiation"
        );
        let own_first = child_history
            .events
            .first()
            .context("empty ValidateOnly History")?;
        let Some(Attributes::WorkflowExecutionStartedEventAttributes(own_start)) =
            &own_first.attributes
        else {
            bail!("ValidateOnly child lacks own start attributes")
        };
        let actual_parent = own_start
            .parent_workflow_execution
            .as_ref()
            .context("ValidateOnly child lacks real parent")?;
        ensure!(
            actual_parent.workflow_id == request.parent_workflow_id
                && actual_parent.run_id == request.parent_workflow_run_id
                && own_start.parent_initiated_event_id == initiated.event_id,
            "ValidateOnly child own History parent link differs from server parent"
        );
        let completed_own = child_history
            .events
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::WorkflowExecutionCompletedEventAttributes(done)) => Some(done),
                _ => None,
            })
            .collect::<Vec<_>>();
        ensure!(
            completed_own.len() == 1,
            "ValidateOnly own History lacks one completed result"
        );
        let own_result = completed_own[0]
            .result
            .as_ref()
            .and_then(|value| {
                (value.payloads.len() == 1).then_some(value.payloads[0].data.as_slice())
            })
            .context("ValidateOnly own completion lacks typed result")?;
        ensure!(
            own_result == parent_result,
            "ValidateOnly child result differs across independent Temporal Histories"
        );
        let accepted: OwnerValidationResultV1 = serde_json::from_slice(own_result)?;
        ensure!(
            serde_json::to_vec(&accepted)? == own_result,
            "ValidateOnly own completion contains noncanonical typed result bytes"
        );
        accepted.verify_success(&typed)?;
        if node.operation == DataForSeoGoogleOrganicSearchV1::contract().reference {
            let independently_checked = validate_dataforseo_owner_request(&typed)?;
            ensure!(
                independently_checked.request_hash == accepted.request_hash
                    && independently_checked.contract_hash == accepted.contract_hash
                    && independently_checked.operation == accepted.operation,
                "DataForSEO operator cannot independently reproduce ValidateOnly acceptance"
            );
        }
    }
    Ok(())
}

async fn fetch_exact_history(
    client: &Client,
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
    handle
        .describe(WorkflowDescribeOptions::builder().build())
        .await
        .context("DataForSEO operator exact-run Describe failed")?;
    let events = handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events()
        .await
        .context("DataForSEO operator exact-run History fetch failed")?;
    ensure!(
        events.len() <= 10_000,
        "DataForSEO operator server History exceeds bound"
    );
    Ok(History::from(events))
}

fn validate_local_temporal_endpoint(raw: &str) -> Result<()> {
    let endpoint = url::Url::parse(raw)?;
    ensure!(
        endpoint.scheme() == "http"
            && endpoint.host_str() == Some("127.0.0.1")
            && endpoint.port().is_some()
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.path() == "/"
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "DataForSEO issuer only accepts local test Temporal (remote TLS authority absent)"
    );
    Ok(())
}

async fn connect_proof_client(endpoint: &str, namespace: &str) -> Result<Client> {
    validate_local_temporal_endpoint(endpoint)?;
    let options = ConnectionOptions::new(url::Url::parse(endpoint)?)
        .identity(format!(
            "alegria-dataforseo-independent-issuer:{}",
            std::process::id()
        ))
        .build();
    let connection = Connection::connect(options).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn attest_from_server(request: &IssueRequest, build_id: &str) -> Result<VerifiedHistory> {
    let endpoint = env::var("DATAFORSEO_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("DATAFORSEO_OPERATOR_TEMPORAL_NAMESPACE")?;
    validate_local_temporal_endpoint(&endpoint)?;
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "DataForSEO issuer namespace must be explicit"
    );
    let client = connect_proof_client(&endpoint, &namespace).await?;
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
    let attested = prove_effect_pair(request, build_id, &parent, &child)?;
    prove_all_validations(&client, &namespace, request, &parent, &attested).await?;
    Ok(attested)
}

/// The operator is a separate PostgreSQL LOGIN, not a worker with an extra
/// flag. Its only application write is an INSERT into its own admission table.
async fn operator_pool() -> Result<(PgPool, String)> {
    let configured_tenant = env::var("DATAFORSEO_OPERATOR_TENANT_ID")?;
    ensure!(
        !configured_tenant.is_empty()
            && configured_tenant.trim() == configured_tenant
            && configured_tenant.len() <= 256
            && !configured_tenant.chars().any(char::is_control),
        "DataForSEO operator tenant is malformed"
    );
    let credential_file = env::var("DATAFORSEO_OPERATOR_DATABASE_URL_FILE")?;
    let path = Path::new(&credential_file);
    ensure!(
        path.is_absolute(),
        "operator credential file path must be absolute"
    );
    ensure!(
        fs::canonicalize(path)?.as_path() == path,
        "DataForSEO operator DSN path must be canonical and symlink-free"
    );
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_file() && meta.len() <= 8192,
        "operator DSN must come from a small regular secret file, not symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "DataForSEO operator DSN secret must be 0600/0400"
        );
    }
    if let Ok(owner_secret) = env::var("DATAFORSEO_OWNER_DATABASE_URL_FILE") {
        ensure!(
            owner_secret != credential_file,
            "operator cannot reuse DataForSEO owner credentials"
        );
    }
    let url = fs::read_to_string(path)?;
    let url = url.trim_end_matches(['\r', '\n']);
    ensure!(
        !url.is_empty() && url.trim() == url,
        "operator DSN must not contain surrounding whitespace"
    );
    let parsed = url::Url::parse(url)?;
    ensure!(
        matches!(parsed.scheme(), "postgres" | "postgresql")
            && !parsed.username().is_empty()
            && parsed.host_str().is_some()
            && !parsed.path().is_empty()
            && parsed.path() != "/"
            && parsed.fragment().is_none(),
        "DataForSEO operator DSN must use explicit PostgreSQL database/login"
    );
    let pool = PgPool::connect(url).await?;
    let row = sqlx::query(
        "SELECT session_user::text AS login,
                current_user::text AS role_name,
                r.rolcanlogin AS can_login,
                (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication) AS elevated,
                EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m
                        WHERE m.member = r.oid) AS has_memberships,
                pipeline.current_session_tenant_v1() AS tenant,
                has_schema_privilege(current_user,'pipeline','USAGE') AS schema_usage,
                has_table_privilege(current_user,
                  'pipeline.dataforseo_owner_admissions_v1','INSERT') AS can_issue,
                has_table_privilege(current_user,
                  'pipeline.dataforseo_owner_admissions_v1','SELECT') AS can_read_admissions,
                has_table_privilege(current_user,
                  'pipeline.operation_dag_business_runs_v1','SELECT') AS can_read_business,
                has_function_privilege(current_user,
                  'pipeline.operation_dag_governance_matches_v2(uuid,text,text,text,text,text,text,boolean)',
                  'EXECUTE') AS can_check_governance,
                has_function_privilege(current_user,
                  'pipeline.begin_dataforseo_owner_v1(uuid,bytea,text,bigint)',
                  'EXECUTE') AS can_run_effect,
                has_function_privilege(current_user,
                  'pipeline.confirm_dataforseo_owner_v1(uuid,bytea,text,text,bytea,bytea,bigint)',
                  'EXECUTE') AS can_confirm_effect,
                has_function_privilege(current_user,
                  'pipeline.lookup_dataforseo_owner_admission_v1(text,uuid,text,text,text,text,text,text,text,text,text,bigint)',
                  'EXECUTE') AS can_lookup_effect,
                has_function_privilege(current_user,
                  'pipeline.record_stub_dataforseo_task_v1(uuid,bytea,text,text)',
                  'EXECUTE') AS can_record_effect,
                has_function_privilege(current_user,
                  'pipeline.lookup_stub_dataforseo_task_v1(uuid,bytea,text)',
                  'EXECUTE') AS can_read_effect_task,
                has_function_privilege(current_user,
                  'pipeline.read_confirmed_dataforseo_owner_receipt_v1(uuid,bytea,text)',
                  'EXECUTE') AS can_read_effect_receipt,
                EXISTS (SELECT 1 FROM pg_catalog.pg_class c
                    WHERE c.oid='pipeline.dataforseo_owner_admissions_v1'::regclass
                      AND c.relowner = r.oid) AS owns_admissions
           FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        row.get::<String, _>("login") == row.get::<String, _>("role_name")
            && row.get::<bool, _>("can_login")
            && !row.get::<bool, _>("elevated")
            && !row.get::<bool, _>("has_memberships")
            && row.get::<Option<String>, _>("tenant").as_deref()
                == Some(configured_tenant.as_str())
            && row.get::<bool, _>("schema_usage")
            && row.get::<bool, _>("can_issue")
            && !row.get::<bool, _>("can_read_admissions")
            && row.get::<bool, _>("can_read_business")
            && row.get::<bool, _>("can_check_governance")
            && !row.get::<bool, _>("can_run_effect")
            && !row.get::<bool, _>("can_confirm_effect")
            && !row.get::<bool, _>("can_lookup_effect")
            && !row.get::<bool, _>("can_record_effect")
            && !row.get::<bool, _>("can_read_effect_task")
            && !row.get::<bool, _>("can_read_effect_receipt")
            && !row.get::<bool, _>("owns_admissions"),
        "DataForSEO operator requires isolated tenant-bound issuer-only PostgreSQL role"
    );
    let forbidden_table: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
           FROM pg_catalog.pg_class c
           JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname<>'information_schema' AND n.nspname !~ '^pg_'
            AND c.relkind IN ('r','p','v','m','f')
            AND (
                (has_any_column_privilege(current_user,c.oid,'INSERT')
                  AND NOT (n.nspname='pipeline'
                    AND c.relname='dataforseo_owner_admissions_v1'))
                OR has_any_column_privilege(current_user,c.oid,'UPDATE')
                OR has_table_privilege(current_user,c.oid,'DELETE')
                OR has_table_privilege(current_user,c.oid,'TRUNCATE')
            )
          ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        forbidden_table.is_none(),
        "DataForSEO issuer role has forbidden foreign write capability"
    );
    let forbidden_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname FROM pg_catalog.pg_namespace n
          WHERE n.nspname<>'information_schema' AND n.nspname !~ '^pg_'
            AND has_schema_privilege(current_user,n.oid,'CREATE') LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        forbidden_schema.is_none(),
        "DataForSEO issuer must not have schema creation privilege"
    );
    let forbidden_sequence: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
           FROM pg_catalog.pg_class c
           JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname<>'information_schema' AND n.nspname !~ '^pg_'
            AND c.relkind='S'
            AND (has_sequence_privilege(current_user,c.oid,'USAGE')
              OR has_sequence_privilege(current_user,c.oid,'UPDATE')) LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        forbidden_sequence.is_none(),
        "DataForSEO issuer must not mutate application sequences"
    );
    Ok((pool, configured_tenant))
}

async fn write_admission(
    pool: &PgPool,
    tenant: &str,
    request: &IssueRequest,
    proof: &VerifiedHistory,
) -> Result<()> {
    ensure!(
        proof.input.scope.tenant_id == tenant
            && proof.input.scope.run_id == request.canonical_run_id.to_string(),
        "DataForSEO immutable History scope differs from authenticated tenant"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut *tx)
        .await?;
    // The same global lock order is used by the independent governance
    // issuers and by the SQL admission/attempt guards.
    for key in [
        format!("operation-dag-business|{}", request.canonical_run_id),
        format!("operation-dag-canonical|{}", request.canonical_run_id),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
            .bind(&key)
            .execute(&mut *tx)
            .await?;
    }
    let business = sqlx::query(
        "SELECT context_key, tenant_id, plan_hash, parent_workflow_id,
                revoked_at IS NULL AND expires_at>clock_timestamp() AS active
           FROM pipeline.operation_dag_business_runs_v1 WHERE run_id=$1",
    )
    .bind(request.canonical_run_id)
    .fetch_one(&mut *tx)
    .await?;
    let context: String = business.try_get("context_key")?;
    ensure!(
        business.try_get::<String, _>("tenant_id")? == tenant
            && business.try_get::<String, _>("plan_hash")?
                == proof.invocation.checked_dispatch.plan_hash.as_str()
            && business.try_get::<String, _>("parent_workflow_id")? == request.parent_workflow_id
            && business.try_get::<bool, _>("active")?,
        "DataForSEO issuer requires active independent business-run authority"
    );
    let parent_sha: String =
        sqlx::query_scalar("SELECT encode(public.digest($1::bytea,'sha256'),'hex')")
            .bind(&proof.parent_start_bytes)
            .fetch_one(&mut *tx)
            .await?;
    let child_sha: String =
        sqlx::query_scalar("SELECT encode(public.digest($1::bytea,'sha256'),'hex')")
            .bind(&proof.child_start_bytes)
            .fetch_one(&mut *tx)
            .await?;
    let governed: bool = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_governance_matches_v2(
          $1,$2,$3,$4,$5,$6,$7,TRUE)",
    )
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&context)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
    .bind(&parent_sha)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        governed,
        "DataForSEO source admission requires independently governed parent History"
    );
    let dispatch = &proof.invocation.checked_dispatch;
    let cost_cap = i64::try_from(proof.input.max_cost_usd_micros)?;
    let affected = sqlx::query(
        "INSERT INTO pipeline.dataforseo_owner_admissions_v1
          (admission_id,run_id,tenant_id,context_key,operation_instance,
           plan_hash,dispatch_execution_key,parent_workflow_id,
           parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
           child_started_event_id,parent_sha256,child_sha256,
           command_sha256,command_hash,input_hash,command_bytes,
           max_cost_usd_micros,expires_at)
          VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
                  encode(public.digest($15::bytea,'sha256'),'hex'),
                  $16,$17,$15,$18,clock_timestamp()+($19::int * interval '1 second'))",
    )
    .bind(request.admission_id)
    .bind(request.canonical_run_id)
    .bind(tenant)
    .bind(&context)
    .bind(dispatch.instance.as_str())
    .bind(dispatch.plan_hash.as_str())
    .bind(&dispatch.execution_key)
    .bind(&request.parent_workflow_id)
    .bind(&request.parent_workflow_run_id)
    .bind(&request.child_workflow_id)
    .bind(&request.child_workflow_run_id)
    .bind(proof.child_started_event_id)
    .bind(&parent_sha)
    .bind(&child_sha)
    .bind(&proof.invocation.command_bytes)
    .bind(owner_content_hash(&proof.invocation.command_bytes))
    .bind(&dispatch.input_hash)
    .bind(cost_cap)
    .bind(i32::try_from(request.expires_in_seconds)?)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(
        affected == 1,
        "DataForSEO issuer did not insert exactly one admission"
    );
    tx.commit().await?;
    Ok(())
}

async fn signal_after_commit(request: &IssueRequest) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let endpoint = env::var("DATAFORSEO_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("DATAFORSEO_OPERATOR_TEMPORAL_NAMESPACE")?;
    validate_local_temporal_endpoint(&endpoint)?;
    let client = connect_proof_client(&endpoint, &namespace).await?;
    WorkflowExecutionInfo::builder()
        .namespace(namespace)
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build()
        .bind_untyped(client)
        .signal(
            UntypedSignal::<UntypedWorkflow>::new(DATAFORSEO_OPERATOR_SIGNAL_V1),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("DataForSEO admission committed, but wakeup signal failed; reconcile manually")
}

pub(super) async fn issue_from_file(path: &Path) -> Result<()> {
    ensure!(
        env::var("ALEGRIA_ENABLE_DATAFORSEO_STUB_OPERATOR_V1").as_deref() == Ok("true"),
        "independent DataForSEO STUB issuer must be explicitly enabled"
    );
    let build_id = env::var("DATAFORSEO_OPERATOR_TRUSTED_BUILD_ID")?;
    let bytes = fs::read(path)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= 8192,
        "DataForSEO operator locator must be bounded"
    );
    let request: IssueRequest = serde_json::from_slice(&bytes)?;
    validate_locator(&request, &build_id)?;
    let proof = attest_from_server(&request, &build_id).await?;
    let (pool, tenant) = operator_pool().await?;
    write_admission(&pool, &tenant, &request, &proof).await?;
    signal_after_commit(&request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_does_not_accept_external_or_remote_temporal_endpoint() {
        for forbidden in [
            "https://127.0.0.1:7233/",
            "http://localhost:7233/",
            "http://10.0.0.1:7233/",
            "http://127.0.0.1:7233/service",
            "http://127.0.0.1:7233/?namespace=default",
            "http://user:pass@127.0.0.1:7233/",
        ] {
            assert!(validate_local_temporal_endpoint(forbidden).is_err());
        }
        assert!(validate_local_temporal_endpoint("http://127.0.0.1:7233/").is_ok());
    }

    #[test]
    fn issuer_locator_cannot_be_dataforseo_auth_or_arbitrary_child() {
        let request = IssueRequest {
            admission_id: Uuid::nil(),
            canonical_run_id: Uuid::new_v4(),
            parent_workflow_id: "x".into(),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            child_workflow_id: "arbitrary".into(),
            child_workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 60,
        };
        assert!(validate_locator(&request, &"a".repeat(40)).is_err());
        assert!(validate_locator(&request, "short").is_err());
    }

    #[test]
    fn issuer_refuses_forged_locator_without_real_server_history() {
        let canonical_run_id = Uuid::new_v4();
        let request = IssueRequest {
            admission_id: Uuid::new_v4(),
            canonical_run_id,
            parent_workflow_id: operation_dag_parent_workflow_id_v2(&canonical_run_id.to_string()),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            child_workflow_id: owner_child_workflow_id_v2(&"a".repeat(64)),
            child_workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 60,
        };
        assert!(validate_locator(&request, &"b".repeat(40)).is_ok());
        let absent = History::from(Vec::new());
        assert!(prove_effect_pair(&request, &"b".repeat(40), &absent, &absent).is_err());
    }

    #[test]
    fn issuer_locator_does_not_accept_user_supplied_source_bytes() {
        let raw = serde_json::json!({
            "admission_id": Uuid::new_v4(),
            "canonical_run_id": Uuid::new_v4(),
            "parent_workflow_id": "claimed",
            "parent_workflow_run_id": Uuid::new_v4(),
            "child_workflow_id": "claimed",
            "child_workflow_run_id": Uuid::new_v4(),
            "expires_in_seconds": 60,
            "trusted_source_hash": "forged"
        });
        assert!(serde_json::from_value::<IssueRequest>(raw).is_err());
    }
}
