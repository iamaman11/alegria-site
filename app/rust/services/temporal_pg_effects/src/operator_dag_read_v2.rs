//! Read-only, tenant-scoped proof of a native V2 Temporal operation DAG.
//!
//! This operator does not issue root authority, start a Workflow, signal a
//! Child, invoke a provider or claim that an owner's durable external effect
//! was independently verified. Only the exact server History is observed.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::Path,
};

#[cfg(unix)]
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2, owner_content_hash,
    owner_validation_child_workflow_id_v1, ArtifactBinding, ExecutionPlan, OperationDagMachine,
    OperationDagReadStepV1, OperationDagReadTraceV1, OperationDagRunV6, OwnerResultV1,
    OwnerValidationResultV1, TemporalExecutionIdentityV4, DAG_MAX_INPUT_BYTES,
    DAG_MAX_RESULT_BYTES, DAG_READ_MAX_REPLY_BYTES, DAG_READ_TRACE_SCHEMA_REVISION_V1,
    OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    OPERATION_DAG_RUN_SCHEMA_REVISION_V6, OWNER_OPERATION_CHILD_TYPE_V1,
    OWNER_VALIDATION_CHILD_TYPE_V1,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::{PgPool, Row};
use temporalio_client::Client;
use temporalio_common::protos::temporal::api::common::v1::Payloads;
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use uuid::Uuid;

#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    time::timeout,
};

use super::operator_dag_governance::{
    read_operator_owned_catalog, read_private_operator_file, validate_local_temporal_endpoint,
};
use super::operator_v4::{fetch_exact_history, first_started_payload};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunLocator {
    canonical_run_id: Uuid,
}

#[derive(Debug, Serialize)]
struct SafeRunInspection {
    operation: &'static str,
    status: &'static str,
    canonical_run_id: Uuid,
    parent_workflow_run_id: String,
    plan_hash: String,
    parent_start_sha256: String,
    step_count: usize,
    owner_result_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_blake3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<OperationDagReadTraceV1>,
    governance_currently_active: bool,
    temporal_history_attestation: &'static str,
    owner_receipt_attestation: &'static str,
    source_authentication: &'static str,
    next_action: &'static str,
}

const OPERATION: &str = "read_operation_dag_run_v2";

// A separately privileged, opt-in *read-only* Unix endpoint is the sole
// transport available to an untrusted local MCP authoring process. The
// operator account retains the private tenant-bound SELECT-only SQL login;
// MCP never receives the database URL, Temporal credentials or raw receipts.
// A dedicated 02710 directory lets an explicitly configured peer group
// traverse a known path, without revealing directory entries. A 0660 socket
// requires group membership, and SO_PEERCRED restricts admission to one exact
// *different* OS user even if that group has several members.
#[cfg(unix)]
const SOCKET_PARENT_MODE: u32 = 0o2710;
#[cfg(unix)]
const SOCKET_MODE: u32 = 0o660;
#[cfg(unix)]
const MAX_SOCKET_REQUEST_BYTES: usize = 256;

#[derive(Serialize)]
struct SafeRunRefusal {
    operation: &'static str,
    status: &'static str,
    code: &'static str,
    next_action: &'static str,
}

fn refusal_diagnostic() -> SafeRunRefusal {
    SafeRunRefusal {
        operation: OPERATION,
        status: "refused",
        code: "run_unavailable_or_untrusted",
        next_action: "Check the tenant-bound read-only role, installed catalog and exact Temporal/SQL run records with the operator.",
    }
}

