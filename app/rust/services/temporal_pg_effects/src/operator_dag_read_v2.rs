//! Read-only, tenant-scoped proof of ONE native V2 Temporal operation.
//!
//! This operator does not issue root authority, start a Workflow, signal a
//! Child, invoke a provider or claim that an owner's durable external effect
//! was independently verified. Only the exact server History is observed.

use std::{env, path::Path};

#[cfg(unix)]
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    enforce_json_budget, operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2,
    owner_content_hash, owner_validation_child_workflow_id_v1, OperationDagReceiptV1,
    OperationDagRunV6, OwnerInvocationV1, OwnerResultV1, OwnerValidationRequestV1,
    OwnerValidationResultV1, TemporalExecutionIdentityV4, TrustedRouteCatalog, DAG_MAX_INPUT_BYTES,
    DAG_MAX_RESULT_BYTES, OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    OWNER_OPERATION_CHILD_TYPE_V1, OWNER_VALIDATION_CHILD_TYPE_V1,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use temporalio_client::Client;
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
#[cfg(unix)]
const MAX_SOCKET_RESPONSE_BYTES: usize = 2 * 1024;

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
    if payload.len() > MAX_SOCKET_RESPONSE_BYTES {
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
    ensure!(
        history.events.len() <= 1_000,
        "one-step server History is too large"
    );
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
        run.schema_revision == 6
            && serde_json::to_vec(&run)? == start
            && run.plan.scope.run_id == locator.canonical_run_id.to_string()
            && run.plan.scope.tenant_id == tenant
            && run.plan.hash.as_str() == plan_hash
            && run.plan.nodes.len() == 1
            && run.owner_commands.len() == 1
            && run.pinned_routes.len() == 1,
        "server Start is not the exact canonical one-operation V2 run"
    );
    let validated = run.plan.validate(catalog.catalog())?;
    catalog.validate_run_routes(&run)?;
    let node = &run.plan.nodes[0];
    ensure!(
        run.owner_commands.contains_key(&node.instance),
        "single operation command absent"
    );

    let (status, receipt_hash, owner_result_count, temporal_proof, next_action) =
        classify_server_result(&history, &run, validated.execution_order())?;
    if status == "temporal_completed_owner_unverified" {
        attest_one_owner_child(
            &client,
            &namespace,
            &history,
            &run,
            &catalog,
            &workflow_id,
            &temporal_run_id,
        )
        .await?;
    }
    Ok(SafeRunInspection {
        operation: OPERATION,
        status,
        canonical_run_id: locator.canonical_run_id,
        parent_workflow_run_id: temporal_run_id,
        plan_hash,
        parent_start_sha256: start_sha256,
        step_count: 1,
        owner_result_count,
        result_blake3: receipt_hash,
        governance_currently_active: governance_active,
        temporal_history_attestation: if status == "temporal_completed_owner_unverified" {
            "exact_server_parent_validation_and_effect_child_histories"
        } else {
            temporal_proof
        },
        owner_receipt_attestation: "not_independently_verified",
        source_authentication: "not_independently_attested",
        next_action,
    })
}

