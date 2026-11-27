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
    OperationDagRunV6, SiteLabExpectedStartV1, TrustedRouteCatalog, OPERATION_DAG_PARENT_QUEUE_V2,
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

use sqlx::PgPool;

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
            "source_authentication":"not_attested",
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