/// A dedicated, local-only status bridge. There is deliberately no TCP
/// listener, user-supplied tenant, DSN, Workflow Run ID, provider credential,
/// arbitrary file path or write/Start/Signal capability here. A same-UID MCP
/// and operator is refused because it would not isolate operator credentials.
#[cfg(unix)]
pub(super) async fn serve_socket() -> Result<()> {
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_READ_V2")
            .ok()
            .as_deref()
            == Some("true"),
        "native V2 read-only operator disabled"
    );
    let path = env::var("OPERATION_DAG_READ_V2_SOCKET_PATH")?;
    let socket_path = Path::new(&path);
    ensure!(
        socket_path.is_absolute(),
        "V2 status socket path must be absolute"
    );
    let parent = socket_path
        .parent()
        .context("V2 status socket has no parent")?;
    ensure!(
        socket_path.file_name().is_some()
            && !socket_path.as_os_str().is_empty()
            && path.as_bytes().len() <= 100
            && socket_path.components().all(|component| {
                !matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            }),
        "invalid or unbounded V2 status socket path"
    );
    let parent_meta = std::fs::symlink_metadata(parent)?;
    ensure!(
        parent_meta.file_type().is_dir()
            && parent_meta.permissions().mode() & 0o7777 == SOCKET_PARENT_MODE,
        "V2 status socket parent must be a private setgid 02710 directory"
    );
    // No unlink/replacement of an existing socket: a stale or ambiguous
    // endpoint is an operator intervention, never automatic authority.
    ensure!(
        std::fs::symlink_metadata(socket_path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "V2 status socket path already exists or cannot be inspected"
    );
    let client_uid_text = env::var("OPERATION_DAG_READ_V2_ALLOWED_CLIENT_UID")?;
    let allowed_client_uid: u32 = client_uid_text.parse()?;
    ensure!(
        allowed_client_uid > 0
            && allowed_client_uid != parent_meta.uid()
            && client_uid_text == allowed_client_uid.to_string(),
        "V2 status client UID must be a canonical nonroot distinct OS user"
    );
    // Establish that the configured operator can read only its own tenant and
    // that the installed catalog and Temporal destination are trusted BEFORE
    // exposing the socket. Every request rechecks the SQL boundary as well.
    let tenant = env::var("OPERATION_DAG_READ_V2_TENANT_ID")?;
    ensure!(
        !tenant.is_empty() && tenant.trim() == tenant,
        "invalid reader tenant"
    );
    let preflight_pool = read_only_pool(&tenant).await?;
    preflight_pool.close().await;
    let catalog_file = env::var("OPERATION_DAG_READ_V2_TRUSTED_CATALOG_FILE")?;
    let _catalog = read_operator_owned_catalog(Path::new(&catalog_file))?;
    let temporal_url = env::var("OPERATION_DAG_READ_V2_TEMPORAL_URL")?;
    validate_local_temporal_endpoint(&temporal_url)?;
    let namespace = env::var("OPERATION_DAG_READ_V2_TEMPORAL_NAMESPACE")?;
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "invalid reader Temporal namespace"
    );
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    let socket_meta = std::fs::symlink_metadata(socket_path)?;
    ensure!(
        socket_meta.file_type().is_socket()
            && socket_meta.permissions().mode() & 0o777 == SOCKET_MODE
            && socket_meta.uid() == parent_meta.uid()
            && socket_meta.gid() == parent_meta.gid()
            && socket_meta.uid() != allowed_client_uid,
        "V2 socket owner/group must match private parent, with distinct authorized MCP UID"
    );
    let permits = std::sync::Arc::new(Semaphore::new(8));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let peer = stream.peer_cred();
        if !peer.is_ok_and(|identity| {
            authorized_client_uid(allowed_client_uid, identity.uid(), socket_meta.uid())
        }) {
            // Do not reveal existence of a run or enumerate allowed UIDs.
            continue;
        }
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            write_socket_diagnostic(&mut stream, &refusal_diagnostic()).await;
            continue;
        };
        tokio::spawn(async move {
            let _permit = permit;
            match timeout(Duration::from_secs(3), read_socket_locator(&mut stream)).await {
                Ok(Ok(locator)) => {
                    match timeout(Duration::from_secs(30), read_verified_run_locator(locator)).await
                    {
                        Ok(Ok(inspection)) => {
                            write_socket_diagnostic(&mut stream, &inspection).await;
                        }
                        _ => write_socket_diagnostic(&mut stream, &refusal_diagnostic()).await,
                    }
                }
                _ => write_socket_diagnostic(&mut stream, &refusal_diagnostic()).await,
            }
        });
    }
}

#[cfg(unix)]
fn authorized_client_uid(expected_uid: u32, peer_uid: u32, operator_uid: u32) -> bool {
    expected_uid > 0 && expected_uid != operator_uid && peer_uid == expected_uid
}

#[cfg(not(unix))]
pub(super) async fn serve_socket() -> Result<()> {
    bail!("read-only V2 operator socket requires Unix SO_PEERCRED and filesystem ACLs")
}

#[cfg(unix)]
async fn read_socket_locator(stream: &mut UnixStream) -> Result<RunLocator> {
    let mut request = Vec::with_capacity(80);
    let mut chunk = [0_u8; 64];
    loop {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        ensure!(
            request.len().saturating_add(count) <= MAX_SOCKET_REQUEST_BYTES,
            "unbounded V2 status request"
        );
        request.extend_from_slice(&chunk[..count]);
    }
    decode_socket_locator_line(&request)
}

