//! Opt-in, tenant-root-checked Start of an immutable one-node Temporal DAG V2.
//!
//! The private request is an operator-controlled snapshot, never an MCP tool
//! argument. It contains independently pinned command/source digests, not
//! digests fabricated from the user-supplied run during this invocation.
//! Starting a Workflow does not authenticate sources, issue a tenant business
//! root, authorize owner effects or attest a completed provider result.

use std::{
    env,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, preflight_unadmitted_single_operation_v2_start,
    verify_external_source_record_v1, ArtifactId, OperationDagRunV6, PlanHash,
    SiteLabExpectedStartV1, TrustedExternalSourceRecordV1, TrustedRouteCatalog,
    EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1, OPERATION_DAG_PARENT_QUEUE_V2,
    OPERATION_DAG_PARENT_TYPE_V2,
};
use serde::{Deserialize, Serialize};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, UntypedWorkflow,
    WorkflowIdConflictPolicy, WorkflowIdReusePolicy, WorkflowStartOptions,
};
use temporalio_common::data_converters::{PayloadConverter, RawValue};
use tokio::time::timeout;
use url::Url;

use sqlx::{PgPool, Row};

use super::{operator_dag_governance, operator_dag_read_v2};

const ENABLE_ENV: &str = "ALEGRIA_ENABLE_SITE_LAB_SINGLE_V2_START";
const TENANT_ENV: &str = "SITE_LAB_SINGLE_V2_TENANT_ID";
const TEMPORAL_URL_ENV: &str = "SITE_LAB_SINGLE_V2_TEMPORAL_URL";
const TEMPORAL_NAMESPACE_ENV: &str = "SITE_LAB_SINGLE_V2_TEMPORAL_NAMESPACE";
const CATALOG_ENV: &str = "SITE_LAB_SINGLE_V2_TRUSTED_CATALOG_FILE";
const ROOT_READER_DSN_ENV: &str = "SITE_LAB_SINGLE_V2_DATABASE_URL_FILE";
const MAX_REQUEST_FUTURE_SECONDS: u64 = 600;
const MAX_CONTEXT_KEY_BYTES: usize = 512;
const MAX_POLICY_ID_BYTES: usize = 128;

/// Only the Start RPC has an ambiguous outcome: Temporal may persist the
/// Workflow even when the caller loses the response. All earlier stages can
/// fail without submitting an execution command. These states are part of the
/// operator's diagnostic protocol, never a client-controlled admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleStartStage {
    OperatorConfiguration,
    PrivateIntent,
    IndependentBusinessRoot,
    ExternalSourceAttestation,
    TemporalConnection,
    FinalPreflight,
    StartSubmitted,
}

impl SingleStartStage {
    fn status(self) -> &'static str {
        match self {
            Self::StartSubmitted => "start_outcome_unknown",
            _ => "refused_before_temporal_start",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::OperatorConfiguration => "operator_configuration_unavailable",
            Self::PrivateIntent => "private_intent_invalid_or_untrusted",
            Self::IndependentBusinessRoot => "independent_business_root_unavailable",
            Self::ExternalSourceAttestation => {
                "independent_external_source_attestation_unavailable"
            }
            Self::TemporalConnection => "temporal_connection_unavailable",
            Self::FinalPreflight => "authorization_expired_or_revoked",
            Self::StartSubmitted => "temporal_start_result_unknown",
        }
    }

    fn next_action(self) -> &'static str {
        match self {
            Self::StartSubmitted => {
                "Inspect the exact Workflow ID in Temporal and the tenant-bound SQL root; never retry a Start merely because its response was lost."
            }
            Self::OperatorConfiguration => {
                "Ask the operator to check separately provisioned tenant, catalog and loopback Temporal settings."
            }
            Self::PrivateIntent => {
                "Supply a fresh, canonical, independently pinned private V2 intent."
            }
            Self::IndependentBusinessRoot | Self::FinalPreflight => {
                "Check independent tenant business-root issuance and the finite request validity window."
            }
            Self::ExternalSourceAttestation => {
                "Have the separately credentialed Acquisition source-transform issuer derive the exact typed payload from pinned tenant-owned raw HTML, then commit one immutable source record per input before Temporal Start."
            }
            Self::TemporalConnection => {
                "Restore the independently configured loopback Temporal endpoint before requesting a fresh intent."
            }
        }
    }
}

