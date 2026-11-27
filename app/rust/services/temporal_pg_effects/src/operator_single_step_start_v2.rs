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
    decode_admission_signing_key_v1, operation_dag_parent_workflow_id_v2,
    preflight_issued_one_step_request_for_reservation_v1,
    preflight_unadmitted_single_operation_v2_start, verify_external_source_record_v1,
    verify_signed_one_step_admission_v1, ArtifactId, IndependentlyAuthenticatedOneStepPolicyV1,
    OneStepRequestIssuanceStateV1, OneStepResourceLimitsV1, OpaqueOneStepRequestLocatorV1,
    OperationDagRunV6, OperationId, OperationRef, PlanHash, SiteLabExpectedStartV1,
    TrustedExternalSourceRecordV1, TrustedOneStepRequestGrantV1, TrustedRouteCatalog,
    DAG_MAX_INPUT_BYTES, EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1,
    OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
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

use super::operator_v4::{fetch_exact_history, first_started_payload};
use super::{operator_dag_governance, operator_dag_read_v2};

const ENABLE_ENV: &str = "ALEGRIA_ENABLE_SITE_LAB_SINGLE_V2_START";
const TENANT_ENV: &str = "SITE_LAB_SINGLE_V2_TENANT_ID";
const TEMPORAL_URL_ENV: &str = "SITE_LAB_SINGLE_V2_TEMPORAL_URL";
const TEMPORAL_NAMESPACE_ENV: &str = "SITE_LAB_SINGLE_V2_TEMPORAL_NAMESPACE";
const CATALOG_ENV: &str = "SITE_LAB_SINGLE_V2_TRUSTED_CATALOG_FILE";
const ROOT_READER_DSN_ENV: &str = "SITE_LAB_SINGLE_V2_DATABASE_URL_FILE";
const GRANTED_ENABLE_ENV: &str = "ALEGRIA_ENABLE_SITE_ARCHITECT_GRANTED_V1_START";
const GRANTED_DSN_FILE_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_DATABASE_URL_FILE";
const GRANTED_PRINCIPAL_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_PRINCIPAL_ID";
const GRANTED_POLICY_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_POLICY_ID";
const GRANTED_CONTEXT_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_CONTEXT_KEY";
const GRANTED_LIMITS_FILE_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_MAX_LIMITS_FILE";
const GRANTED_STATUS_ENABLE_ENV: &str = "ALEGRIA_ENABLE_SITE_ARCHITECT_GRANTED_V1_STATUS";
const GRANTED_STATUS_DSN_FILE_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_STATUS_DATABASE_URL_FILE";
const GRANTED_RESERVER_LOGIN_ENV: &str = "SITE_ARCHITECT_GRANTED_V1_RESERVER_LOGIN";
const GRANTED_STATUS_READER_DSN_ENV: &str = "OPERATION_DAG_READ_V2_DATABASE_URL_FILE";
const GRANTED_STATUS_TEMPORAL_URL_ENV: &str = "OPERATION_DAG_READ_V2_TEMPORAL_URL";
const GRANTED_STATUS_NAMESPACE_ENV: &str = "OPERATION_DAG_READ_V2_TEMPORAL_NAMESPACE";
const GRANTED_STATUS_CATALOG_ENV: &str = "OPERATION_DAG_READ_V2_TRUSTED_CATALOG_FILE";
const SIGNED_ADMISSION_KEY_FILE_ENV: &str = "SITE_ARCHITECT_SIGNED_ADMISSION_V1_KEY_FILE";
const SIGNED_ADMISSION_ISSUER_ENV: &str = "SITE_ARCHITECT_SIGNED_ADMISSION_V1_ISSUER";
const SIGNED_ADMISSION_RESOURCE_ENV: &str = "SITE_ARCHITECT_SIGNED_ADMISSION_V1_RESOURCE";
const SIGNED_ADMISSION_SUBJECT_ENV: &str = "SITE_ARCHITECT_SIGNED_ADMISSION_V1_SUBJECT";
const MAX_REQUEST_FUTURE_SECONDS: u64 = 600;
const MAX_CONTEXT_KEY_BYTES: usize = 512;
const MAX_POLICY_ID_BYTES: usize = 128;

/// A Temporal Start acknowledgement may be lost after the Workflow was
/// persisted; a one-shot SQL reservation acknowledgement may also be lost
/// after its commit, but cannot send the Temporal command on that branch.
/// These states are diagnostic only, never a client-controlled admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleStartStage {
    OperatorConfiguration,
    PrivateIntent,
    IndependentBusinessRoot,
    ExternalSourceAttestation,
    TemporalConnection,
    FinalPreflight,
    IndependentRequestGrant,
    GrantReservationUncertain,
    GrantUnknownTransitionUncertain,
    GrantUnknownCommittedStartNotSent,
    StartSubmitted,
}