#[cfg(unix)]
fn decode_socket_locator_line(request: &[u8]) -> Result<RunLocator> {
    ensure!(
        request.len() >= 2
            && request.len() <= MAX_SOCKET_REQUEST_BYTES
            && request.last() == Some(&b'\n'),
        "one complete bounded canonical V2 status line required"
    );
    decode_canonical_run_locator(&request[..request.len() - 1])
}

#[cfg(unix)]
async fn write_socket_diagnostic<T: Serialize>(stream: &mut UnixStream, response: &T) {
    let Ok(mut payload) = serde_json::to_vec(&response) else {
        return;
    };
    if payload.len() >= DAG_READ_MAX_REPLY_BYTES {
        return;
    }
    payload.push(b'\n');
    let _ = timeout(Duration::from_secs(3), stream.write_all(&payload)).await;
    let _ = stream.shutdown().await;
}

fn decode_canonical_run_locator(bytes: &[u8]) -> Result<RunLocator> {
    ensure!(
        bytes.len() <= 256,
        "V2 run locator exceeds bounded UUID request"
    );
    let locator: RunLocator = serde_json::from_slice(bytes)?;
    ensure!(
        !locator.canonical_run_id.is_nil() && serde_json::to_vec(&locator)?.as_slice() == bytes,
        "business run locator must be a canonical nonnil UUID-only JSON request"
    );
    Ok(locator)
}

/// Even a refused lookup must not echo raw Workflow/SQL/provider errors or
/// distinguish a nonexistent run from somebody else's tenant-scoped run.
pub(super) async fn read_from_file(path: &Path) -> Result<()> {
    match read_verified_run(path).await {
        Ok(report) => {
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        Err(_) => {
            println!("{}", serde_json::to_string(&refusal_diagnostic())?);
            bail!("read-only V2 run proof refused; see redacted JSON diagnostic")
        }
    }
}

async fn read_verified_run(path: &Path) -> Result<SafeRunInspection> {
    // File transport has exactly the same opt-in boundary as the socket.
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_READ_V2")
            .ok()
            .as_deref()
            == Some("true"),
        "native V2 read-only operator disabled"
    );
    let bytes = read_private_operator_file(path, "V2 read-only run locator")?;
    read_verified_run_locator(decode_canonical_run_locator(&bytes)?).await
}