fn bounded_policy_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_POLICY_ID_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn bounded_context_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTEXT_KEY_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

#[derive(Serialize)]
struct SingleStartFailureReport<'a> {
    schema_revision: u16,
    operation: &'static str,
    status: &'static str,
    code: &'static str,
    canonical_run_id: Option<&'a str>,
    parent_workflow_id: Option<&'a str>,
    execution: &'static str,
    owner_admission: &'static str,
    next_action: &'static str,
}

fn start_failure_report<'a>(
    stage: SingleStartStage,
    canonical_run_id: Option<&'a str>,
    parent_workflow_id: Option<&'a str>,
) -> SingleStartFailureReport<'a> {
    SingleStartFailureReport {
        schema_revision: 1,
        operation: "site_lab_start_single_v2",
        status: stage.status(),
        code: stage.code(),
        canonical_run_id,
        parent_workflow_id,
        execution: "not_confirmed",
        owner_admission: "not_issued_by_start",
        next_action: stage.next_action(),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SingleOperationStartIntentV1 {
    schema_revision: u16,
    run: OperationDagRunV6,
    context_key: String,
    expected: SiteLabExpectedStartV1,
    expires_at_unix_seconds: u64,
}

struct OperatorConfig {
    tenant: String,
    url: Url,
    namespace: String,
    catalog: String,
}

fn checked_local_temporal_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("parse operator-only Temporal endpoint")?;
    ensure!(
        url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "single-step operator requires exact loopback HTTP Temporal endpoint"
    );
    Ok(url)
}

fn checked_operator_config(read: impl Fn(&str) -> Option<String>) -> Result<OperatorConfig> {
    ensure!(
        read(ENABLE_ENV).as_deref() == Some("true"),
        "single-step Temporal Start is not operator-enabled"
    );
    let tenant = read(TENANT_ENV).context("missing independently configured operator tenant")?;
    ensure!(
        bounded_policy_identifier(&tenant),
        "invalid operator-scoped tenant"
    );
    let url = checked_local_temporal_url(
        &read(TEMPORAL_URL_ENV).context("missing operator-only Temporal endpoint")?,
    )?;
    let namespace =
        read(TEMPORAL_NAMESPACE_ENV).context("missing operator-only Temporal namespace")?;
    ensure!(
        bounded_policy_identifier(&namespace),
        "invalid operator Temporal namespace"
    );
    let catalog = read(CATALOG_ENV).context("missing installed trusted owner catalog")?;
    ensure!(
        !catalog.is_empty() && catalog == catalog.trim(),
        "invalid trusted owner catalog path"
    );
    Ok(OperatorConfig {
        tenant,
        url,
        namespace,
        catalog,
    })
}

fn checked_request_ttl(expires_at: u64, now: u64) -> Result<()> {
    ensure!(
        expires_at > now && expires_at <= now.saturating_add(MAX_REQUEST_FUTURE_SECONDS),
        "single-step operator request expired or exceeds ten-minute authorization window"
    );
    Ok(())
}

fn decode_checked_intent(
    bytes: &[u8],
    tenant: &str,
    catalog: &TrustedRouteCatalog,
    now: u64,
) -> Result<SingleOperationStartIntentV1> {
    let intent: SingleOperationStartIntentV1 =
        serde_json::from_slice(bytes).context("decode typed private single-operation intent")?;
    ensure!(
        serde_json::to_vec(&intent)?.as_slice() == bytes,
        "single-step operator request must be canonical typed JSON"
    );
    ensure!(
        intent.schema_revision == 1,
        "unsupported single-step operator intent revision"
    );
    checked_request_ttl(intent.expires_at_unix_seconds, now)?;
    ensure!(
        intent.run.plan.scope.tenant_id == tenant,
        "single-step operator intent tenant differs from authenticated operator policy"
    );
    ensure!(
        bounded_context_key(&intent.context_key),
        "single-step operator requires exact nonblank business-root context"
    );
    let run_id = &intent.run.plan.scope.run_id;
    let uuid = uuid::Uuid::parse_str(run_id)
        .context("single-step operator requires canonical UUID run_id")?;
    ensure!(
        uuid.to_string() == *run_id && !uuid.is_nil(),
        "single-step operator run_id must be canonical nonnil UUID"
    );
    preflight_unadmitted_single_operation_v2_start(&intent.run, catalog, &intent.expected)
        .context("single-step operator rejected by native immutable V2 preflight")?;
    Ok(intent)
}