impl SingleStartStage {
    fn status(self) -> &'static str {
        match self {
            Self::StartSubmitted => "start_outcome_unknown",
            Self::GrantReservationUncertain | Self::GrantUnknownTransitionUncertain => {
                "grant_state_uncertain_start_not_sent"
            }
            Self::GrantUnknownCommittedStartNotSent => "grant_consumed_start_not_sent",
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
            Self::IndependentRequestGrant => "independent_request_grant_unavailable",
            Self::GrantReservationUncertain => "grant_reservation_unconfirmed",
            Self::GrantUnknownTransitionUncertain => "grant_unknown_state_unconfirmed",
            Self::GrantUnknownCommittedStartNotSent => "grant_consumed_before_start_expired",
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
            Self::IndependentRequestGrant => {
                "Check separately issued request grant, authenticated principal, context ACL, budget and one-use state."
            }
            Self::GrantReservationUncertain | Self::GrantUnknownTransitionUncertain => {
                "Do not retry Start: inspect the exact SQL grant state, reservation nonce and Workflow ID under independent operator authority. No Temporal Start RPC was sent by this invocation."
            }
            Self::GrantUnknownCommittedStartNotSent => {
                "A one-use SQL grant was durably consumed but no Temporal Start was sent; reconcile the grant and deterministic Workflow ID manually. Never retry this request."
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

/// The request ID is a SQL locator, never authority. All runnable bytes are
/// still private operator-controlled input, and all principal/budget policy is
/// configured independently of this file and its contents.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GrantedSingleOperationStartIntentV1 {
    schema_revision: u16,
    request_id: String,
    intent: SingleOperationStartIntentV1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartMode {
    PrivateOperator,
    IndependentlyGranted,
}

struct CheckedGrantPolicy {
    request_id: uuid::Uuid,
    principal_id: String,
    policy_id: String,
    context_key: String,
    maximum_limits: OneStepResourceLimitsV1,
}

fn checked_grant_config(
    read: impl Fn(&str) -> Option<String>,
    limits_bytes: &[u8],
) -> Result<(String, String, String, OneStepResourceLimitsV1)> {
    ensure!(
        read(GRANTED_ENABLE_ENV).as_deref() == Some("true"),
        "one-shot grant Start is not enabled by independent operator"
    );
    let principal = read(GRANTED_PRINCIPAL_ENV)
        .context("missing independently configured operator principal")?;
    let policy =
        read(GRANTED_POLICY_ENV).context("missing independently configured principal policy")?;
    let context =
        read(GRANTED_CONTEXT_ENV).context("missing independently configured principal context")?;
    ensure!(
        bounded_policy_identifier(&principal)
            && bounded_policy_identifier(&policy)
            && bounded_context_key(&context),
        "invalid independently configured principal, policy or context"
    );
    let limits: OneStepResourceLimitsV1 = serde_json::from_slice(limits_bytes)
        .context("invalid private independently authorized maximum budget")?;
    ensure!(
        serde_json::to_vec(&limits)?.as_slice() == limits_bytes,
        "authorized maximum budget must be canonical typed JSON"
    );
    Ok((principal, policy, context, limits))
}

fn decode_granted_intent(
    bytes: &[u8],
    tenant: &str,
    catalog: &TrustedRouteCatalog,
    now: u64,
) -> Result<(SingleOperationStartIntentV1, uuid::Uuid)> {
    let wrapper: GrantedSingleOperationStartIntentV1 = serde_json::from_slice(bytes)
        .context("decode one-shot privately approved request locator and intent")?;
    ensure!(
        wrapper.schema_revision == 1 && serde_json::to_vec(&wrapper)?.as_slice() == bytes,
        "noncanonical or unsupported one-shot request envelope"
    );
    let id = uuid::Uuid::parse_str(&wrapper.request_id)?;
    ensure!(
        !id.is_nil() && id.to_string() == wrapper.request_id,
        "one-shot request ID must be canonical nonnil UUID"
    );
    let intent_bytes = serde_json::to_vec(&wrapper.intent)?;
    let checked = decode_checked_intent(&intent_bytes, tenant, catalog, now)?;
    Ok((checked, id))
}

async fn checked_one_shot_grant_pool(expected_tenant: &str) -> Result<PgPool> {
    let path = env::var(GRANTED_DSN_FILE_ENV)
        .context("one-shot Start requires separately provisioned SQL grant-reserver LOGIN")?;
    let bytes = operator_dag_governance::read_private_operator_file(
        Path::new(&path),
        "one-shot SQL grant reserver DSN",
    )?;
    let url = String::from_utf8(bytes)?;
    let url = url.trim_end_matches(['\r', '\n']);
    ensure!(
        !url.is_empty() && url.trim() == url,
        "invalid grant reserver DSN"
    );
    let pool = PgPool::connect(url).await?;
    let role = sqlx::query(
        "SELECT current_user::text effective,session_user::text login,
                r.rolcanlogin login_enabled,r.rolinherit inherits_roles,
                (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication) elevated,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_one_step_request_grants_v1','SELECT') grant_reader,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_one_step_request_grants_v1','INSERT') grant_issuer,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_business_runs_v1','INSERT') root_issuer
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("elevated")
            && role.get::<bool, _>("grant_reader")
            && !role.get::<bool, _>("grant_issuer")
            && !role.get::<bool, _>("root_issuer"),
        "one-shot SQL reserver must be restricted distinct authenticated LOGIN"
    );
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT parent.rolname::text FROM pg_catalog.pg_auth_members m
         JOIN pg_catalog.pg_roles parent ON parent.oid=m.roleid
         WHERE m.member=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user)
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        membership.is_none(),
        "one-shot reserver cannot SET ROLE or inherit privileges"
    );
    let foreign_write: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname||'.'||c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname NOT IN ('information_schema') AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (has_any_column_privilege(session_user,c.oid,'INSERT')
                OR has_any_column_privilege(session_user,c.oid,'UPDATE')
                OR has_table_privilege(session_user,c.oid,'DELETE')
                OR has_table_privilege(session_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        foreign_write.is_none(),
        "one-shot reserver has forbidden direct app writes"
    );
    let actual_tenant: Option<String> =
        sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
            .fetch_one(&pool)
            .await?;
    ensure!(
        actual_tenant.as_deref() == Some(expected_tenant),
        "one-shot SQL reserver LOGIN tenant binding mismatch"
    );
    Ok(pool)
}

/// A status reader is NOT the Start reserver. Even a SELECT-only Start login
/// can execute SECURITY DEFINER reserve/unknown functions, so reusing its
/// credentials here would silently give an alleged read surface Start power.
async fn checked_one_shot_status_pool(expected_tenant: &str) -> Result<PgPool> {
    let path = env::var(GRANTED_STATUS_DSN_FILE_ENV)
        .context("one-shot status requires a dedicated SELECT-only SQL LOGIN credential file")?;
    let bytes = operator_dag_governance::read_private_operator_file(
        Path::new(&path),
        "one-shot status SELECT-only SQL DSN",
    )?;
    let dsn = String::from_utf8(bytes)?;
    let dsn = dsn.trim_end_matches(['\r', '\n']);
    ensure!(
        !dsn.is_empty() && dsn.trim() == dsn,
        "invalid dedicated one-shot status SQL DSN"
    );
    let pool = PgPool::connect(dsn).await?;
    let role = sqlx::query(
        "SELECT current_user::text AS effective,session_user::text AS login,
                r.rolcanlogin login_enabled,r.rolinherit inherits_roles,
                (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication) elevated,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_one_step_request_grants_v1','SELECT') can_read_grant,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_business_runs_v1','SELECT') can_read_root,
                has_table_privilege(session_user,
                 'pipeline.operation_dag_run_governance_v2','SELECT') can_read_governance,
                has_database_privilege(session_user,current_database(),'CREATE') can_create_db,
                has_function_privilege(session_user,
                 'pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid)',
                 'EXECUTE') can_reserve,
                has_function_privilege(session_user,
                 'pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid)',
                 'EXECUTE') can_mark_unknown,
                has_function_privilege(session_user,
                 'pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text)',
                 'EXECUTE') can_confirm
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("elevated")
            && role.get::<bool, _>("can_read_grant")
            && !role.get::<bool, _>("can_read_root")
            && !role.get::<bool, _>("can_read_governance")
            && !role.get::<bool, _>("can_create_db")
            && !role.get::<bool, _>("can_reserve")
            && !role.get::<bool, _>("can_mark_unknown")
            && !role.get::<bool, _>("can_confirm"),
        "one-shot status login has forbidden Start or write authority"
    );
    let role_membership: Option<String> = sqlx::query_scalar(
        "SELECT parent.rolname::text FROM pg_catalog.pg_auth_members m
         JOIN pg_catalog.pg_roles parent ON parent.oid=m.roleid
         WHERE m.member=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user)
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        role_membership.is_none(),
        "one-shot status login must not inherit or SET ROLE"
    );
    let direct_write: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname||'.'||c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname NOT IN ('information_schema') AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (has_any_column_privilege(session_user,c.oid,'INSERT')
                OR has_any_column_privilege(session_user,c.oid,'UPDATE')
                OR has_table_privilege(session_user,c.oid,'DELETE')
                OR has_table_privilege(session_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        direct_write.is_none(),
        "one-shot status login has forbidden direct table writes"
    );
    let create_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname::text FROM pg_catalog.pg_namespace n
         WHERE n.nspname NOT IN ('information_schema') AND n.nspname !~ '^pg_'
           AND has_schema_privilege(session_user,n.oid,'CREATE')
         ORDER BY n.nspname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        create_schema.is_none(),
        "one-shot status login must not create app SQL objects"
    );
    let bound_tenant: Option<String> =
        sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
            .fetch_one(&pool)
            .await?;
    ensure!(
        bound_tenant.as_deref() == Some(expected_tenant),
        "one-shot status SQL LOGIN is not independently bound to this tenant"
    );
    Ok(pool)
}