async fn read_verified_run_locator(locator: RunLocator) -> Result<SafeRunInspection> {
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_READ_V2")
            .ok()
            .as_deref()
            == Some("true"),
        "native V2 read-only operator disabled"
    );
    let tenant = env::var("OPERATION_DAG_READ_V2_TENANT_ID")?;
    ensure!(
        !tenant.is_empty() && tenant.trim() == tenant,
        "reader tenant must be configured"
    );
    let pool = read_only_pool(&tenant).await?;
    // The PostgreSQL LOGIN, not a caller-provided tenant field or mutable
    // session GUC, establishes identity. Missing/cross-tenant RLS rows are
    // deliberately indistinguishable in the public diagnostic.
    let root = sqlx::query(
        "SELECT tenant_id,context_key,plan_hash,parent_workflow_id,
                expires_at,revoked_at
         FROM pipeline.operation_dag_business_runs_v1 WHERE run_id=$1",
    )
    .bind(locator.canonical_run_id)
    .fetch_optional(&pool)
    .await?
    .context("tenant-owned business run not found")?;
    let workflow_id = operation_dag_parent_workflow_id_v2(&locator.canonical_run_id.to_string());
    let context = root.get::<String, _>("context_key");
    let plan_hash = root.get::<String, _>("plan_hash");
    ensure!(
        root.get::<String, _>("tenant_id") == tenant
            && root.get::<String, _>("parent_workflow_id") == workflow_id,
        "business root identity mismatch"
    );
    let governance = sqlx::query(
        "SELECT tenant_id,context_key,plan_hash,parent_workflow_id,
                parent_workflow_run_id,parent_workflow_type,parent_task_queue,
                parent_start_sha256,expires_at,revoked_at
         FROM pipeline.operation_dag_run_governance_v2
         WHERE canonical_run_id=$1 ORDER BY issued_at DESC LIMIT 2",
    )
    .bind(locator.canonical_run_id)
    .fetch_all(&pool)
    .await?;
    ensure!(
        governance.len() == 1,
        "none or ambiguous independent Temporal governance identity"
    );
    let admitted = &governance[0];
    ensure!(
        admitted.get::<String, _>("tenant_id") == tenant
            && admitted.get::<String, _>("context_key") == context
            && admitted.get::<String, _>("plan_hash") == plan_hash
            && admitted.get::<String, _>("parent_workflow_id") == workflow_id
            && admitted.get::<String, _>("parent_workflow_type") == OPERATION_DAG_PARENT_TYPE_V2
            && admitted.get::<String, _>("parent_task_queue") == OPERATION_DAG_PARENT_QUEUE_V2,
        "trusted governance differs from the canonical business root"
    );
    let temporal_run_id = admitted.get::<String, _>("parent_workflow_run_id");
    ensure!(
        Uuid::parse_str(&temporal_run_id)
            .is_ok_and(|id| id.to_string() == temporal_run_id && !id.is_nil()),
        "noncanonical Temporal server Run ID"
    );
    let governance_active = root
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at")
        .is_none()
        && admitted
            .get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at")
            .is_none()
        && root.get::<chrono::DateTime<chrono::Utc>, _>("expires_at") > chrono::Utc::now()
        && admitted.get::<chrono::DateTime<chrono::Utc>, _>("expires_at") > chrono::Utc::now();

    let temporal_url = env::var("OPERATION_DAG_READ_V2_TEMPORAL_URL")?;
    validate_local_temporal_endpoint(&temporal_url)?;
    let namespace = env::var("OPERATION_DAG_READ_V2_TEMPORAL_NAMESPACE")?;
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "reader Temporal namespace invalid"
    );
    let catalog_path = env::var("OPERATION_DAG_READ_V2_TRUSTED_CATALOG_FILE")?;
    let catalog = read_operator_owned_catalog(Path::new(&catalog_path))?;
    let client = crate::connect_client(
        &temporal_url,
        format!("alegria-v2-read-only-{}", std::process::id()),
        &namespace,
    )
    .await?;
    let history = fetch_exact_history(&client, &namespace, &workflow_id, &temporal_run_id).await?;
    let start = first_started_payload(
        &history,
        OPERATION_DAG_PARENT_TYPE_V2,
        OPERATION_DAG_PARENT_QUEUE_V2,
    )?;
    ensure!(
        !start.is_empty() && start.len() <= DAG_MAX_INPUT_BYTES,
        "unbounded native V2 parent Start input"
    );
    let start_sha256: String =
        sqlx::query_scalar("SELECT encode(public.digest($1::bytea,'sha256'),'hex')")
            .bind(&start)
            .fetch_one(&pool)
            .await?;
    ensure!(
        start_sha256 == admitted.get::<String, _>("parent_start_sha256"),
        "server Start bytes differ from independently issued SQL governance"
    );
    let run: OperationDagRunV6 = serde_json::from_slice(&start)?;
    ensure!(
        run.schema_revision == OPERATION_DAG_RUN_SCHEMA_REVISION_V6
            && serde_json::to_vec(&run)? == start
            && run.plan.scope.run_id == locator.canonical_run_id.to_string()
            && run.plan.scope.tenant_id == tenant
            && run.plan.hash.as_str() == plan_hash,
        "server Start is not the exact canonical V2 run"
    );
    let step_count = run.plan.nodes.len();
    ensure!(
        step_count != 1 || history.events.len() <= 1_000,
        "one-step server History is too large"
    );
    let plan = run.plan.clone();
    let machine = OperationDagMachine::new_v2_with_owner_validation(&workflow_id, run, catalog)?;
    let (status, receipt_bytes, temporal_proof, next_action) = classify_server_result(&history)?;
    let trace = if let Some(bytes) = receipt_bytes {
        let reader = DagHistoryReader {
            client: &client,
            namespace: &namespace,
            history: &history,
            parent: TemporalExecutionIdentityV4 {
                workflow_id,
                run_id: temporal_run_id.clone(),
            },
        };
        let trace = reader.attest_completed_dag(machine, &plan, bytes).await?;
        if step_count > 1 {
            ensure!(trace.has_valid_shape(step_count), "invalid V2 trace shape");
            Some(trace)
        } else {
            None
        }
    } else {
        None
    };
    let owner_result_count = if receipt_bytes.is_some() {
        step_count
    } else {
        0
    };
    Ok(SafeRunInspection {
        operation: OPERATION,
        status,
        canonical_run_id: locator.canonical_run_id,
        parent_workflow_run_id: temporal_run_id,
        plan_hash,
        parent_start_sha256: start_sha256,
        step_count,
        owner_result_count,
        result_blake3: receipt_bytes.map(owner_content_hash),
        trace,
        governance_currently_active: governance_active,
        temporal_history_attestation: temporal_proof,
        owner_receipt_attestation: "not_independently_verified",
        source_authentication: "not_independently_attested",
        next_action,
    })
}