async fn assert_active_independent_business_root(
    pool: &PgPool,
    tenant: &str,
    intent: &SingleOperationStartIntentV1,
) -> Result<()> {
    // A root exists only if the independent tenant-authenticated business
    // issuer committed the canonical scope. The start role has SELECT only.
    // A short remaining TTL is rejected: governance and the owner each need
    // time to attest the actual Temporal History before any provider effect.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pipeline.operation_dag_business_runs_v1
            WHERE run_id=$1 AND tenant_id=$2 AND context_key=$3 AND plan_hash=$4
              AND parent_workflow_id=$5 AND revoked_at IS NULL
              AND expires_at > clock_timestamp() + interval '90 seconds'
        )",
    )
    .bind(uuid::Uuid::parse_str(&intent.run.plan.scope.run_id)?)
    .bind(tenant)
    .bind(&intent.context_key)
    .bind(intent.run.plan.hash.as_str())
    .bind(operation_dag_parent_workflow_id_v2(
        &intent.run.plan.scope.run_id,
    ))
    .fetch_one(pool)
    .await?;
    ensure!(
        exists,
        "independent tenant business root absent, expired, revoked, or not bound to exact V2 run"
    );
    Ok(())
}

/// Only Acquisition is enrolled in mandatory source attestation in this
/// version. Keep other historical one-node Start flows unchanged until each
/// respective owner has its own source-ingress issuer and physical acceptance.
/// Pinned owner routes have already passed exact trusted-catalog preflight.
fn requires_acquisition_source_attestation(run: &OperationDagRunV6) -> bool {
    run.plan.nodes.iter().any(|node| {
        run.pinned_routes
            .iter()
            .any(|route| route.operation == node.operation && route.owner.as_str() == "acquisition")
    })
}

/// Provenance is just a lookup selector; the bytes in the plan cannot mint
/// the row. The separate SQL principal, RLS, issuer-only INSERT/revocation,
/// exact immutable binding and live validity window provide its authority.
fn source_record_uuid(provenance: &str) -> Result<uuid::Uuid> {
    let supplied = provenance
        .strip_prefix("source-record-v1:")
        .context("Acquisition input lacks independently issued source-record identity")?;
    let record_id = uuid::Uuid::parse_str(supplied)
        .context("Acquisition source-record identity is not a UUID")?;
    ensure!(
        !record_id.is_nil() && record_id.to_string() == supplied,
        "Acquisition source-record identity must be one canonical nonnil UUID"
    );
    Ok(record_id)
}

/// A source row cannot be its own proof if it came from the same SQL login
/// performing Start. Keep this check independent of Acquisition's HTML parser:
/// that owner is exclusively responsible for faithful source transformation,
/// while the generic operator verifies source-record provenance and bytes.
fn checked_source_record_sql_metadata(
    issued_by: &str,
    reader_login: &str,
    source_page_id: i64,
    source_page_content_hash: &str,
    source_page_raw_sha256: &str,
) -> Result<()> {
    let canonical_digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    ensure!(
        !issued_by.is_empty()
            && !reader_login.is_empty()
            && issued_by != reader_login
            && source_page_id > 0
            && canonical_digest(source_page_content_hash)
            && canonical_digest(source_page_raw_sha256),
        "Acquisition source record lacks distinct SQL issuer or pinned raw snapshot identity"
    );
    Ok(())
}