async fn independently_check_one_shot_grant(
    pool: &PgPool,
    policy: &CheckedGrantPolicy,
    tenant: &str,
    intent: &SingleOperationStartIntentV1,
    catalog: &TrustedRouteCatalog,
    now_unix_seconds: i64,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT request_id::text AS request_id,authorization_event_id::text AS authorization_event_id,
                run_id::text AS run_id,tenant_id,principal_id,policy_id,context_key,
                operation_id,operation_revision,parent_workflow_id,plan_hash,
                expected_start,limits,state,issued_by::text AS issuer,
                session_user::text AS reserver,
                floor(extract(epoch from issued_at))::bigint AS issued_unix,
                floor(extract(epoch from expires_at))::bigint AS expires_unix,
                floor(extract(epoch from revoked_at))::bigint AS revoked_unix
         FROM pipeline.operation_dag_one_step_request_grants_v1
         WHERE request_id=$1 AND tenant_id=$2 AND run_id=$3",
    )
    .bind(policy.request_id)
    .bind(tenant)
    .bind(uuid::Uuid::parse_str(&intent.run.plan.scope.run_id)?)
    .fetch_optional(pool)
    .await?
    .context("separately issued one-shot grant does not exist for authenticated tenant/run")?;
    ensure!(
        row.try_get::<String, _>("issuer")? != row.try_get::<String, _>("reserver")?,
        "issuer and one-shot reserver must be independent SQL principals"
    );
    let state = match row.try_get::<String, _>("state")?.as_str() {
        "unclaimed" => OneStepRequestIssuanceStateV1::Unclaimed,
        "reserved" => OneStepRequestIssuanceStateV1::Reserved,
        "start_submitted" => OneStepRequestIssuanceStateV1::StartSubmitted,
        "start_outcome_unknown" => OneStepRequestIssuanceStateV1::StartOutcomeUnknown,
        "start_confirmed" => OneStepRequestIssuanceStateV1::StartConfirmed,
        _ => anyhow::bail!("one-shot SQL grant state is not a known native revision"),
    };
    let grant = TrustedOneStepRequestGrantV1 {
        schema_revision: 1,
        request_id: row.try_get("request_id")?,
        authorization_event_id: row.try_get("authorization_event_id")?,
        tenant_id: row.try_get("tenant_id")?,
        authenticated_principal_id: row.try_get("principal_id")?,
        authorization_policy_id: row.try_get("policy_id")?,
        context_key: row.try_get("context_key")?,
        canonical_run_id: row.try_get("run_id")?,
        operation: OperationRef {
            id: OperationId(row.try_get::<String, _>("operation_id")?),
            revision: u32::try_from(row.try_get::<i32, _>("operation_revision")?)?,
        },
        parent_workflow_id: row.try_get("parent_workflow_id")?,
        expected: row
            .try_get::<sqlx::types::Json<SiteLabExpectedStartV1>, _>("expected_start")?
            .0,
        limits: row
            .try_get::<sqlx::types::Json<OneStepResourceLimitsV1>, _>("limits")?
            .0,
        issued_at_unix_seconds: row.try_get("issued_unix")?,
        expires_at_unix_seconds: row.try_get("expires_unix")?,
        revoked_at_unix_seconds: row.try_get("revoked_unix")?,
        state,
    };
    ensure!(
        intent.expected == grant.expected && intent.context_key == policy.context_key,
        "sealed private intent differs from independent user-request grant"
    );
    let authenticated = IndependentlyAuthenticatedOneStepPolicyV1 {
        tenant_id: tenant.to_owned(),
        principal_id: policy.principal_id.clone(),
        policy_id: policy.policy_id.clone(),
        context_key: policy.context_key.clone(),
        maximum_limits: policy.maximum_limits.clone(),
    };
    preflight_issued_one_step_request_for_reservation_v1(
        &OpaqueOneStepRequestLocatorV1 {
            request_id: policy.request_id.to_string(),
        },
        &grant,
        &intent.run,
        catalog,
        &authenticated,
        now_unix_seconds,
    )
    .context("one-shot request authorization, immutable inputs or budget not admitted")?;
    Ok(())
}

async fn sql_reserve_one_shot(
    pool: &PgPool,
    policy: &CheckedGrantPolicy,
    tenant: &str,
    intent: &SingleOperationStartIntentV1,
    nonce: uuid::Uuid,
) -> Result<()> {
    let accepted: bool = sqlx::query_scalar(
        "SELECT pipeline.reserve_operation_dag_one_step_request_v1($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(policy.request_id)
    .bind(uuid::Uuid::parse_str(&intent.run.plan.scope.run_id)?)
    .bind(tenant)
    .bind(&policy.principal_id)
    .bind(&policy.policy_id)
    .bind(&policy.context_key)
    .bind(intent.run.plan.hash.as_str())
    .bind(nonce)
    .fetch_one(pool)
    .await?;
    ensure!(
        accepted,
        "atomic one-shot reservation refused or already claimed"
    );
    Ok(())
}

async fn sql_commit_unknown_before_temporal_start(
    pool: &PgPool,
    request_id: uuid::Uuid,
    nonce: uuid::Uuid,
) -> Result<()> {
    let accepted: bool =
        sqlx::query_scalar("SELECT pipeline.mark_operation_dag_one_step_start_unknown_v1($1,$2)")
            .bind(request_id)
            .bind(nonce)
            .fetch_one(pool)
            .await?;
    ensure!(accepted, "one-shot grant unknown-state write refused");
    Ok(())
}

/// A request UUID selects one SQL row. It does not grant authority to read
/// another principal's request or supply a Temporal run/History assertion.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GrantedStartStatusLocatorV1 {
    schema_revision: u16,
    request_id: uuid::Uuid,
}