fn classify_server_result(
    history: &History,
    run: &OperationDagRunV6,
    expected_order: &[operation_protocol::OperationInstanceId],
) -> Result<(
    &'static str,
    Option<String>,
    usize,
    &'static str,
    &'static str,
)> {
    let completed: Vec<_> = history
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::WorkflowExecutionCompletedEventAttributes(done)) => Some(done),
            _ => None,
        })
        .collect();
    let terminal_count = history
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.attributes.as_ref(),
                Some(Attributes::WorkflowExecutionCompletedEventAttributes(_))
                    | Some(Attributes::WorkflowExecutionFailedEventAttributes(_))
                    | Some(Attributes::WorkflowExecutionCanceledEventAttributes(_))
                    | Some(Attributes::WorkflowExecutionTimedOutEventAttributes(_))
                    | Some(Attributes::WorkflowExecutionTerminatedEventAttributes(_))
                    | Some(Attributes::WorkflowExecutionContinuedAsNewEventAttributes(
                        _
                    ))
            )
        })
        .count();
    ensure!(
        terminal_count <= 1 && completed.len() <= 1,
        "contradictory native V2 terminal events"
    );
    if let Some(done) = completed.first() {
        ensure!(
            matches!(
                history
                    .events
                    .last()
                    .and_then(|event| event.attributes.as_ref()),
                Some(Attributes::WorkflowExecutionCompletedEventAttributes(_))
            ),
            "Temporal completion is not the last server History event"
        );
        let payloads = done
            .result
            .as_ref()
            .context("completed native V2 result lacks payload")?;
        ensure!(
            payloads.payloads.len() == 1
                && !payloads.payloads[0].data.is_empty()
                && payloads.payloads[0].data.len() <= DAG_MAX_RESULT_BYTES,
            "completed native V2 result is missing or exceeds budget"
        );
        let bytes = &payloads.payloads[0].data;
        let receipt: OperationDagReceiptV1 = serde_json::from_slice(bytes)?;
        ensure!(
            serde_json::to_vec(&receipt)?.as_slice() == bytes.as_slice(),
            "noncanonical native V2 result bytes"
        );
        enforce_json_budget(&receipt, DAG_MAX_RESULT_BYTES, "v2_native_read_receipt")?;
        ensure!(
            receipt.plan_hash == run.plan.hash
                && receipt.order.as_slice() == expected_order
                && receipt.owner_results.len() == 1
                && receipt
                    .owner_results
                    .contains_key(&run.plan.nodes[0].instance)
                && receipt.produced_evidence.len() == run.plan.nodes[0].outputs.len()
                && receipt.artifacts.len()
                    == run.plan.external_artifacts.len() + run.plan.nodes[0].outputs.len(),
            "native V2 result does not match one exact operation and its sealed plan"
        );
        // The parent may be complete while a malicious/stale receipt swaps
        // externally admitted source bytes. A metadata-only comparison is
        // insufficient: exact source bytes were sealed in the parent Start.
        for (artifact_id, source) in &run.admitted {
            ensure!(
                receipt.artifacts.get(artifact_id) == Some(source),
                "native V2 receipt altered the original admitted source bytes"
            );
        }
        return Ok((
            "temporal_completed_owner_unverified",
            Some(owner_content_hash(bytes)),
            1,
            "exact_server_parent_start_and_completion",
            "Ask the installed owner for an independently attested durable provider receipt before treating outputs as research evidence.",
        ));
    }
    let terminal_status = history
        .events
        .last()
        .and_then(|event| match &event.attributes {
            Some(Attributes::WorkflowExecutionFailedEventAttributes(_)) => Some("temporal_failed"),
            Some(Attributes::WorkflowExecutionCanceledEventAttributes(_)) => {
                Some("temporal_canceled")
            }
            Some(Attributes::WorkflowExecutionTimedOutEventAttributes(_)) => {
                Some("temporal_timed_out")
            }
            Some(Attributes::WorkflowExecutionTerminatedEventAttributes(_)) => {
                Some("temporal_terminated")
            }
            Some(Attributes::WorkflowExecutionContinuedAsNewEventAttributes(_)) => {
                Some("temporal_continued_as_new")
            }
            _ => None,
        });
    if let Some(status) = terminal_status {
        return Ok((
            status,
            None,
            0,
            "exact_server_parent_start_and_terminal_event",
            "Inspect the server-side terminal step without exposing raw Activity or provider errors to this reader.",
        ));
    }
    Ok((
        "temporal_running",
        None,
        0,
        "exact_server_parent_start_only",
        "Wait for the native Temporal Workflow to finish, then read this same business run again.",
    ))
}