async fn assert_active_independent_acquisition_sources(
    pool: &PgPool,
    tenant: &str,
    intent: &SingleOperationStartIntentV1,
) -> Result<usize> {
    if !requires_acquisition_source_attestation(&intent.run) {
        return Ok(0);
    }
    ensure!(
        !intent.run.admitted.is_empty()
            && intent.run.admitted.len() == intent.run.plan.external_artifacts.len(),
        "Acquisition owner requires a complete independently attested external source set"
    );
    for (artifact_id, artifact) in &intent.run.admitted {
        let record_id = source_record_uuid(&artifact.provenance)?;
        // No caller-supplied receipt is trusted. Only rows independently
        // loaded through the restricted tenant-bound SELECT-only PG LOGIN
        // are interpreted as trusted source records. Revocation/expiry is
        // checked by PG at the time of the query and again in typed Rust.
        let row = sqlx::query(
            "SELECT record_id::text AS record_id, tenant_id,context_key,
                    run_id::text AS run_id,plan_hash,artifact_id,artifact_schema,
                    payload_hash,
                    issued_by::text AS issued_by,
                    session_user::text AS reader_login,
                    source_page_id, source_page_content_hash, source_page_raw_sha256,
                    floor(extract(epoch from issued_at))::bigint AS issued_at_unix_seconds,
                    floor(extract(epoch from expires_at))::bigint AS expires_at_unix_seconds,
                    floor(extract(epoch from revoked_at))::bigint AS revoked_at_unix_seconds
             FROM pipeline.operation_dag_external_source_attestations_v1
             WHERE record_id=$1 AND run_id=$2 AND tenant_id=$3 AND context_key=$4
               AND plan_hash=$5 AND artifact_id=$6
               AND revoked_at IS NULL
               AND issued_at <= clock_timestamp()
               AND expires_at > clock_timestamp() + interval '90 seconds'",
        )
        .bind(record_id)
        .bind(uuid::Uuid::parse_str(&intent.run.plan.scope.run_id)?)
        .bind(tenant)
        .bind(&intent.context_key)
        .bind(intent.run.plan.hash.as_str())
        .bind(artifact_id.as_str())
        .fetch_optional(pool)
        .await?
        .context("independent Acquisition source record absent, expired or revoked")?;
        checked_source_record_sql_metadata(
            &row.try_get::<String, _>("issued_by")?,
            &row.try_get::<String, _>("reader_login")?,
            row.try_get("source_page_id")?,
            &row.try_get::<String, _>("source_page_content_hash")?,
            &row.try_get::<String, _>("source_page_raw_sha256")?,
        )?;
        let record = TrustedExternalSourceRecordV1 {
            schema_revision: EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1,
            record_id: row.try_get("record_id")?,
            tenant_id: row.try_get("tenant_id")?,
            context_key: row.try_get("context_key")?,
            run_id: row.try_get("run_id")?,
            plan_hash: PlanHash(row.try_get("plan_hash")?),
            artifact_id: ArtifactId(row.try_get("artifact_id")?),
            schema: row
                .try_get::<sqlx::types::Json<operation_protocol::ArtifactSchema>, _>(
                    "artifact_schema",
                )?
                .0,
            payload_hash: row.try_get("payload_hash")?,
            issued_at_unix_seconds: row.try_get("issued_at_unix_seconds")?,
            expires_at_unix_seconds: row.try_get("expires_at_unix_seconds")?,
            revoked_at_unix_seconds: row.try_get("revoked_at_unix_seconds")?,
        };
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system clock before Unix epoch")?
                .as_secs(),
        )
        .context("source attestation clock exceeds signed Unix seconds")?;
        verify_external_source_record_v1(
            &record,
            &intent.run,
            &intent.context_key,
            artifact_id,
            now,
        )
        .context("independent Acquisition source record does not match typed immutable input")?;
    }
    Ok(intent.run.admitted.len())
}

fn one_step_start_options(workflow_id: String) -> WorkflowStartOptions {
    WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, workflow_id)
        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
        .id_conflict_policy(WorkflowIdConflictPolicy::Fail)
        .build()
}

pub(super) async fn start_from_private_request(path: &Path) -> Result<()> {
    let mut stage = SingleStartStage::OperatorConfiguration;
    let mut run_id = None;
    let mut parent_workflow_id = None;
    match try_start_from_private_request(path, &mut stage, &mut run_id, &mut parent_workflow_id)
        .await
    {
        Ok(()) => Ok(()),
        Err(_) => {
            // Never echo private source bytes, provider commands, SQL errors,
            // credentials or an unverified caller-provided tenant identity.
            // In particular, an RPC failure after submission does NOT prove
            // that Temporal rejected the Start. Its deterministic Workflow ID
            // is the only safe place to reconcile the unknown outcome.
            println!(
                "{}",
                serde_json::to_string(&start_failure_report(
                    stage,
                    run_id.as_deref(),
                    parent_workflow_id.as_deref(),
                ))?
            );
            anyhow::bail!("single-step V2 operator refused or could not confirm Temporal Start; see redacted JSON diagnostic")
        }
    }
}