fn classify_server_result(
    history: &History,
) -> Result<(&'static str, Option<&[u8]>, &'static str, &'static str)> {
    let mut terminal = history.events.iter().filter_map(|event| {
        let status = match event.attributes.as_ref()? {
            Attributes::WorkflowExecutionCompletedEventAttributes(_) => {
                "temporal_completed_owner_unverified"
            }
            Attributes::WorkflowExecutionFailedEventAttributes(_) => "temporal_failed",
            Attributes::WorkflowExecutionCanceledEventAttributes(_) => "temporal_canceled",
            Attributes::WorkflowExecutionTimedOutEventAttributes(_) => "temporal_timed_out",
            Attributes::WorkflowExecutionTerminatedEventAttributes(_) => "temporal_terminated",
            Attributes::WorkflowExecutionContinuedAsNewEventAttributes(_) => {
                "temporal_continued_as_new"
            }
            _ => return None,
        };
        Some((event, status))
    });
    let Some((event, status)) = terminal.next() else {
        return Ok((
            "temporal_running",
            None,
            "exact_server_parent_start_only",
            "Wait for the native Temporal Workflow to finish, then read this same business run again.",
        ));
    };
    ensure!(
        terminal.next().is_none()
            && history
                .events
                .last()
                .is_some_and(|last| last.event_id == event.event_id),
        "contradictory native V2 terminal events"
    );
    if let Some(Attributes::WorkflowExecutionCompletedEventAttributes(done)) = &event.attributes {
        return Ok((
            status,
            Some(only_payload(done.result.as_ref(), DAG_MAX_RESULT_BYTES)?),
            "exact_server_parent_validation_and_effect_child_histories",
            "Ask the installed owner for an independently attested durable provider receipt before treating outputs as research evidence.",
        ));
    }
    Ok((
        status,
        None,
        "exact_server_parent_start_and_terminal_event",
        "Inspect the server-side terminal step without exposing raw Activity or provider errors to this reader.",
    ))
}

fn only_payload(payloads: Option<&Payloads>, limit: usize) -> Result<&[u8]> {
    let payloads = &payloads
        .context("Temporal history payload absent")?
        .payloads;
    ensure!(
        payloads.len() == 1 && !payloads[0].data.is_empty() && payloads[0].data.len() <= limit,
        "Temporal History must carry one bounded typed payload"
    );
    Ok(&payloads[0].data)
}

fn only_parent_event<'a, T>(
    history: &'a History,
    select: impl Fn(&'a Attributes) -> Option<&'a T>,
) -> Result<(i64, &'a T)> {
    let mut matches = history
        .events
        .iter()
        .filter_map(|event| Some((event.event_id, select(event.attributes.as_ref()?)?)));
    let matched = matches
        .next()
        .context("expected parent Child event absent")?;
    ensure!(matches.next().is_none(), "ambiguous parent Child event");
    Ok(matched)
}

struct CompletedChild<T> {
    result: T,
    run_id: Uuid,
    initiated_at: i64,
    completed_at: i64,
}

struct DagHistoryReader<'a> {
    client: &'a Client,
    namespace: &'a str,
    history: &'a History,
    parent: TemporalExecutionIdentityV4,
}