fn decode_granted_start_status_locator(bytes: &[u8]) -> Result<GrantedStartStatusLocatorV1> {
    ensure!(bytes.len() <= 128, "one-shot status locator is unbounded");
    let locator: GrantedStartStatusLocatorV1 = serde_json::from_slice(bytes)?;
    ensure!(
        locator.schema_revision == 1
            && !locator.request_id.is_nil()
            && serde_json::to_vec(&locator)?.as_slice() == bytes,
        "one-shot status locator must be one canonical nonnil UUID-only request"
    );
    Ok(locator)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistedGrantedStartState {
    Unclaimed,
    Reserved,
    StartOutcomeUnknown,
    StartConfirmed,
}

impl PersistedGrantedStartState {
    fn from_sql(state: &str) -> Result<Self> {
        match state {
            "unclaimed" => Ok(Self::Unclaimed),
            "reserved" => Ok(Self::Reserved),
            "start_outcome_unknown" => Ok(Self::StartOutcomeUnknown),
            "start_confirmed" => Ok(Self::StartConfirmed),
            _ => anyhow::bail!("unknown or unsupported SQL one-shot grant state"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Reserved => "reserved",
            Self::StartOutcomeUnknown => "start_outcome_unknown",
            Self::StartConfirmed => "start_confirmed",
        }
    }

    fn requires_history_inspection(self) -> bool {
        matches!(self, Self::StartOutcomeUnknown | Self::StartConfirmed)
    }

    fn status(self, exact_server_history: bool) -> &'static str {
        match (self, exact_server_history) {
            (Self::Unclaimed, _) => "grant_unclaimed",
            (Self::Reserved, _) => "grant_reserved_outcome_unverified",
            (Self::StartOutcomeUnknown, false) => "start_outcome_unknown",
            (Self::StartOutcomeUnknown, true) => {
                "temporal_start_history_verified_grant_pending_confirmation"
            }
            (Self::StartConfirmed, false) => "sql_grant_confirmed_history_unverified",
            (Self::StartConfirmed, true) => "temporal_start_history_verified_grant_confirmed",
        }
    }

    fn next_action(self, exact_server_history: bool) -> &'static str {
        match (self, exact_server_history) {
            (Self::Unclaimed, _) => {
                "Request a separately authorized Start only while the SQL grant is active; this status read does not reserve or dispatch."
            }
            (Self::Reserved, _) => {
                "Reservation outcome requires independent SQL reconciliation. Do not retry or dispatch Temporal Start using this consumed grant."
            }
            (Self::StartOutcomeUnknown, false) => {
                "An independent governance issuer must attest exact server Temporal History, or an operator must investigate its absence. Never repeat Start for this grant."
            }
            (Self::StartOutcomeUnknown, true) => {
                "Exact Temporal Start History is independently attested. A separately authorized SQL reconciler may confirm the one-use grant; this read did not change it."
            }
            (Self::StartConfirmed, false) => {
                "SQL records a confirmed Start, but this read could not verify exact server History. Reconcile credentials, governance and Temporal independently; never replay."
            }
            (Self::StartConfirmed, true) => {
                "Start identity is independently verified. Inspect owner History and separately attested durable provider receipt before claiming any execution result."
            }
        }
    }
}

fn status_grant_actors_are_independent(
    state: PersistedGrantedStartState,
    issuer: &str,
    status_reader: &str,
    configured_reserver: &str,
    recorded_reserver: Option<&str>,
    reservation_nonce: Option<uuid::Uuid>,
) -> bool {
    !issuer.is_empty()
        && !status_reader.is_empty()
        && !configured_reserver.is_empty()
        && issuer != status_reader
        && issuer != configured_reserver
        && status_reader != configured_reserver
        && match state {
            PersistedGrantedStartState::Unclaimed => {
                recorded_reserver.is_none() && reservation_nonce.is_none()
            }
            PersistedGrantedStartState::Reserved
            | PersistedGrantedStartState::StartOutcomeUnknown
            | PersistedGrantedStartState::StartConfirmed => {
                recorded_reserver == Some(configured_reserver)
                    && reservation_nonce.is_some_and(|nonce| !nonce.is_nil())
            }
        }
}

#[derive(Serialize)]
struct SafeGrantedStartStatusV1 {
    schema_revision: u16,
    operation: &'static str,
    status: &'static str,
    request_id: uuid::Uuid,
    canonical_run_id: uuid::Uuid,
    parent_workflow_id: String,
    grant_state: &'static str,
    authorization_currently_active: bool,
    temporal_start_history_attestation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_workflow_run_id: Option<String>,
    execution: &'static str,
    owner_admission: &'static str,
    next_action: &'static str,
}

#[derive(Serialize)]
struct SafeGrantedStatusRefusalV1 {
    schema_revision: u16,
    operation: &'static str,
    status: &'static str,
    code: &'static str,
    execution: &'static str,
    next_action: &'static str,
}