async fn try_start_from_private_request(
    path: &Path,
    stage: &mut SingleStartStage,
    run_id: &mut Option<String>,
    parent_workflow_id: &mut Option<String>,
) -> Result<()> {
    // The operator-only process does not load the worker's execution mode or
    // launch a DAG runner. It connects only to an explicitly configured local
    // Temporal endpoint, and its process identity must already be privileged.
    let config = checked_operator_config(|key| env::var(key).ok())?;
    *stage = SingleStartStage::PrivateIntent;
    let bytes = operator_dag_governance::read_private_operator_file(
        path,
        "single-step V2 immutable operator intent",
    )?;
    let catalog = operator_dag_governance::read_operator_owned_catalog(Path::new(&config.catalog))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before Unix epoch")?
        .as_secs();
    let intent = decode_checked_intent(&bytes, &config.tenant, &catalog, now)?;
    *run_id = Some(intent.run.plan.scope.run_id.clone());
    *stage = SingleStartStage::IndependentBusinessRoot;
    let pool = timeout(
        Duration::from_secs(15),
        operator_dag_read_v2::tenant_read_only_pool(&config.tenant, ROOT_READER_DSN_ENV, false),
    )
    .await
    .context("tenant-root reader connection timed out before Temporal Start")??;
    timeout(
        Duration::from_secs(15),
        assert_active_independent_business_root(&pool, &config.tenant, &intent),
    )
    .await
    .context("business-root proof timed out before Temporal Start")??;
    *stage = SingleStartStage::ExternalSourceAttestation;
    timeout(
        Duration::from_secs(15),
        assert_active_independent_acquisition_sources(&pool, &config.tenant, &intent),
    )
    .await
    .context("Acquisition source proof timed out before Temporal Start")??;
    let workflow_id = operation_dag_parent_workflow_id_v2(&intent.run.plan.scope.run_id);
    *parent_workflow_id = Some(workflow_id.clone());
    *stage = SingleStartStage::TemporalConnection;
    let connection = timeout(
        Duration::from_secs(15),
        Connection::connect(
            ConnectionOptions::new(config.url)
                .identity(format!("alegria-site-lab-single-v2-{}", std::process::id()))
                .build(),
        ),
    )
    .await
    .context("single-step Temporal connection timed out before Start")??;
    let client = Client::new(connection, ClientOptions::new(&config.namespace).build())
        .map_err(|error| anyhow::anyhow!("initialize single-step Temporal client: {error}"))?;
    *stage = SingleStartStage::FinalPreflight;
    // An endpoint connection may have stalled after the initial file check.
    // Do not dispatch an operator intent after its own finite validity window.
    let dispatch_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before Unix epoch")?
        .as_secs();
    checked_request_ttl(intent.expires_at_unix_seconds, dispatch_time)?;
    timeout(
        Duration::from_secs(15),
        assert_active_independent_business_root(&pool, &config.tenant, &intent),
    )
    .await
    .context("final business-root proof timed out before Temporal Start")??;
    // A source issuer may revoke a previously checked record while the
    // operator is connecting to Temporal. Recheck every Acquisition source
    // immediately before submission, using the independent SQL authority.
    *stage = SingleStartStage::ExternalSourceAttestation;
    timeout(
        Duration::from_secs(15),
        assert_active_independent_acquisition_sources(&pool, &config.tenant, &intent),
    )
    .await
    .context("final Acquisition source proof timed out before Temporal Start")??;
    *stage = SingleStartStage::FinalPreflight;
    // Root SELECT is an await point. The operator intent could expire while
    // PostgreSQL responds, even though it was valid when the query began.
    // Check its validity again at the *last* synchronous point before RPC.
    let final_dispatch_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before Unix epoch")?
        .as_secs();
    checked_request_ttl(intent.expires_at_unix_seconds, final_dispatch_time)?;
    *stage = SingleStartStage::StartSubmitted;
    let handle = timeout(
        Duration::from_secs(20),
        client.start_workflow(
            UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
            RawValue::from_value(&intent.run, &PayloadConverter::serde_json()),
            one_step_start_options(workflow_id.clone()),
        ),
    )
    .await
    .with_context(|| {
        format!(
            "Temporal Start outcome UNKNOWN for {workflow_id}; do not repeat; query the exact Workflow ID"
        )
    })?
    .context("Temporal returned an error for the single-step Start; verify its actual Workflow ID before any retry")?;
    let server_run_id = handle
        .info()
        .run_id
        .clone()
        .context("Temporal did not return a server-issued Workflow Run ID")?;
    println!(
        "{}",
        serde_json::json!({
            "operation":"site_lab_start_single_v2",
            "status":"started_business_root_verified_owner_unadmitted",
            "run_id":intent.run.plan.scope.run_id,
            "tenant_id":config.tenant,
            "parent_workflow_id":workflow_id,
            "parent_workflow_run_id":server_run_id,
            "plan_hash":intent.run.plan.hash,
            "source_authentication":if requires_acquisition_source_attestation(&intent.run) {
                "source_record_independently_issued_hash_verified_at_start"
            } else {
                "not_attested"
            },
            "source_transformation":if requires_acquisition_source_attestation(&intent.run) {
                "acquisition_issuer_responsible_not_recomputed_by_generic_operator"
            } else {
                "not_attested"
            },
            "business_root":"independently_issued_active_at_start",
            "owner_admission":"not_issued_by_start",
            "execution":"not_confirmed",
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use operation_protocol::{
        ContractHash, ExecutionPlan, OperationNode, OperationRef, PinnedOwnerRouteV1, RunScope,
        OPERATION_DAG_RUN_SCHEMA_REVISION_V6, PLAN_SCHEMA_REVISION,
    };

    fn one_step_candidate(owner: &str) -> OperationDagRunV6 {
        let operation = OperationRef {
            id: "acquisition.classify_source_page".into(),
            revision: 1,
        };
        OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan: ExecutionPlan {
                schema_revision: PLAN_SCHEMA_REVISION,
                scope: RunScope {
                    tenant_id: "tenant-ci".into(),
                    run_id: uuid::Uuid::new_v4().to_string(),
                },
                allowed_effects: BTreeSet::new(),
                external_artifacts: BTreeMap::new(),
                pinned_contracts: Vec::new(),
                nodes: vec![OperationNode {
                    instance: "one_step".into(),
                    operation: operation.clone(),
                    inputs: BTreeMap::new(),
                    outputs: BTreeMap::new(),
                    after: BTreeSet::new(),
                    permitted_effects: BTreeSet::new(),
                }],
                hash: PlanHash(String::new()),
            },
            admitted: BTreeMap::new(),
            owner_commands: BTreeMap::new(),
            pinned_routes: vec![PinnedOwnerRouteV1 {
                operation,
                owner: owner.into(),
                contract_hash: ContractHash(String::new()),
                workflow_type: String::new(),
                task_queue: String::new(),
                build_id: String::new(),
            }],
        }
    }

    #[test]
    fn acquisition_start_gate_is_owner_specific_and_never_trusts_self_described_provenance() {
        let mut run = one_step_candidate("acquisition");
        assert!(requires_acquisition_source_attestation(&run));
        run.pinned_routes[0].owner = "truth_domain".into();
        assert!(!requires_acquisition_source_attestation(&run));
        run.pinned_routes[0].owner = "acquisition".into();
        run.pinned_routes[0].operation.id = "acquisition.other".into();
        assert!(!requires_acquisition_source_attestation(&run));

        let issued_id = "0e2dddf2-0f45-46ed-89dd-3dce7e8b247d";
        assert_eq!(
            source_record_uuid(&format!("source-record-v1:{issued_id}"))
                .unwrap()
                .to_string(),
            issued_id
        );
        for forged in [
            "untrusted-provenance",
            "source-record-v1:",
            "source-record-v1:00000000-0000-0000-0000-000000000000",
            "source-record-v1:0E2DDDF2-0F45-46ED-89DD-3DCE7E8B247D",
            "source-record-v1:0e2dddf2-0f45-46ed-89dd-3dce7e8b247d trailing",
        ] {
            assert!(source_record_uuid(forged).is_err(), "{forged}");
        }
    }

    #[test]
    fn source_sql_evidence_requires_distinct_issuer_and_canonical_raw_identity() {
        let hash = "a".repeat(64);
        assert!(
            checked_source_record_sql_metadata("source_issuer", "start_reader", 14, &hash, &hash)
                .is_ok()
        );
        for (issuer, reader, page, digest) in [
            ("start_reader", "start_reader", 14, hash.as_str()),
            ("", "start_reader", 14, hash.as_str()),
            ("source_issuer", "", 14, hash.as_str()),
            ("source_issuer", "start_reader", 0, hash.as_str()),
            ("source_issuer", "start_reader", -4, hash.as_str()),
            (
                "source_issuer",
                "start_reader",
                14,
                "Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            ("source_issuer", "start_reader", 14, "a"),
        ] {
            assert!(
                checked_source_record_sql_metadata(issuer, reader, page, digest, &hash).is_err(),
                "must reject missing or self-issued source metadata"
            );
        }
        assert!(
            checked_source_record_sql_metadata(
                "source_issuer",
                "start_reader",
                14,
                &hash,
                "BAD_SHA256"
            )
            .is_err()
        );
    }

    #[test]
    fn single_step_start_requires_exact_operator_opt_in_and_tenant() {
        let none = checked_operator_config(|_| None);
        assert!(none.is_err());
        let fake = |key: &str| match key {
            ENABLE_ENV => Some("true".to_owned()),
            TENANT_ENV => Some("tenant-one".to_owned()),
            TEMPORAL_URL_ENV => Some("http://127.0.0.1:7233/".to_owned()),
            TEMPORAL_NAMESPACE_ENV => Some("test-namespace".to_owned()),
            CATALOG_ENV => Some("/etc/alegria/owner-routes-v1.json".to_owned()),
            _ => None,
        };
        let checked = checked_operator_config(fake).unwrap();
        assert_eq!(checked.tenant, "tenant-one");
        assert_eq!(checked.url.as_str(), "http://127.0.0.1:7233/");
        let invalid = checked_operator_config(|key| {
            if key == TENANT_ENV {
                Some(" another-tenant".to_owned())
            } else {
                fake(key)
            }
        });
        assert!(invalid.is_err());
    }

    #[test]
    fn single_step_start_never_accepts_public_or_ambiguous_temporal_endpoint() {
        for rejected in [
            "https://127.0.0.1:7233/",
            "http://example.com:7233/",
            "http://127.0.0.1:7233/extra",
            "http://127.0.0.1:7233/?x=1",
            "http://user@localhost:7233/",
            "http://localhost/",
            "http://127.0.0.1:7233/#x",
        ] {
            assert!(checked_local_temporal_url(rejected).is_err(), "{rejected}");
        }
        assert!(checked_local_temporal_url("http://localhost:7233/").is_ok());
    }

    #[test]
    fn single_step_start_rejects_stale_and_unbounded_intents() {
        assert!(checked_request_ttl(100, 100).is_err());
        assert!(checked_request_ttl(99, 100).is_err());
        assert!(checked_request_ttl(701, 100).is_err());
        assert!(checked_request_ttl(101, 100).is_ok());
        assert!(checked_request_ttl(700, 100).is_ok());
    }

    #[test]
    fn single_step_start_rejects_unbounded_or_control_bearing_scope_fields() {
        assert!(!bounded_policy_identifier(""));
        assert!(!bounded_policy_identifier("tenant\nother"));
        assert!(!bounded_policy_identifier(" tenant"));
        assert!(!bounded_policy_identifier(
            &"t".repeat(MAX_POLICY_ID_BYTES + 1)
        ));
        assert!(bounded_policy_identifier(&"t".repeat(MAX_POLICY_ID_BYTES)));
        assert!(!bounded_context_key("context\r\nforged"));
        assert!(!bounded_context_key(" context"));
        assert!(!bounded_context_key(&"x".repeat(MAX_CONTEXT_KEY_BYTES + 1)));
        assert!(bounded_context_key(&"x".repeat(MAX_CONTEXT_KEY_BYTES)));

        let configured = |key: &str| match key {
            ENABLE_ENV => Some("true".to_owned()),
            TENANT_ENV => Some("tenant\nforged".to_owned()),
            TEMPORAL_URL_ENV => Some("http://127.0.0.1:7233/".to_owned()),
            TEMPORAL_NAMESPACE_ENV => Some("namespace".to_owned()),
            CATALOG_ENV => Some("/etc/alegria/owner-routes-v1.json".to_owned()),
            _ => None,
        };
        assert!(checked_operator_config(configured).is_err());
    }

    #[test]
    fn single_step_start_diagnostics_preserve_precise_unknown_outcome_boundary() {
        for stage in [
            SingleStartStage::OperatorConfiguration,
            SingleStartStage::PrivateIntent,
            SingleStartStage::IndependentBusinessRoot,
            SingleStartStage::ExternalSourceAttestation,
            SingleStartStage::TemporalConnection,
            SingleStartStage::FinalPreflight,
        ] {
            let report = serde_json::to_value(start_failure_report(stage, None, None)).unwrap();
            assert_eq!(report["status"], "refused_before_temporal_start");
            assert_eq!(report["execution"], "not_confirmed");
            assert!(report["canonical_run_id"].is_null());
            assert!(report["parent_workflow_id"].is_null());
            assert_ne!(report["code"], "temporal_start_result_unknown");
        }
        let source_denied = serde_json::to_value(start_failure_report(
            SingleStartStage::ExternalSourceAttestation,
            None,
            None,
        ))
        .unwrap();
        assert_eq!(
            source_denied["code"],
            "independent_external_source_attestation_unavailable"
        );
        assert_eq!(source_denied["status"], "refused_before_temporal_start");
        assert!(!source_denied.to_string().contains("raw_text"));
        let report = serde_json::to_value(start_failure_report(
            SingleStartStage::StartSubmitted,
            Some("29b23f2a-f6ea-4571-b2dc-2630cdbf3aef"),
            Some("alegria-operation-dag-v2-29b23f2a-f6ea-4571-b2dc-2630cdbf3aef"),
        ))
        .unwrap();
        assert_eq!(report["status"], "start_outcome_unknown");
        assert_eq!(report["code"], "temporal_start_result_unknown");
        assert_eq!(report["owner_admission"], "not_issued_by_start");
        assert!(report["next_action"]
            .as_str()
            .unwrap()
            .contains("never retry a Start"));
        let serialized = report.to_string();
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("source_bytes"));
        assert!(!serialized.contains("command_bytes"));
    }

    #[test]
    fn single_step_start_refuses_relative_and_world_writable_request_paths() {
        for path in [
            "operator-intent.json",
            "/tmp/../x.json",
            "/tmp/absent-site-lab-intent.json",
        ] {
            assert!(operator_dag_governance::read_private_operator_file(
                Path::new(path),
                "single-step intent",
            )
            .is_err());
        }
    }

    #[test]
    fn single_step_start_pins_both_workflow_id_reuse_policies() {
        let options = one_step_start_options("alegria-operation-dag-v2-test".to_owned());
        assert_eq!(
            options.id_reuse_policy,
            WorkflowIdReusePolicy::RejectDuplicate
        );
        assert_eq!(options.id_conflict_policy, WorkflowIdConflictPolicy::Fail);
    }

    #[cfg(unix)]
    #[test]
    fn single_step_start_requires_private_regular_file_and_private_ancestry() {
        use std::fs;
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = env::current_dir()
            .unwrap()
            .join(format!(".ci-single-v2-intent-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let private_file = root.join("intent.json");
        fs::write(&private_file, b"test-only-private-bytes").unwrap();
        fs::set_permissions(&private_file, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            operator_dag_governance::read_private_operator_file(
                &private_file,
                "single-step intent",
            )
            .unwrap(),
            b"test-only-private-bytes"
        );

        fs::set_permissions(&private_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            operator_dag_governance::read_private_operator_file(&private_file, "intent").is_err()
        );
        fs::set_permissions(&private_file, fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.join("linked-intent.json");
        symlink(&private_file, &link).unwrap();
        assert!(operator_dag_governance::read_private_operator_file(&link, "intent").is_err());

        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            operator_dag_governance::read_private_operator_file(&private_file, "intent").is_err()
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(&link).unwrap();
        fs::remove_file(&private_file).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