/// Parent completion by itself is not a provider receipt. At least verify the
/// native owner's *separate* actual Temporal Child History, exact typed input,
/// reciprocal server parent identity, typed completion and parent receipt.
/// SQL/provider readback belongs to the independently installed owner.
async fn attest_one_owner_child(
    client: &Client,
    namespace: &str,
    parent: &History,
    run: &OperationDagRunV6,
    catalog: &TrustedRouteCatalog,
    parent_workflow_id: &str,
    parent_run_id: &str,
) -> Result<()> {
    let node = &run.plan.nodes[0];
    let route = &run.pinned_routes[0];
    let parent_identity = TemporalExecutionIdentityV4 {
        workflow_id: parent_workflow_id.to_owned(),
        run_id: parent_run_id.to_owned(),
    };
    let validation_id =
        owner_validation_child_workflow_id_v1(&parent_identity, &run.plan.hash, &node.instance);
    let wire = run
        .owner_commands
        .get(&node.instance)
        .context("single owner command not pinned")?;
    ensure!(
        route.operation == node.operation
            && route.owner == wire.owner
            && route.contract_hash == wire.contract_hash
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1,
        "one-owner trusted route/command mismatch"
    );
    let child_initiations = parent
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
        child_initiations == 2,
        "one-node V2 run must schedule exactly one ValidateOnly and one effect Child"
    );
    let validations = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs))
                if attrs.workflow_id == validation_id =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        validations.len() == 1
            && validations[0]
                .1
                .workflow_type
                .as_ref()
                .map(|typed| typed.name.as_str())
                == Some(OWNER_VALIDATION_CHILD_TYPE_V1)
            && validations[0]
                .1
                .task_queue
                .as_ref()
                .map(|queue| queue.name.as_str())
                == Some(route.task_queue.as_str()),
        "exactly one typed owner ValidateOnly must be scheduled"
    );
    let validate_request_bytes = validations[0]
        .1
        .input
        .as_ref()
        .and_then(|input| (input.payloads.len() == 1).then_some(input.payloads[0].data.as_slice()))
        .context("ValidateOnly ChildInitiated lacks exact typed request bytes")?;
    ensure!(
        !validate_request_bytes.is_empty() && validate_request_bytes.len() <= DAG_MAX_INPUT_BYTES,
        "ValidateOnly typed request exceeds bounded one-node input"
    );
    let validate_request: OwnerValidationRequestV1 =
        serde_json::from_slice(validate_request_bytes)?;
    validate_request.verify_against_run(run)?;
    ensure!(
        serde_json::to_vec(&validate_request)?.as_slice() == validate_request_bytes
            && validate_request.parent == parent_identity
            && validate_request.instance == node.instance
            && validate_request.operation == node.operation
            && validate_request.owner == route.owner
            && validate_request.contract_hash == wire.contract_hash
            && validate_request.command_bytes == wire.command_bytes,
        "ValidateOnly request does not match exact immutable parent command"
    );
    let validate_starts = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs))
                if attrs.initiated_event_id == validations[0].0.event_id =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        validate_starts.len() == 1,
        "ValidateOnly Child must start exactly once"
    );
    let validate_server_run = validate_starts[0]
        .1
        .workflow_execution
        .as_ref()
        .context("ValidateOnly Child lacks independently issued server Run ID")?;
    ensure!(
        validate_server_run.workflow_id == validation_id
            && validate_starts[0]
                .1
                .workflow_type
                .as_ref()
                .map(|typed| typed.name.as_str())
                == Some(OWNER_VALIDATION_CHILD_TYPE_V1),
        "ValidateOnly server ChildStarted mismatches pinned workflow identity"
    );
    let validated = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(attrs))
                if attrs.initiated_event_id == validations[0].0.event_id
                    && attrs.started_event_id == validate_starts[0].0.event_id =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        validated.len() == 1,
        "owner ValidateOnly did not complete once"
    );
    let validate_parent_result = validated[0]
        .1
        .result
        .as_ref()
        .and_then(|result| {
            (result.payloads.len() == 1).then_some(result.payloads[0].data.as_slice())
        })
        .context("ValidateOnly parent completion lacks typed result")?;
    let validate_child = fetch_exact_history(
        client,
        namespace,
        &validation_id,
        &validate_server_run.run_id,
    )
    .await?;
    ensure!(
        validate_child.events.len() <= 1_000,
        "ValidateOnly own server History exceeds one-node budget"
    );
    let validate_child_start = first_started_payload(
        &validate_child,
        OWNER_VALIDATION_CHILD_TYPE_V1,
        &route.task_queue,
    )?;
    ensure!(
        validate_child_start == validate_request_bytes,
        "ValidateOnly own server Start differs from parent ChildInitiated bytes"
    );
    let own_first = validate_child
        .events
        .first()
        .context("ValidateOnly own server History lacks Start")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(own_start)) =
        own_first.attributes.as_ref()
    else {
        bail!("ValidateOnly own server History has no Start attributes")
    };
    let validation_parent = own_start
        .parent_workflow_execution
        .as_ref()
        .context("ValidateOnly own server Start lacks real parent identity")?;
    ensure!(
        validation_parent.workflow_id == parent_workflow_id
            && validation_parent.run_id == parent_run_id
            && own_start.parent_initiated_event_id == validations[0].0.event_id,
        "ValidateOnly reciprocal server parent identity mismatches"
    );
    let validate_done = validate_child
        .events
        .last()
        .context("ValidateOnly own server History lacks completion")?;
    let Some(Attributes::WorkflowExecutionCompletedEventAttributes(validate_done)) =
        validate_done.attributes.as_ref()
    else {
        bail!("ValidateOnly own server History did not complete successfully")
    };
    let validate_own_result = validate_done
        .result
        .as_ref()
        .and_then(|result| {
            (result.payloads.len() == 1).then_some(result.payloads[0].data.as_slice())
        })
        .context("ValidateOnly own completion lacks exact typed result")?;
    ensure!(
        validate_own_result == validate_parent_result,
        "ValidateOnly completed bytes differ between independent server Histories"
    );
    let accepted: OwnerValidationResultV1 = serde_json::from_slice(validate_own_result)?;
    ensure!(
        serde_json::to_vec(&accepted)?.as_slice() == validate_own_result,
        "ValidateOnly typed acceptance contains noncanonical bytes"
    );
    accepted.verify_success(&validate_request)?;

    let effects = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs))
                if attrs.workflow_id.starts_with("alegria-owner-operation-v2-") =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        effects.len() == 1,
        "exactly one V2 owner effect child is required"
    );
    let (initiated, effect) = effects[0];
    ensure!(
        effect
            .workflow_type
            .as_ref()
            .map(|typed| typed.name.as_str())
            == Some(OWNER_OPERATION_CHILD_TYPE_V1)
            && effect.task_queue.as_ref().map(|queue| queue.name.as_str())
                == Some(route.task_queue.as_str()),
        "owner effect Child Type or Task Queue mismatches installed route"
    );
    let started = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs))
                if attrs.initiated_event_id == initiated.event_id =>
            {
                Some((event, attrs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        started.len() == 1
            && validated[0].0.event_id < initiated.event_id
            && validated[0].0.event_id < started[0].0.event_id,
        "owner effect was scheduled or started before typed ValidateOnly completed"
    );
    let child_run = started[0]
        .1
        .workflow_execution
        .as_ref()
        .context("owner Child server execution identity missing")?;
    ensure!(
        child_run.workflow_id == effect.workflow_id && !child_run.run_id.is_empty(),
        "owner Child server identity mismatches initiated event"
    );
    let child =
        fetch_exact_history(client, namespace, &child_run.workflow_id, &child_run.run_id).await?;
    ensure!(
        child.events.len() <= 1_000,
        "owner Child History exceeds one-step budget"
    );
    let child_start =
        first_started_payload(&child, OWNER_OPERATION_CHILD_TYPE_V1, &route.task_queue)?;
    ensure!(
        child_start.len() <= DAG_MAX_INPUT_BYTES
            && effect.input.as_ref().is_some_and(|input| {
                input.payloads.len() == 1
                    && input.payloads[0].data.as_slice() == child_start.as_slice()
            }),
        "server parent/Child command bytes differ"
    );
    let own_start = child.events.first().context("missing owner Child start")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(own_start)) =
        &own_start.attributes
    else {
        bail!("owner Child own History lacks start attributes")
    };
    let actual_parent = own_start
        .parent_workflow_execution
        .as_ref()
        .context("owner Child lacks real server parent")?;
    ensure!(
        actual_parent.workflow_id == parent_workflow_id
            && actual_parent.run_id == parent_run_id
            && own_start.parent_initiated_event_id == initiated.event_id,
        "owner Child reciprocal Temporal parent identity mismatches"
    );
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_start)?;
    ensure!(
        serde_json::to_vec(&invocation)? == child_start
            && invocation.parent.workflow_id == parent_workflow_id
            && invocation.parent.run_id == parent_run_id
            && invocation.checked_dispatch.instance == node.instance
            && invocation.checked_dispatch.plan_hash == run.plan.hash
            && invocation.checked_dispatch.scope == run.plan.scope
            && invocation.checked_dispatch.operation == node.operation
            && invocation.checked_dispatch.owner == route.owner
            && wire.operation == node.operation
            && wire.command_bytes == invocation.command_bytes
            && effect.workflow_id
                == owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key,),
        "native owner Child typed command differs from exact parent Start"
    );
    catalog.route_for_dispatch(&invocation.checked_dispatch)?;

    let parent_completions = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(attrs))
                if attrs.initiated_event_id == initiated.event_id =>
            {
                Some(attrs)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let child_completions = child
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::WorkflowExecutionCompletedEventAttributes(attrs)) => Some(attrs),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        parent_completions.len() == 1 && child_completions.len() == 1,
        "owner Child lacks one exact parent/own completed result"
    );
    let parent_result = parent_completions[0]
        .result
        .as_ref()
        .and_then(|result| {
            (result.payloads.len() == 1).then_some(result.payloads[0].data.as_slice())
        })
        .context("owner parent ChildCompleted has no typed result")?;
    let own_result = child_completions[0]
        .result
        .as_ref()
        .and_then(|result| {
            (result.payloads.len() == 1).then_some(result.payloads[0].data.as_slice())
        })
        .context("owner Child own History has no typed result")?;
    ensure!(
        !parent_result.is_empty()
            && parent_result.len() <= DAG_MAX_RESULT_BYTES
            && parent_result == own_result,
        "owner Child completed bytes differ across genuine server Histories"
    );
    let result: OwnerResultV1 = serde_json::from_slice(own_result)?;
    ensure!(
        serde_json::to_vec(&result)?.as_slice() == own_result,
        "noncanonical owner Child completed result"
    );
    result.verify_integrity(&invocation.checked_dispatch, catalog)?;

    let parent_done = parent
        .events
        .last()
        .context("native DAG lacks terminal completed event")?;
    let Some(Attributes::WorkflowExecutionCompletedEventAttributes(parent_done)) =
        &parent_done.attributes
    else {
        bail!("native DAG completed result does not terminate parent History")
    };
    let receipt_bytes = parent_done
        .result
        .as_ref()
        .and_then(|result| {
            (result.payloads.len() == 1).then_some(result.payloads[0].data.as_slice())
        })
        .context("native DAG parent completion lacks typed receipt")?;
    let receipt: OperationDagReceiptV1 = serde_json::from_slice(receipt_bytes)?;
    let stored_result = receipt
        .owner_results
        .get(&node.instance)
        .context("native DAG receipt does not carry completed owner")?;
    ensure!(
        serde_json::to_vec(stored_result)?.as_slice() == own_result,
        "native DAG final receipt differs from independently fetched owner Child"
    );
    let result_hash = owner_content_hash(own_result);
    for (port, artifact_id) in &node.outputs {
        let output = result
            .outputs
            .get(port)
            .context("real owner Child output port absent")?;
        let final_artifact = receipt
            .artifacts
            .get(artifact_id)
            .context("native DAG final output artifact absent")?;
        let evidence = receipt
            .produced_evidence
            .get(artifact_id)
            .context("native DAG produced evidence absent")?;
        ensure!(
            final_artifact == output
                && evidence.producer == node.instance
                && &evidence.port == port
                && &evidence.artifact_id == artifact_id
                && evidence.dispatch_execution_key == invocation.checked_dispatch.execution_key
                && evidence.child_workflow_id == effect.workflow_id
                && evidence.result_hash == result_hash
                && evidence.receipt_hash == owner_content_hash(&result.receipt_bytes)
                && evidence.payload_hash == owner_content_hash(&output.payload),
            "native DAG output differs from independently attested owner Child result"
        );
    }
    Ok(())
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
        assert!(wire.len() <= MAX_SOCKET_RESPONSE_BYTES);
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