impl DagHistoryReader<'_> {
    async fn completed_child<T: DeserializeOwned + Serialize>(
        &self,
        workflow_id: &str,
        workflow_type: &str,
        queue: &str,
        request: &impl Serialize,
    ) -> Result<CompletedChild<T>> {
        let (initiated_at, initiated) =
            only_parent_event(self.history, |attributes| match attributes {
                Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(event)
                    if event.workflow_id == workflow_id =>
                {
                    Some(event)
                }
                _ => None,
            })?;
        let input = only_payload(initiated.input.as_ref(), DAG_MAX_INPUT_BYTES)?;
        ensure!(
            initiated
                .workflow_type
                .as_ref()
                .map(|kind| kind.name.as_str())
                == Some(workflow_type)
                && initiated.task_queue.as_ref().map(|q| q.name.as_str()) == Some(queue)
                && serde_json::to_vec(request)?.as_slice() == input,
            "Child invocation differs from the reconstructed DAG dispatch"
        );
        let (started_at, started) =
            only_parent_event(self.history, |attributes| match attributes {
                Attributes::ChildWorkflowExecutionStartedEventAttributes(event)
                    if event.initiated_event_id == initiated_at =>
                {
                    Some(event)
                }
                _ => None,
            })?;
        let execution = started
            .workflow_execution
            .as_ref()
            .context("Child server identity absent")?;
        let run_id = Uuid::parse_str(&execution.run_id)?;
        ensure!(
            execution.workflow_id == workflow_id
                && run_id.to_string() == execution.run_id
                && !run_id.is_nil()
                && started
                    .workflow_type
                    .as_ref()
                    .map(|kind| kind.name.as_str())
                    == Some(workflow_type),
            "Temporal Child Started does not match expected server identity"
        );
        let (completed_at, completed) =
            only_parent_event(self.history, |attributes| match attributes {
                Attributes::ChildWorkflowExecutionCompletedEventAttributes(event)
                    if event.initiated_event_id == initiated_at =>
                {
                    Some(event)
                }
                _ => None,
            })?;
        ensure!(
            completed.started_event_id == started_at
                && completed.workflow_execution.as_ref() == Some(execution)
                && initiated_at < started_at
                && started_at < completed_at,
            "Temporal Child completion does not match its start"
        );
        let parent_result = only_payload(completed.result.as_ref(), DAG_MAX_RESULT_BYTES)?;
        let history =
            fetch_exact_history(self.client, self.namespace, workflow_id, &execution.run_id)
                .await?;
        ensure!(
            history.events.len() <= 1_000,
            "Child History exceeds event budget"
        );
        ensure!(
            first_started_payload(&history, workflow_type, queue)? == input,
            "Child server Start differs from the parent ChildInitiated bytes"
        );
        let Some(Attributes::WorkflowExecutionStartedEventAttributes(own_start)) = history
            .events
            .first()
            .and_then(|event| event.attributes.as_ref())
        else {
            bail!("Child own History lacks Start");
        };
        let parent = own_start
            .parent_workflow_execution
            .as_ref()
            .context("Child own parent identity absent")?;
        ensure!(
            parent.workflow_id == self.parent.workflow_id
                && parent.run_id == self.parent.run_id
                && own_start.parent_initiated_event_id == initiated_at,
            "Child reciprocal server parent identity mismatches"
        );
        let (own_completed_at, own_completed) =
            only_parent_event(&history, |attributes| match attributes {
                Attributes::WorkflowExecutionCompletedEventAttributes(event) => Some(event),
                _ => None,
            })?;
        let own_result = only_payload(own_completed.result.as_ref(), DAG_MAX_RESULT_BYTES)?;
        ensure!(
            history
                .events
                .last()
                .is_some_and(|event| event.event_id == own_completed_at)
                && own_result == parent_result,
            "Child completed bytes differ across genuine server Histories"
        );
        let result: T = serde_json::from_slice(own_result)?;
        ensure!(
            serde_json::to_vec(&result)?.as_slice() == own_result,
            "noncanonical Child result"
        );
        Ok(CompletedChild {
            result,
            run_id,
            initiated_at,
            completed_at,
        })
    }

    async fn attest_completed_dag(
        &self,
        mut machine: OperationDagMachine,
        plan: &ExecutionPlan,
        receipt_bytes: &[u8],
    ) -> Result<OperationDagReadTraceV1> {
        let initiated_count = self
            .history
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.attributes.as_ref(),
                    Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(_))
                )
            })
            .count();
        ensure!(
            initiated_count == 2 * plan.nodes.len(),
            "unexpected V2 Child count"
        );

        let validations = machine.v2_owner_validation_requests(self.parent.clone())?;
        let mut validation_results = Vec::with_capacity(validations.len());
        let mut completed_through = 0;
        for (route, request) in validations {
            ensure!(
                route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1,
                "untrusted owner Child Workflow Type"
            );
            let workflow_id = owner_validation_child_workflow_id_v1(
                &self.parent,
                &request.plan_hash,
                &request.instance,
            );
            let child = self
                .completed_child::<OwnerValidationResultV1>(
                    &workflow_id,
                    OWNER_VALIDATION_CHILD_TYPE_V1,
                    &route.task_queue,
                    &request,
                )
                .await?;
            completed_through = completed_through.max(child.completed_at);
            validation_results.push(child.result);
        }
        machine.complete_v2_owner_validation(self.parent.clone(), &validation_results)?;

        let nodes: BTreeMap<_, _> = plan
            .nodes
            .iter()
            .map(|node| (&node.instance, node))
            .collect();
        let mut indices = BTreeMap::new();
        let mut steps = Vec::with_capacity(plan.nodes.len());
        while machine.next_instance().is_some() {
            let wave = machine.next_independent_owner_invocations(self.parent.clone())?;
            ensure!(!wave.is_empty(), "V2 DAG has no ready Child");
            let previous_wave = completed_through;
            for (route, invocation) in wave {
                let dispatch = &invocation.checked_dispatch;
                let workflow_id = owner_child_workflow_id_v2(&dispatch.execution_key);
                let child = self
                    .completed_child::<OwnerResultV1>(
                        &workflow_id,
                        &route.workflow_type,
                        &route.task_queue,
                        &invocation,
                    )
                    .await?;
                ensure!(
                    child.initiated_at > previous_wave,
                    "owner Child started before its validation or dependencies completed"
                );
                completed_through = completed_through.max(child.completed_at);

                let node = nodes[&dispatch.instance];
                let input_steps = node
                    .inputs
                    .values()
                    .filter_map(|binding| match binding {
                        ArtifactBinding::Produced { producer, .. } => Some(indices[producer]),
                        ArtifactBinding::External { .. } => None,
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let receipt_blake3 = owner_content_hash(&child.result.receipt_bytes);
                machine.advance_owner_result(child.result)?;
                indices.insert(dispatch.instance.clone(), steps.len());
                steps.push(OperationDagReadStepV1 {
                    operation: node.operation.clone(),
                    input_steps,
                    child_workflow_id: workflow_id,
                    child_run_id: child.run_id,
                    receipt_blake3,
                });
            }
        }
        // Reconstruct the entire typed receipt using the same reducer as the
        // parent. Comparing exact bytes also ties External and Produced
        // artifacts to independently fetched workflow events.
        ensure!(
            serde_json::to_vec(&machine.finish()?)?.as_slice() == receipt_bytes,
            "parent completion differs from the reconstructed DAG receipt"
        );
        Ok(OperationDagReadTraceV1 {
            schema_revision: DAG_READ_TRACE_SCHEMA_REVISION_V1,
            steps,
        })
    }
}