/// An opt-in PRIVATE local read, independent of Start opt-in. It does not
/// reserve, mutate or retry a request. Unavailable Temporal/governance proof
/// is an explicitly unverified status, never inferred success from the ACK.
pub(super) async fn read_granted_single_status_from_private_locator(path: &Path) -> Result<()> {
    match try_read_granted_single_status_from_private_locator(path).await {
        Ok(report) => {
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        Err(_) => {
            println!(
                "{}",
                serde_json::to_string(&SafeGrantedStatusRefusalV1 {
                    schema_revision: 1,
                    operation: "site_architect_read_granted_single_v1",
                    status: "refused",
                    code: "grant_unavailable_or_untrusted",
                    execution: "not_confirmed",
                    next_action: "Ask the operator to verify the private locator, separately configured principal, tenant-bound SQL grant reader and governance credentials.",
                })?
            );
            anyhow::bail!("one-shot status read refused; see redacted JSON diagnostic")
        }
    }
}

async fn try_read_granted_single_status_from_private_locator(
    path: &Path,
) -> Result<SafeGrantedStartStatusV1> {
    ensure!(
        env::var(GRANTED_STATUS_ENABLE_ENV).ok().as_deref() == Some("true"),
        "one-shot status read is not independently enabled"
    );
    let tenant = env::var(TENANT_ENV)?;
    let principal = env::var(GRANTED_PRINCIPAL_ENV)?;
    let policy = env::var(GRANTED_POLICY_ENV)?;
    let context = env::var(GRANTED_CONTEXT_ENV)?;
    let trusted_reserver = env::var(GRANTED_RESERVER_LOGIN_ENV)?;
    ensure!(
        bounded_policy_identifier(&tenant)
            && bounded_policy_identifier(&principal)
            && bounded_policy_identifier(&policy)
            && bounded_context_key(&context)
            && bounded_policy_identifier(&trusted_reserver),
        "independent one-shot status tenant/principal/reserver policy is invalid"
    );
    let bytes = operator_dag_governance::read_private_operator_file(
        path,
        "private one-shot status request locator",
    )?;
    let locator = decode_granted_start_status_locator(&bytes)?;
    let pool = timeout(
        Duration::from_secs(15),
        checked_one_shot_status_pool(&tenant),
    )
    .await
    .context("one-shot status SQL reader timed out")??;
    // The independently bound SQL LOGIN, not the opaque locator, defines the
    // tenant. Explicit principal/policy/context filters prevent a shared
    // tenant from enumerating another user's request via a UUID.
    let row = timeout(
        Duration::from_secs(15),
        sqlx::query(
            "SELECT run_id,tenant_id,context_key,parent_workflow_id,plan_hash,state,
                    issued_by::text AS issuer,session_user::text AS reader,
                    reserved_by::text AS reserver,reservation_nonce,
                    (revoked_at IS NULL AND expires_at > clock_timestamp()) AS authorization_active
             FROM pipeline.operation_dag_one_step_request_grants_v1
             WHERE request_id=$1 AND tenant_id=$2 AND principal_id=$3
               AND policy_id=$4 AND context_key=$5",
        )
        .bind(locator.request_id)
        .bind(&tenant)
        .bind(&principal)
        .bind(&policy)
        .bind(&context)
        .fetch_optional(&pool),
    )
    .await
    .context("one-shot status grant read timed out")??
    .context("one-shot grant unavailable under independently bound principal")?;
    let state = PersistedGrantedStartState::from_sql(row.try_get("state")?)?;
    let issuer = row.try_get::<String, _>("issuer")?;
    let reader = row.try_get::<String, _>("reader")?;
    let reserved_by = row.try_get::<Option<String>, _>("reserver")?;
    let nonce = row.try_get::<Option<uuid::Uuid>, _>("reservation_nonce")?;
    ensure!(
        status_grant_actors_are_independent(
            state,
            &issuer,
            &reader,
            &trusted_reserver,
            reserved_by.as_deref(),
            nonce,
        ),
        "one-shot grant issuer/reserver or persistence state is untrusted"
    );
    let canonical_run_id: uuid::Uuid = row.try_get("run_id")?;
    let workflow_id: String = row.try_get("parent_workflow_id")?;
    let plan_hash: String = row.try_get("plan_hash")?;
    ensure!(
        !canonical_run_id.is_nil()
            && workflow_id == operation_dag_parent_workflow_id_v2(&canonical_run_id.to_string())
            && plan_hash.len() == 64
            && plan_hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && row.try_get::<String, _>("tenant_id")? == tenant
            && row.try_get::<String, _>("context_key")? == context,
        "one-shot SQL scope/parent identity differs from native workflow"
    );
    let attested_server_run = if state.requires_history_inspection() {
        // A separate SELECT-only tenant login reads independently issued
        // governance. Any credential, History or hash mismatch produces NO
        // positive attestation; the grant remains SQL-canonical and consumed.
        timeout(
            Duration::from_secs(30),
            independently_attest_granted_parent_start_history(
                &tenant,
                &context,
                canonical_run_id,
                &workflow_id,
                &plan_hash,
                &reader,
                &trusted_reserver,
            ),
        )
        .await
        .ok()
        .and_then(|result| result.ok())
        .flatten()
    } else {
        None
    };
    let exact_server_history = attested_server_run.is_some();
    Ok(SafeGrantedStartStatusV1 {
        schema_revision: 1,
        operation: "site_architect_read_granted_single_v1",
        status: state.status(exact_server_history),
        request_id: locator.request_id,
        canonical_run_id,
        parent_workflow_id: workflow_id,
        grant_state: state.as_str(),
        authorization_currently_active: row.try_get("authorization_active")?,
        temporal_start_history_attestation: if exact_server_history {
            "exact_server_history_matches_independent_sql_governance"
        } else {
            "unavailable_or_unverified"
        },
        parent_workflow_run_id: attested_server_run,
        execution: "not_confirmed",
        owner_admission: "not_issued_by_status",
        next_action: state.next_action(exact_server_history),
    })
}

async fn independently_attest_granted_parent_start_history(
    tenant: &str,
    context: &str,
    canonical_run_id: uuid::Uuid,
    workflow_id: &str,
    plan_hash: &str,
    status_reader_login: &str,
    trusted_reserver_login: &str,
) -> Result<Option<String>> {
    let pool =
        operator_dag_read_v2::tenant_read_only_pool(tenant, GRANTED_STATUS_READER_DSN_ENV, true)
            .await?;
    let governance_reader_login: String = sqlx::query_scalar("SELECT session_user::text")
        .fetch_one(&pool)
        .await?;
    ensure!(
        governance_reader_login != status_reader_login
            && governance_reader_login != trusted_reserver_login,
        "independent governance proof reader cannot reuse status or Start SQL LOGIN"
    );
    let root = sqlx::query(
        "SELECT tenant_id,context_key,plan_hash,parent_workflow_id
         FROM pipeline.operation_dag_business_runs_v1 WHERE run_id=$1",
    )
    .bind(canonical_run_id)
    .fetch_optional(&pool)
    .await?
    .context("independently issued business root unavailable")?;
    ensure!(
        root.try_get::<String, _>("tenant_id")? == tenant
            && root.try_get::<String, _>("context_key")? == context
            && root.try_get::<String, _>("plan_hash")? == plan_hash
            && root.try_get::<String, _>("parent_workflow_id")? == workflow_id,
        "independent business root does not match one-shot grant"
    );
    let governance = sqlx::query(
        "SELECT parent_workflow_run_id,parent_start_sha256,tenant_id,context_key,
                plan_hash,parent_workflow_id,parent_workflow_type,parent_task_queue,
                issued_by::text AS issuer
         FROM pipeline.operation_dag_run_governance_v2
         WHERE canonical_run_id=$1 ORDER BY issued_at DESC LIMIT 2",
    )
    .bind(canonical_run_id)
    .fetch_all(&pool)
    .await?;
    if governance.is_empty() {
        return Ok(None);
    }
    ensure!(
        governance.len() == 1,
        "ambiguous independent Temporal governance"
    );
    let attestation = &governance[0];
    let server_run_id = attestation.try_get::<String, _>("parent_workflow_run_id")?;
    let server_uuid = uuid::Uuid::parse_str(&server_run_id)?;
    ensure!(
        !server_uuid.is_nil()
            && server_uuid.to_string() == server_run_id
            && attestation.try_get::<String, _>("issuer")? != trusted_reserver_login
            && attestation.try_get::<String, _>("issuer")? != status_reader_login
            && attestation.try_get::<String, _>("tenant_id")? == tenant
            && attestation.try_get::<String, _>("context_key")? == context
            && attestation.try_get::<String, _>("plan_hash")? == plan_hash
            && attestation.try_get::<String, _>("parent_workflow_id")? == workflow_id
            && attestation.try_get::<String, _>("parent_workflow_type")?
                == OPERATION_DAG_PARENT_TYPE_V2
            && attestation.try_get::<String, _>("parent_task_queue")?
                == OPERATION_DAG_PARENT_QUEUE_V2,
        "separately issued governance does not match exact one-step identity"
    );
    let temporal_url = env::var(GRANTED_STATUS_TEMPORAL_URL_ENV)?;
    checked_local_temporal_url(&temporal_url)?;
    let namespace = env::var(GRANTED_STATUS_NAMESPACE_ENV)?;
    ensure!(
        bounded_policy_identifier(&namespace),
        "invalid independent Temporal namespace"
    );
    let catalog_path = env::var(GRANTED_STATUS_CATALOG_ENV)?;
    let catalog = operator_dag_governance::read_operator_owned_catalog(Path::new(&catalog_path))?;
    let client = crate::connect_client(
        &temporal_url,
        format!("alegria-granted-start-history-{}", std::process::id()),
        &namespace,
    )
    .await?;
    let history = fetch_exact_history(&client, &namespace, workflow_id, &server_run_id).await?;
    ensure!(
        history.events.len() <= 1_000,
        "one-step Temporal History is unbounded"
    );
    let payload = first_started_payload(
        &history,
        OPERATION_DAG_PARENT_TYPE_V2,
        OPERATION_DAG_PARENT_QUEUE_V2,
    )?;
    ensure!(
        !payload.is_empty() && payload.len() <= DAG_MAX_INPUT_BYTES,
        "one-step Temporal parent Start payload exceeds budget"
    );
    let exact_sha256: String =
        sqlx::query_scalar("SELECT encode(public.digest($1::bytea,'sha256'),'hex')")
            .bind(&payload)
            .fetch_one(&pool)
            .await?;
    ensure!(
        exact_sha256 == attestation.try_get::<String, _>("parent_start_sha256")?,
        "Temporal server Start bytes differ from independently issued SQL hash"
    );
    let run: OperationDagRunV6 = serde_json::from_slice(&payload)?;
    ensure!(
        run.schema_revision == 6
            && serde_json::to_vec(&run)? == payload
            && run.plan.scope.tenant_id == tenant
            && run.plan.scope.run_id == canonical_run_id.to_string()
            && run.plan.hash.as_str() == plan_hash
            && run.plan.nodes.len() == 1
            && run.owner_commands.len() == 1
            && run.pinned_routes.len() == 1,
        "Temporal parent Start differs from canonical single operation"
    );
    run.plan.validate(catalog.catalog())?;
    catalog.validate_run_routes(&run)?;
    Ok(Some(server_run_id))
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
    start_private_request_in_mode(path, StartMode::PrivateOperator).await
}

/// Local opt-in pilot only. This is deliberately not a ChatGPT MCP mutation:
/// a SQL principal with independently provisioned tenant/ACL/budget policy and
/// a private operator-controlled immutable run are prerequisites.
pub(super) async fn start_from_granted_private_request(path: &Path) -> Result<()> {
    start_private_request_in_mode(path, StartMode::IndependentlyGranted).await
}

/// Authenticated entry for a separately connected, private transport. The
/// signed file must be supplied by the OAuth-verified resource Worker through
/// an independently secured bridge; the full DAG intent is still a distinct
/// operator-owned private file. A successful signature is NOT a SQL grant.
/// This pilot binds exactly one installed subject to one operator principal;
/// arbitrary multi-user subjects are deliberately unsupported until a trusted
/// per-subject SQL ACL is implemented and physically accepted.
pub(super) async fn start_from_signed_granted_private_request(
    signed_path: &Path,
    private_intent_path: &Path,
) -> Result<()> {
    if authorize_signed_one_step_request(signed_path, private_intent_path).is_err() {
        println!(
            "{}",
            serde_json::json!({
                "schema_revision":1,
                "operation":"site_architect_start_signed_granted_single_v1",
                "status":"launch_blocked",
                "code":"signed_remote_admission_not_verified",
                "execution":"not_started",
                "next_action":"Check independently configured OAuth subject, short-lived signature and private intent; no Temporal Start was attempted."
            })
        );
        anyhow::bail!("remote MCP signed admission refused before SQL or Temporal Start");
    }
    start_from_granted_private_request(private_intent_path).await
}

fn authorize_signed_one_step_request(signed_path: &Path, private_intent_path: &Path) -> Result<()> {
    let issuer = env::var(SIGNED_ADMISSION_ISSUER_ENV).context("missing installed OAuth issuer")?;
    let audience =
        env::var(SIGNED_ADMISSION_RESOURCE_ENV).context("missing installed MCP resource")?;
    let subject =
        env::var(SIGNED_ADMISSION_SUBJECT_ENV).context("missing installed OAuth subject")?;
    let key_file =
        env::var(SIGNED_ADMISSION_KEY_FILE_ENV).context("missing installed admission key")?;
    let key_bytes = operator_dag_governance::read_private_operator_file(
        Path::new(&key_file),
        "short-lived remote admission HMAC verification key",
    )?;
    let key = decode_admission_signing_key_v1(&key_bytes)
        .context("invalid private admission verification key")?;
    let intent_bytes = operator_dag_governance::read_private_operator_file(
        private_intent_path,
        "separately stored one-use granted private intent",
    )?;
    let intent: GrantedSingleOperationStartIntentV1 = serde_json::from_slice(&intent_bytes)
        .context("invalid stored one-use granted private intent")?;
    ensure!(
        serde_json::to_vec(&intent)? == intent_bytes && intent.schema_revision == 1,
        "noncanonical private granted Start intent"
    );
    let signed_bytes = operator_dag_governance::read_private_operator_file(
        signed_path,
        "OAuth MCP authenticated admission proof",
    )?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("operator clock invalid")?
        .as_secs() as i64;
    verify_signed_one_step_admission_v1(
        &signed_bytes,
        &key,
        &issuer,
        &audience,
        &subject,
        &intent.request_id,
        now,
    )
    .context("remote MCP ingress signature, identity or one-use request invalid")?;
    Ok(())
}

async fn start_private_request_in_mode(path: &Path, mode: StartMode) -> Result<()> {
    let mut stage = SingleStartStage::OperatorConfiguration;
    let mut run_id = None;
    let mut parent_workflow_id = None;
    match try_start_from_private_request(
        path,
        mode,
        &mut stage,
        &mut run_id,
        &mut parent_workflow_id,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(_) => {
            // Never echo private source bytes, provider commands, SQL errors,
            // credentials or an unverified caller-provided tenant identity.
            // In particular, an RPC failure after submission does NOT prove
            // that Temporal rejected the Start. Its deterministic Workflow ID
            // is the only safe place to reconcile the unknown outcome.
            let mut report =
                start_failure_report(stage, run_id.as_deref(), parent_workflow_id.as_deref());
            if mode == StartMode::IndependentlyGranted {
                report.operation = "site_architect_start_granted_single_v1";
            }
            println!("{}", serde_json::to_string(&report)?);
            anyhow::bail!("single-step V2 operator refused or could not confirm Temporal Start; see redacted JSON diagnostic")
        }
    }
}

async fn try_start_from_private_request(
    path: &Path,
    mode: StartMode,
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
    let (intent, grant_request_id) = match mode {
        StartMode::PrivateOperator => (
            decode_checked_intent(&bytes, &config.tenant, &catalog, now)?,
            None,
        ),
        StartMode::IndependentlyGranted => {
            let (checked, request_id) =
                decode_granted_intent(&bytes, &config.tenant, &catalog, now)?;
            (checked, Some(request_id))
        }
    };
    *run_id = Some(intent.run.plan.scope.run_id.clone());
    // The independent principal/policy/ceiling MUST NOT come from the run,
    // its wrapper or the SQL grant row. They belong to a separately
    // provisioned OS operator and an independently tenant-bound SQL LOGIN.
    // This CLI proves no remote ChatGPT subject authentication.
    let grant_policy = if let Some(request_id) = grant_request_id {
        *stage = SingleStartStage::IndependentRequestGrant;
        let limits_file = env::var(GRANTED_LIMITS_FILE_ENV)
            .context("one-shot Start requires private authorized maximum limits")?;
        let limits_bytes = operator_dag_governance::read_private_operator_file(
            Path::new(&limits_file),
            "one-shot independently authorized maximum limits",
        )?;
        let (principal_id, policy_id, context_key, maximum_limits) =
            checked_grant_config(|key| env::var(key).ok(), &limits_bytes)?;
        Some(CheckedGrantPolicy {
            request_id,
            principal_id,
            policy_id,
            context_key,
            maximum_limits,
        })
    } else {
        None
    };
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
    let grant_pool = if let Some(policy) = grant_policy.as_ref() {
        *stage = SingleStartStage::IndependentRequestGrant;
        let restricted = timeout(
            Duration::from_secs(15),
            checked_one_shot_grant_pool(&config.tenant),
        )
        .await
        .context("independent one-shot grant LOGIN connection timed out")??;
        timeout(
            Duration::from_secs(15),
            independently_check_one_shot_grant(
                &restricted,
                policy,
                &config.tenant,
                &intent,
                &catalog,
                i64::try_from(now)?,
            ),
        )
        .await
        .context("independent one-shot grant preflight timed out")??;
        Some(restricted)
    } else {
        None
    };
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
    if let (Some(policy), Some(grant_pool)) = (&grant_policy, &grant_pool) {
        // A preflight alone is never a Start permit: recheck the SQL issuer
        // record against independent ACL/ceiling immediately before the
        // transactionally exclusive one-use reserve.
        *stage = SingleStartStage::IndependentRequestGrant;
        timeout(
            Duration::from_secs(15),
            independently_check_one_shot_grant(
                grant_pool,
                policy,
                &config.tenant,
                &intent,
                &catalog,
                i64::try_from(final_dispatch_time)?,
            ),
        )
        .await
        .context("final independent one-shot grant preflight timed out")??;
        let reservation_nonce = uuid::Uuid::new_v4();
        // From this point a lost SQL acknowledgement can leave a durable
        // reservation. Never issue Temporal Start or retry the claim on an
        // ambiguous SQL response: reconciliation is mandatory.
        *stage = SingleStartStage::GrantReservationUncertain;
        timeout(
            Duration::from_secs(15),
            sql_reserve_one_shot(
                grant_pool,
                policy,
                &config.tenant,
                &intent,
                reservation_nonce,
            ),
        )
        .await
        .context("one-shot durable reservation response uncertain")??;
        // Persist Unknown *before* any RPC. If the process dies or loses the
        // SQL acknowledgement, this request is never automatically replayed.
        *stage = SingleStartStage::GrantUnknownTransitionUncertain;
        timeout(
            Duration::from_secs(15),
            sql_commit_unknown_before_temporal_start(
                grant_pool,
                policy.request_id,
                reservation_nonce,
            ),
        )
        .await
        .context("one-shot SQL outcome fence not confirmed; no Start sent")??;
        *stage = SingleStartStage::GrantUnknownCommittedStartNotSent;
    }
    // Both SQL commits are await points. Expiry after the durable unknown
    // fence must refuse the RPC (without replaying the consumed grant).
    let after_sql_fence = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before Unix epoch")?
        .as_secs();
    checked_request_ttl(intent.expires_at_unix_seconds, after_sql_fence)?;
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
            "operation":if mode == StartMode::IndependentlyGranted {
                "site_architect_start_granted_single_v1"
            } else {
                "site_lab_start_single_v2"
            },
            "status":if mode == StartMode::IndependentlyGranted {
                "temporal_start_acknowledged_grant_pending_independent_history"
            } else {
                "started_business_root_verified_owner_unadmitted"
            },
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
            "grant_state":if mode == StartMode::IndependentlyGranted {
                "start_outcome_unknown_until_independent_temporal_history_verification"
            } else {
                "not_required_for_private_operator_pilot"
            },
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

    #[test]
    fn granted_status_locator_accepts_only_one_canonical_opaque_request_uuid() {
        let canonical =
            br#"{"schema_revision":1,"request_id":"e0813cac-2277-454e-980a-2e08110c1a08"}"#;
        let checked = decode_granted_start_status_locator(canonical).unwrap();
        assert_eq!(
            checked.request_id.to_string(),
            "e0813cac-2277-454e-980a-2e08110c1a08"
        );
        let rejected: &[&[u8]] = &[
            br#"{"request_id":"e0813cac-2277-454e-980a-2e08110c1a08","schema_revision":1}"#,
            br#"{"schema_revision":2,"request_id":"e0813cac-2277-454e-980a-2e08110c1a08"}"#,
            br#"{"schema_revision":1,"request_id":"00000000-0000-0000-0000-000000000000"}"#,
            br#"{"schema_revision":1,"request_id":"E0813CAC-2277-454E-980A-2E08110C1A08"}"#,
            br#"{"schema_revision":1,"request_id":"e0813cac-2277-454e-980a-2e08110c1a08","tenant_id":"forged"}"#,
            br#"{"schema_revision":1,"request_id":"e0813cac-2277-454e-980a-2e08110c1a08","parent_workflow_run_id":"forged"}"#,
        ];
        for &rejected in rejected {
            assert!(decode_granted_start_status_locator(rejected).is_err());
        }
    }

    #[test]
    fn granted_start_status_never_conflates_temporal_start_with_owner_execution() {
        let known = [
            ("unclaimed", "grant_unclaimed"),
            ("reserved", "grant_reserved_outcome_unverified"),
            ("start_outcome_unknown", "start_outcome_unknown"),
            ("start_confirmed", "sql_grant_confirmed_history_unverified"),
        ];
        for (sql, expected) in known {
            let state = PersistedGrantedStartState::from_sql(sql).unwrap();
            assert_eq!(state.as_str(), sql);
            assert_eq!(state.status(false), expected);
            assert_eq!(
                state.requires_history_inspection(),
                sql == "start_outcome_unknown" || sql == "start_confirmed"
            );
        }
        let unknown = PersistedGrantedStartState::StartOutcomeUnknown;
        assert_eq!(
            unknown.status(true),
            "temporal_start_history_verified_grant_pending_confirmation"
        );
        assert!(unknown.next_action(false).contains("Never repeat Start"));
        assert!(unknown.next_action(true).contains("read did not change it"));
        let confirmed = PersistedGrantedStartState::StartConfirmed;
        assert_eq!(
            confirmed.status(true),
            "temporal_start_history_verified_grant_confirmed"
        );
        assert!(confirmed.next_action(true).contains("provider receipt"));
        assert!(PersistedGrantedStartState::from_sql("start_submitted").is_err());
        assert!(PersistedGrantedStartState::from_sql("completed").is_err());
    }

    #[test]
    fn one_shot_status_reader_is_not_the_authenticated_reservation_owner() {
        let nonce = uuid::Uuid::new_v4();
        let valid = |state, issuer, reader, trusted_reserver, recorded, recorded_nonce| {
            status_grant_actors_are_independent(
                state,
                issuer,
                reader,
                trusted_reserver,
                recorded,
                recorded_nonce,
            )
        };
        assert!(valid(
            PersistedGrantedStartState::Unclaimed,
            "grant_issuer",
            "status_reader",
            "start_reserver",
            None,
            None
        ));
        for state in [
            PersistedGrantedStartState::Reserved,
            PersistedGrantedStartState::StartOutcomeUnknown,
            PersistedGrantedStartState::StartConfirmed,
        ] {
            assert!(valid(
                state,
                "grant_issuer",
                "status_reader",
                "start_reserver",
                Some("start_reserver"),
                Some(nonce)
            ));
            for (issuer, reader, reserver, recorded) in [
                (
                    "grant_issuer",
                    "status_reader",
                    "start_reserver",
                    Some("status_reader"),
                ),
                (
                    "grant_issuer",
                    "start_reserver",
                    "start_reserver",
                    Some("start_reserver"),
                ),
                (
                    "start_reserver",
                    "status_reader",
                    "start_reserver",
                    Some("start_reserver"),
                ),
                (
                    "status_reader",
                    "status_reader",
                    "start_reserver",
                    Some("start_reserver"),
                ),
                (
                    "grant_issuer",
                    "status_reader",
                    "wrong_reserver",
                    Some("start_reserver"),
                ),
            ] {
                assert!(!valid(
                    state,
                    issuer,
                    reader,
                    reserver,
                    recorded,
                    Some(nonce)
                ));
            }
            assert!(!valid(
                state,
                "grant_issuer",
                "status_reader",
                "start_reserver",
                Some("start_reserver"),
                Some(uuid::Uuid::nil())
            ));
        }
        assert!(!valid(
            PersistedGrantedStartState::Unclaimed,
            "grant_issuer",
            "status_reader",
            "start_reserver",
            Some("start_reserver"),
            Some(nonce)
        ));
    }

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
        assert!(checked_source_record_sql_metadata(
            "source_issuer",
            "start_reader",
            14,
            &hash,
            &hash
        )
        .is_ok());
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
        assert!(checked_source_record_sql_metadata(
            "source_issuer",
            "start_reader",
            14,
            &hash,
            "BAD_SHA256"
        )
        .is_err());
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
    fn granted_start_requires_separately_configured_principal_policy_and_budget() {
        let limits = OneStepResourceLimitsV1 {
            max_admitted_source_bytes: 1024,
            max_effect_dispatches: 1,
            max_provider_requests: 1,
            currency_code: "USD".into(),
            max_charge_minor_units: 50,
        };
        let bytes = serde_json::to_vec(&limits).unwrap();
        let trusted = |key: &str| match key {
            GRANTED_ENABLE_ENV => Some("true".into()),
            GRANTED_PRINCIPAL_ENV => Some("principal-7".into()),
            GRANTED_POLICY_ENV => Some("site-limited-v1".into()),
            GRANTED_CONTEXT_ENV => Some("site-a/research".into()),
            _ => None,
        };
        let (principal, policy, context, decoded) = checked_grant_config(trusted, &bytes).unwrap();
        assert_eq!(
            (principal.as_str(), policy.as_str(), context.as_str()),
            ("principal-7", "site-limited-v1", "site-a/research")
        );
        assert_eq!(decoded, limits);
        assert!(checked_grant_config(|_| None, &bytes).is_err());
        assert!(checked_grant_config(
            |key| if key == GRANTED_PRINCIPAL_ENV {
                Some("attacker\nprincipal".into())
            } else {
                trusted(key)
            },
            &bytes
        )
        .is_err());
        assert!(checked_grant_config(trusted, b"{\"currency_code\":\"USD\"}").is_err());
        let mut reordered = serde_json::to_value(&limits).unwrap();
        reordered["unexpected_budget_override"] = serde_json::json!(0);
        assert!(checked_grant_config(trusted, &serde_json::to_vec(&reordered).unwrap()).is_err());
    }

    #[test]
    fn granted_operator_envelope_cannot_smuggle_identity_or_temporal_route() {
        let expected = SiteLabExpectedStartV1 {
            schema_revision: 1,
            plan_hash: PlanHash("a".repeat(64)),
            command_hashes: BTreeMap::new(),
            source_payload_hashes: BTreeMap::new(),
        };
        let wrapper = GrantedSingleOperationStartIntentV1 {
            schema_revision: 1,
            request_id: "e0813cac-2277-454e-980a-2e08110c1a08".into(),
            intent: SingleOperationStartIntentV1 {
                schema_revision: 1,
                run: one_step_candidate("acquisition"),
                context_key: "site-a/research".into(),
                expected,
                expires_at_unix_seconds: 1_900_000_100,
            },
        };
        let mut forged = serde_json::to_value(&wrapper).unwrap();
        forged["tenant_id"] = serde_json::json!("attacker");
        assert!(
            serde_json::from_slice::<GrantedSingleOperationStartIntentV1>(
                &serde_json::to_vec(&forged).unwrap()
            )
            .is_err()
        );
        let mut forged = serde_json::to_value(&wrapper).unwrap();
        forged["temporal_url"] = serde_json::json!("http://evil:7233/");
        assert!(
            serde_json::from_slice::<GrantedSingleOperationStartIntentV1>(
                &serde_json::to_vec(&forged).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn grant_lost_sql_ack_is_not_misreported_as_temporal_submission() {
        for stage in [
            SingleStartStage::GrantReservationUncertain,
            SingleStartStage::GrantUnknownTransitionUncertain,
        ] {
            let report = serde_json::to_value(start_failure_report(stage, None, None)).unwrap();
            assert_eq!(report["status"], "grant_state_uncertain_start_not_sent");
            assert_eq!(report["execution"], "not_confirmed");
            assert_ne!(report["code"], "temporal_start_result_unknown");
            assert!(report["next_action"]
                .as_str()
                .unwrap()
                .contains("Do not retry"));
        }
        let preflight = serde_json::to_value(start_failure_report(
            SingleStartStage::IndependentRequestGrant,
            None,
            None,
        ))
        .unwrap();
        assert_eq!(preflight["status"], "refused_before_temporal_start");
        assert_eq!(preflight["code"], "independent_request_grant_unavailable");
        let consumed = serde_json::to_value(start_failure_report(
            SingleStartStage::GrantUnknownCommittedStartNotSent,
            None,
            None,
        ))
        .unwrap();
        assert_eq!(consumed["status"], "grant_consumed_start_not_sent");
        assert!(consumed["next_action"]
            .as_str()
            .unwrap()
            .contains("Never retry"));
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