async fn read_only_pool(expected_tenant: &str) -> Result<PgPool> {
    tenant_read_only_pool(
        expected_tenant,
        "OPERATION_DAG_READ_V2_DATABASE_URL_FILE",
        true,
    )
    .await
}

/// Identical fail-closed SQL principal check for the start preflight's own
/// SELECT-only tenant role. A starter need not read the governance table
/// because its own job is to prove a preexisting business root before Start;
/// it must never possess issuer INSERT/revocation permissions.
pub(super) async fn tenant_read_only_pool(
    expected_tenant: &str,
    dsn_file_env: &str,
    require_governance_read: bool,
) -> Result<PgPool> {
    let path = env::var(dsn_file_env)
        .context("read-only V2 operator requires private SQL credential file")?;
    let bytes = read_private_operator_file(Path::new(&path), "V2 read-only database DSN")?;
    let dsn = String::from_utf8(bytes)?;
    let dsn = dsn.trim_end_matches(|character| character == '\r' || character == '\n');
    ensure!(!dsn.is_empty() && dsn.trim() == dsn, "invalid reader DSN");
    let pool = PgPool::connect(dsn).await?;
    let role = sqlx::query(
        "SELECT current_user::text effective,session_user::text login,
                r.rolcanlogin login_enabled,r.rolinherit inherits_roles,
                r.rolsuper privileged,r.rolbypassrls bypass,
                r.rolcreaterole role_admin,r.rolcreatedb db_admin,
                r.rolreplication replication,
                has_table_privilege(session_user,
                    'pipeline.operation_dag_business_runs_v1','SELECT') can_read_root,
                has_table_privilege(session_user,
                    'pipeline.operation_dag_run_governance_v2','SELECT') can_read_governance
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("privileged")
            && !role.get::<bool, _>("bypass")
            && !role.get::<bool, _>("role_admin")
            && !role.get::<bool, _>("db_admin")
            && !role.get::<bool, _>("replication")
            && role.get::<bool, _>("can_read_root")
            && role.get::<bool, _>("can_read_governance") == require_governance_read,
        "V2 reader is not a distinct nonprivileged SELECT-only tenant LOGIN"
    );
    let member_of_other_role: Option<String> = sqlx::query_scalar(
        "SELECT inherited.rolname::text
         FROM pg_catalog.pg_auth_members membership
         JOIN pg_catalog.pg_roles inherited ON inherited.oid=membership.roleid
         WHERE membership.member=(SELECT oid FROM pg_catalog.pg_roles
                                  WHERE rolname=session_user)
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        member_of_other_role.is_none(),
        "V2 reader must not inherit or SET ROLE into another SQL principal"
    );
    let unexpected_mutation: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname NOT IN ('information_schema')
           AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (has_any_column_privilege(current_user,c.oid,'INSERT')
                OR has_any_column_privilege(current_user,c.oid,'UPDATE')
                OR has_table_privilege(current_user,c.oid,'DELETE')
                OR has_table_privilege(current_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        unexpected_mutation.is_none(),
        "V2 reader has a mutation capability and cannot inspect runs"
    );
    let actual_tenant: Option<String> =
        sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
            .fetch_one(&pool)
            .await?;
    ensure!(
        actual_tenant.as_deref() == Some(expected_tenant),
        "reader SQL LOGIN tenant binding differs from the configured tenant"
    );
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_accepts_only_one_canonical_uuid_locator() {
        let id = Uuid::new_v4();
        let valid = format!("{{\"canonical_run_id\":\"{id}\"}}");
        assert_eq!(
            decode_canonical_run_locator(valid.as_bytes())
                .unwrap()
                .canonical_run_id,
            id
        );
        assert!(decode_canonical_run_locator(
            format!("{{\"canonical_run_id\":\"{id}\",\"tenant_id\":\"forged\"}}").as_bytes()
        )
        .is_err());
        assert!(decode_canonical_run_locator(
            format!("{{\"canonical_run_id\":\"{id}\",\"parent_workflow_run_id\":\"forged\"}}")
                .as_bytes()
        )
        .is_err());
        let uppercase_uuid = format!(
            "{{\"canonical_run_id\":\"{}\"}}",
            id.to_string().to_uppercase()
        );
        assert!(decode_canonical_run_locator(uppercase_uuid.as_bytes()).is_err());
        assert!(decode_canonical_run_locator(format!("{valid}\n").as_bytes()).is_err());
        assert!(decode_canonical_run_locator(&[b' '; 257]).is_err());
        assert!(decode_canonical_run_locator(
            b"{\"canonical_run_id\":\"00000000-0000-0000-0000-000000000000\"}"
        )
        .is_err());
    }

    #[test]
    fn safe_diagnostic_has_no_history_payload_or_provider_success() {
        let report = SafeRunInspection {
            operation: OPERATION,
            status: "temporal_completed_owner_unverified",
            canonical_run_id: Uuid::new_v4(),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            plan_hash: "a".repeat(64),
            parent_start_sha256: "b".repeat(64),
            step_count: 1,
            owner_result_count: 1,
            result_blake3: Some("c".repeat(64)),
            trace: None,
            governance_currently_active: false,
            temporal_history_attestation:
                "exact_server_parent_validation_and_effect_child_histories",
            owner_receipt_attestation: "not_independently_verified",
            source_authentication: "not_independently_attested",
            next_action: "Get an independently verified owner receipt.",
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("temporal_completed_owner_unverified"));
        assert!(json.contains("not_independently_verified"));
        assert!(!json.contains("receipt_bytes"));
        assert!(!json.contains("payload"));
        assert!(!json.contains("provider_success"));
        assert!(!json.contains("command_bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn socket_requires_one_bounded_canonical_uuid_line_and_exact_eof() {
        let id = Uuid::new_v4();
        let wire = format!("{{\"canonical_run_id\":\"{id}\"}}\n");
        assert_eq!(
            decode_socket_locator_line(wire.as_bytes())
                .unwrap()
                .canonical_run_id,
            id
        );
        for bad in [
            wire.trim_end_matches('\n').to_owned(),
            format!("{wire}\n"),
            format!("{wire}{{}}\n"),
            format!("{{\"canonical_run_id\":\"{id}\",\"tenant_id\":\"other\"}}\n"),
            format!(" {{\"canonical_run_id\":\"{id}\"}}\n"),
            format!(
                "{{\"canonical_run_id\":\"{}\"}}\n",
                id.to_string().to_uppercase()
            ),
            "x".repeat(MAX_SOCKET_REQUEST_BYTES + 1),
        ] {
            assert!(
                decode_socket_locator_line(bad.as_bytes()).is_err(),
                "unexpectedly accepted: {bad}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refusal_diagnostic_is_constant_without_tenant_or_history_data() {
        let report = serde_json::to_value(refusal_diagnostic()).unwrap();
        let wire = report.to_string();
        assert_eq!(report["status"], "refused");
        assert_eq!(report["code"], "run_unavailable_or_untrusted");
        for forbidden in [
            "tenant_id",
            "run_id",
            "workflow_run_id",
            "sql",
            "dsn",
            "receipt_bytes",
            "provider_payload",
        ] {
            assert!(!wire.contains(forbidden), "refusal leaked {forbidden}");
        }
        assert!(wire.len() < DAG_READ_MAX_REPLY_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn unix_status_peer_must_be_exact_distinct_nonroot_uid() {
        assert!(authorized_client_uid(1042, 1042, 1017));
        assert!(!authorized_client_uid(1042, 1043, 1017));
        assert!(!authorized_client_uid(1042, 1042, 1042));
        assert!(!authorized_client_uid(0, 0, 1017));
        assert!(!authorized_client_uid(0, 1000, 0));
    }
}
