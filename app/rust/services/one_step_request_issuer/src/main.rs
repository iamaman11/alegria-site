//! Separately credentialed tenant-specific issuance of one preapproved V2
//! operation request. This binary has no Temporal SDK, provider I/O, public
//! HTTP/MCP listener or arbitrary Start method. The SQL approval must have
//! been inserted by a DIFFERENT login authorized under a DBA-provisioned
//! principal policy; this process can only SELECT the approval and INSERT
//! one immutable grant. SQL independently rechecks the same requirements.
//!
//! Distinguish the states precisely: an issued request grant is *not* an
//! executed operation, nor an authenticated remote ChatGPT user identity.
//! A separately credentialed one-use Start bridge still must reserve and
//! durably record the Temporal RPC's ambiguous outcome before dispatch.

mod catalog;
mod issue_json;
mod private_file;

use std::{env, path::Path};

use anyhow::{ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, preflight_issued_one_step_request_for_reservation_v1,
    IndependentlyAuthenticatedOneStepPolicyV1, OneStepRequestIssuanceStateV1,
    OneStepResourceLimitsV1, OpaqueOneStepRequestLocatorV1, TrustedOneStepRequestGrantV1,
    TrustedRouteCatalog, ONE_STEP_REQUEST_GRANT_SCHEMA_REVISION_V1,
};
use sqlx::{types::Json, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::issue_json::OneStepIssuanceIntentV1;

const ENABLE_ENV: &str = "ALEGRIA_ENABLE_ONE_STEP_REQUEST_ISSUER_V1";
const DSN_FILE_ENV: &str = "ONE_STEP_REQUEST_ISSUER_DATABASE_URL_FILE";
const CATALOG_FILE_ENV: &str = "ONE_STEP_REQUEST_ISSUER_TRUSTED_CATALOG_FILE";

fn checked_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn checked_canonical_uuid(value: &str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value).context("invalid canonical run identity")?;
    ensure!(
        !parsed.is_nil() && parsed.to_string() == value,
        "run identity must be canonical nonnil UUID"
    );
    Ok(parsed)
}

/// Approval and DBA policy are distinct SQL records. Neither is populated
/// from the caller's private request fields or a self-consistent DAG.
struct IndependentlyApprovedRequestV1 {
    request_id: Uuid,
    authorization_event_id: Uuid,
    run_id: Uuid,
    tenant_id: String,
    principal_id: String,
    policy_id: String,
    context_key: String,
    operation_id: String,
    operation_revision: u32,
    parent_workflow_id: String,
    plan_hash: String,
    expected: operation_protocol::SiteArchitectExpectedStartV1,
    limits: OneStepResourceLimitsV1,
    maximum_limits: OneStepResourceLimitsV1,
    approved_at_unix_seconds: i64,
    expires_at_unix_seconds: i64,
    revoked: bool,
    db_now_unix_seconds: i64,
}

async fn restricted_grant_pool() -> Result<PgPool> {
    let dsn_path = env::var(DSN_FILE_ENV).context("missing request issuer DB credential path")?;
    let bytes = private_file::read_private_file(Path::new(&dsn_path))
        .context("issuer_phase_sql_dsn_file")?;
    let raw = String::from_utf8(bytes).context("request issuer DSN must be UTF-8")?;
    let dsn = raw.trim_end_matches(['\r', '\n']);
    ensure!(!dsn.is_empty() && dsn.trim() == dsn, "invalid issuer DSN");
    let pool = PgPool::connect(dsn)
        .await
        .context("independent request issuer SQL login unavailable")
        .context("issuer_phase_sql_connect")?;
    let role = sqlx::query(
        // The issuer must NOT need USAGE on raw merely to check that it has
        // zero raw DML authority. Resolve raw relations from pg_catalog OIDs:
        // a name-based has_table_privilege lookup can itself require schema
        // visibility, accidentally expanding this issuer's SQL privileges.
        "WITH raw_table_oids AS (
           SELECT
             (SELECT c.oid FROM pg_catalog.pg_class c
              JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
              WHERE n.nspname='raw' AND c.relname='pages') pages_oid,
             (SELECT c.oid FROM pg_catalog.pg_class c
              JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
              WHERE n.nspname='raw' AND c.relname='sections') sections_oid
         )
         SELECT current_user::text effective, session_user::text login,
                r.rolcanlogin login_enabled, r.rolinherit inherits_roles,
                r.rolsuper privileged, r.rolbypassrls bypass,
                r.rolcreaterole role_admin, r.rolcreatedb db_admin,
                r.rolreplication replication,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_grants_v1','INSERT') grant_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_grants_v1','UPDATE') grant_update,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_grants_v1','DELETE') grant_delete,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_grants_v1','TRUNCATE') grant_truncate,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_approvals_v1','INSERT') approval_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_approvals_v1','UPDATE') approval_update,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_approvals_v1','DELETE') approval_delete,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_request_approvals_v1','TRUNCATE') approval_truncate,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_principal_policies_v1','INSERT') policy_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_principal_policies_v1','UPDATE') policy_update,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_principal_policies_v1','DELETE') policy_delete,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_one_step_principal_policies_v1','TRUNCATE') policy_truncate,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_business_runs_v1','INSERT') root_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_business_runs_v1','UPDATE') root_update,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_external_source_attestations_v1','INSERT') source_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_external_source_attestations_v1','UPDATE') source_update,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_run_governance_v2','INSERT') governance_insert,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_run_governance_v2','UPDATE') governance_update,
                coalesce(has_table_privilege(session_user,raw_table_oids.pages_oid,'INSERT'),true) raw_insert,
                coalesce(has_table_privilege(session_user,raw_table_oids.pages_oid,'UPDATE'),true) raw_update,
                coalesce(has_table_privilege(session_user,raw_table_oids.pages_oid,'DELETE'),true) raw_delete,
                coalesce(has_table_privilege(session_user,raw_table_oids.pages_oid,'TRUNCATE'),true) raw_truncate,
                coalesce(has_table_privilege(session_user,raw_table_oids.sections_oid,'INSERT'),true) sections_insert,
                coalesce(has_table_privilege(session_user,raw_table_oids.sections_oid,'UPDATE'),true) sections_update,
                coalesce(has_table_privilege(session_user,raw_table_oids.sections_oid,'DELETE'),true) sections_delete,
                coalesce(has_table_privilege(session_user,raw_table_oids.sections_oid,'TRUNCATE'),true) sections_truncate
         FROM pg_catalog.pg_roles r CROSS JOIN raw_table_oids
         WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await
    .context("issuer_phase_sql_role_query")?;
    // Fixed, non-sensitive failure categories make isolated CI role setup
    // diagnosable without ever returning SQL errors, paths, or login names.
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("privileged")
            && !role.get::<bool, _>("bypass")
            && !role.get::<bool, _>("role_admin")
            && !role.get::<bool, _>("db_admin")
            && !role.get::<bool, _>("replication"),
        "issuer_phase_sql_role_login_shape"
    );
    ensure!(
        role.get::<bool, _>("grant_insert")
            && !role.get::<bool, _>("grant_update")
            && !role.get::<bool, _>("grant_delete")
            && !role.get::<bool, _>("grant_truncate"),
        "issuer_phase_sql_grant_dml"
    );
    ensure!(
        !role.get::<bool, _>("approval_insert")
            && !role.get::<bool, _>("approval_update")
            && !role.get::<bool, _>("approval_delete")
            && !role.get::<bool, _>("approval_truncate"),
        "issuer_phase_sql_approval_dml"
    );
    ensure!(
        !role.get::<bool, _>("policy_insert")
            && !role.get::<bool, _>("policy_update")
            && !role.get::<bool, _>("policy_delete")
            && !role.get::<bool, _>("policy_truncate"),
        "issuer_phase_sql_policy_dml"
    );
    ensure!(
        !role.get::<bool, _>("root_insert")
            && !role.get::<bool, _>("root_update")
            && !role.get::<bool, _>("source_insert")
            && !role.get::<bool, _>("source_update")
            && !role.get::<bool, _>("governance_insert")
            && !role.get::<bool, _>("governance_update"),
        "issuer_phase_sql_owner_dml"
    );
    ensure!(
        !role.get::<bool, _>("raw_insert")
            && !role.get::<bool, _>("raw_update")
            && !role.get::<bool, _>("raw_delete")
            && !role.get::<bool, _>("raw_truncate")
            && !role.get::<bool, _>("sections_insert")
            && !role.get::<bool, _>("sections_update")
            && !role.get::<bool, _>("sections_delete")
            && !role.get::<bool, _>("sections_truncate"),
        "issuer_phase_sql_raw_dml"
    );
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("privileged")
            && !role.get::<bool, _>("bypass")
            && !role.get::<bool, _>("role_admin")
            && !role.get::<bool, _>("db_admin")
            && !role.get::<bool, _>("replication")
            && role.get::<bool, _>("grant_insert")
            && !role.get::<bool, _>("grant_update")
            && !role.get::<bool, _>("grant_delete")
            && !role.get::<bool, _>("grant_truncate")
            && !role.get::<bool, _>("approval_insert")
            && !role.get::<bool, _>("approval_update")
            && !role.get::<bool, _>("approval_delete")
            && !role.get::<bool, _>("approval_truncate")
            && !role.get::<bool, _>("policy_insert")
            && !role.get::<bool, _>("policy_update")
            && !role.get::<bool, _>("policy_delete")
            && !role.get::<bool, _>("policy_truncate")
            && !role.get::<bool, _>("root_insert")
            && !role.get::<bool, _>("root_update")
            && !role.get::<bool, _>("source_insert")
            && !role.get::<bool, _>("source_update")
            && !role.get::<bool, _>("governance_insert")
            && !role.get::<bool, _>("governance_update")
            && !role.get::<bool, _>("raw_insert")
            && !role.get::<bool, _>("raw_update")
            && !role.get::<bool, _>("raw_delete")
            && !role.get::<bool, _>("raw_truncate")
            && !role.get::<bool, _>("sections_insert")
            && !role.get::<bool, _>("sections_update")
            && !role.get::<bool, _>("sections_delete")
            && !role.get::<bool, _>("sections_truncate"),
        "one-step request SQL login has unsafe or insufficient privileges"
    );
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT granted.rolname::text FROM pg_catalog.pg_auth_members m
         JOIN pg_catalog.pg_roles granted ON granted.oid=m.roleid
         WHERE m.member=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user)
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .context("issuer_phase_sql_membership_query")?;
    ensure!(membership.is_none(), "issuer_phase_sql_membership_denied");
    Ok(pool)
}

async fn independently_approved_request(
    transaction: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    tenant: &str,
) -> Result<IndependentlyApprovedRequestV1> {
    let row = sqlx::query(
        "SELECT a.request_id, a.authorization_event_id, a.run_id, a.tenant_id,
                a.principal_id, a.policy_id, a.context_key, a.operation_id,
                a.operation_revision, a.parent_workflow_id, a.plan_hash,
                a.expected_start, a.limits, p.max_limits,
                extract(epoch from a.approved_at)::bigint approved_at_seconds,
                extract(epoch from a.expires_at)::bigint expires_at_seconds,
                (a.revoked_at IS NOT NULL) revoked,
                extract(epoch from clock_timestamp())::bigint db_now_seconds
         FROM pipeline.operation_dag_one_step_request_approvals_v1 a
         JOIN pipeline.operation_dag_one_step_principal_policies_v1 p
           ON p.role_name = a.approved_by
          AND p.tenant_id = a.tenant_id AND p.context_key = a.context_key
          AND p.principal_id = a.principal_id AND p.policy_id = a.policy_id
         WHERE a.authorization_event_id = $1 AND a.tenant_id = $2",
    )
    .bind(event_id)
    .bind(tenant)
    .fetch_optional(&mut **transaction)
    .await?
    .context("independent approved request and trusted principal policy unavailable")?;
    let operation_revision: i32 = row.try_get("operation_revision")?;
    let operation_revision = u32::try_from(operation_revision)?;
    Ok(IndependentlyApprovedRequestV1 {
        request_id: row.try_get("request_id")?,
        authorization_event_id: row.try_get("authorization_event_id")?,
        run_id: row.try_get("run_id")?,
        tenant_id: row.try_get("tenant_id")?,
        principal_id: row.try_get("principal_id")?,
        policy_id: row.try_get("policy_id")?,
        context_key: row.try_get("context_key")?,
        operation_id: row.try_get("operation_id")?,
        operation_revision,
        parent_workflow_id: row.try_get("parent_workflow_id")?,
        plan_hash: row.try_get("plan_hash")?,
        expected: row.try_get::<Json<_>, _>("expected_start")?.0,
        limits: row.try_get::<Json<_>, _>("limits")?.0,
        maximum_limits: row.try_get::<Json<_>, _>("max_limits")?.0,
        approved_at_unix_seconds: row.try_get("approved_at_seconds")?,
        expires_at_unix_seconds: row.try_get("expires_at_seconds")?,
        revoked: row.try_get("revoked")?,
        db_now_unix_seconds: row.try_get("db_now_seconds")?,
    })
}

fn verified_private_intent<'a>(
    intent: &OneStepIssuanceIntentV1,
    approved: &IndependentlyApprovedRequestV1,
    catalog: &'a TrustedRouteCatalog,
) -> Result<TrustedOneStepRequestGrantV1> {
    let run_id = checked_canonical_uuid(&intent.run.plan.scope.run_id)?;
    ensure!(
        checked_identifier(&intent.context_key, 512)
            && intent.run.plan.nodes.len() == 1
            && intent.request_id == approved.request_id
            && intent.authorization_event_id == approved.authorization_event_id
            && run_id == approved.run_id
            && approved.tenant_id == intent.run.plan.scope.tenant_id
            && approved.context_key == intent.context_key
            && !approved.revoked
            && approved.approved_at_unix_seconds <= approved.db_now_unix_seconds
            && approved.db_now_unix_seconds < approved.expires_at_unix_seconds
            && intent.expires_at_unix_seconds <= approved.expires_at_unix_seconds
            && intent.expires_at_unix_seconds > approved.db_now_unix_seconds
            && approved.parent_workflow_id
                == operation_dag_parent_workflow_id_v2(&intent.run.plan.scope.run_id)
            && approved.plan_hash == intent.run.plan.hash.as_str()
            && intent.expected == approved.expected
            && intent.run.plan.nodes[0].operation.id.as_str() == approved.operation_id.as_str()
            && intent.run.plan.nodes[0].operation.revision == approved.operation_revision,
        "one-step private intent does not match independently approved, live SQL request"
    );
    let record = TrustedOneStepRequestGrantV1 {
        schema_revision: ONE_STEP_REQUEST_GRANT_SCHEMA_REVISION_V1,
        request_id: approved.request_id.to_string(),
        authorization_event_id: approved.authorization_event_id.to_string(),
        tenant_id: approved.tenant_id.clone(),
        authenticated_principal_id: approved.principal_id.clone(),
        authorization_policy_id: approved.policy_id.clone(),
        context_key: approved.context_key.clone(),
        canonical_run_id: approved.run_id.to_string(),
        operation: intent.run.plan.nodes[0].operation.clone(),
        parent_workflow_id: approved.parent_workflow_id.clone(),
        expected: approved.expected.clone(),
        limits: approved.limits.clone(),
        issued_at_unix_seconds: approved.db_now_unix_seconds,
        expires_at_unix_seconds: intent.expires_at_unix_seconds,
        revoked_at_unix_seconds: None,
        state: OneStepRequestIssuanceStateV1::Unclaimed,
    };
    let policy = IndependentlyAuthenticatedOneStepPolicyV1 {
        tenant_id: approved.tenant_id.clone(),
        principal_id: approved.principal_id.clone(),
        policy_id: approved.policy_id.clone(),
        context_key: approved.context_key.clone(),
        maximum_limits: approved.maximum_limits.clone(),
    };
    preflight_issued_one_step_request_for_reservation_v1(
        &OpaqueOneStepRequestLocatorV1 {
            request_id: intent.request_id.to_string(),
        },
        &record,
        &intent.run,
        catalog,
        &policy,
        approved.db_now_unix_seconds,
    )
    .context("approved one-step request fails sealed owner/ACL/budget preflight")?;
    Ok(record)
}

async fn insert_preapproved_grant(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OneStepIssuanceIntentV1,
    grant: &TrustedOneStepRequestGrantV1,
) -> Result<()> {
    let affected = sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_request_grants_v1
          (request_id, authorization_event_id, run_id, tenant_id, principal_id,
           policy_id, context_key, operation_id, operation_revision,
           parent_workflow_id, plan_hash, expected_start, limits, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,
                 to_timestamp($14::double precision))",
    )
    .bind(intent.request_id)
    .bind(intent.authorization_event_id)
    .bind(checked_canonical_uuid(&grant.canonical_run_id)?)
    .bind(&grant.tenant_id)
    .bind(&grant.authenticated_principal_id)
    .bind(&grant.authorization_policy_id)
    .bind(&grant.context_key)
    .bind(grant.operation.id.as_str())
    .bind(i32::try_from(grant.operation.revision)?)
    .bind(&grant.parent_workflow_id)
    .bind(grant.expected.plan_hash.as_str())
    .bind(Json(&grant.expected))
    .bind(Json(&grant.limits))
    .bind(intent.expires_at_unix_seconds)
    .execute(&mut **transaction)
    .await
    .context("SQL one-step grant issuance refused or already issued")?
    .rows_affected();
    ensure!(affected == 1, "one-step grant issuance inserted no record");
    Ok(())
}

async fn execute(path: &Path) -> Result<()> {
    ensure!(
        env::var(ENABLE_ENV).as_deref() == Ok("true"),
        "request issuer must be explicitly enabled by the operator"
    );
    let bytes = private_file::read_private_file(path).context("issuer_phase_private_input")?;
    let intent =
        issue_json::decode_issuance_intent(&bytes).context("issuer_phase_private_input")?;
    let catalog_path = env::var(CATALOG_FILE_ENV)
        .context("missing independently installed owner catalog path")
        .context("issuer_phase_catalog")?;
    let catalog = catalog::read_installed_catalog(Path::new(&catalog_path))
        .context("issuer_phase_catalog")?;
    // This call checks native V6 route/command/source hashes against a private
    // SQL-approved event and the independently installed provider-neutral
    // owner catalog. Neither the request nor issuer creates authorization.
    let pool = restricted_grant_pool()
        .await
        .context("issuer_phase_sql_principal")?;
    let mut transaction = pool.begin().await.context("issuer_phase_sql_principal")?;
    let tenant: Option<String> = sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
        .fetch_one(&mut *transaction)
        .await
        .context("issuer_phase_sql_tenant_query")?;
    let tenant = tenant
        .context("request issuer SQL login has no tenant identity")
        .context("issuer_phase_sql_tenant_identity")?;
    ensure!(
        tenant == intent.run.plan.scope.tenant_id,
        "issuer_phase_sql_tenant_identity"
    );
    let approved =
        independently_approved_request(&mut transaction, intent.authorization_event_id, &tenant)
            .await
            .context("issuer_phase_approval_lookup")?;
    let grant = verified_private_intent(&intent, &approved, &catalog)
        .context("issuer_phase_typed_preflight")?;
    insert_preapproved_grant(&mut transaction, &intent, &grant)
        .await
        .context("issuer_phase_grant_insert")?;
    transaction
        .commit()
        .await
        .context("one-step durable SQL issuance commit could not be confirmed")
        .context("issuer_phase_commit_unknown")?;
    println!(
        "{}",
        issue_json::issued_report(intent.request_id, &grant.canonical_run_id)
    );
    Ok(())
}

/// Only fixed stage labels can leave this process. Never log the underlying
/// anyhow chain: it may contain SQL diagnostics, credentials or private paths.
fn redacted_failure_phase(error: &anyhow::Error) -> &'static str {
    const STAGES: &[&str] = &[
        "issuer_phase_private_input",
        "issuer_phase_catalog",
        "issuer_phase_sql_dsn_file",
        "issuer_phase_sql_connect",
        "issuer_phase_sql_role_query",
        "issuer_phase_sql_role_login_shape",
        "issuer_phase_sql_grant_dml",
        "issuer_phase_sql_approval_dml",
        "issuer_phase_sql_policy_dml",
        "issuer_phase_sql_owner_dml",
        "issuer_phase_sql_raw_dml",
        "issuer_phase_sql_membership_query",
        "issuer_phase_sql_membership_denied",
        "issuer_phase_sql_tenant_query",
        "issuer_phase_sql_tenant_identity",
        "issuer_phase_sql_principal",
        "issuer_phase_approval_lookup",
        "issuer_phase_typed_preflight",
        "issuer_phase_grant_insert",
        "issuer_phase_commit_unknown",
    ];
    STAGES
        .iter()
        .copied()
        .find(|stage| error.chain().any(|cause| cause.to_string() == *stage))
        .unwrap_or("issuer_phase_other")
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args_os();
    let _program = args.next();
    let command = args.next();
    let private_path = args.next();
    ensure!(
        command.as_deref() == Some(std::ffi::OsStr::new("--issue-request-v1"))
            && private_path.is_some()
            && args.next().is_none(),
        "usage: one_step_request_issuer --issue-request-v1 /absolute/private/intent.json"
    );
    match execute(Path::new(
        private_path.as_ref().expect("private path present"),
    ))
    .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            // Redact SQL credentials, exact principal/policy, source, command
            // bytes and filesystem paths. A failed COMMIT is NOT proof of
            // rollback. Do not silently repeat a one-time authorization.
            eprintln!(
                "one-step issuer {}: sensitive diagnostic withheld",
                redacted_failure_phase(&error)
            );
            println!("{}", issue_json::unknown_report());
            anyhow::bail!(
                "one-step issuer refused or cannot confirm SQL commit; see redacted diagnostic"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::anyhow;

    #[test]
    fn redacted_failure_phase_reports_only_reviewed_fixed_labels() {
        let error =
            anyhow!("postgres_password=must-not-leak").context("issuer_phase_typed_preflight");
        assert_eq!(
            redacted_failure_phase(&error),
            "issuer_phase_typed_preflight"
        );
        let error = anyhow!("private path and SQL must not appear");
        assert_eq!(redacted_failure_phase(&error), "issuer_phase_other");
    }

    #[test]
    fn canonical_run_uuid_and_context_are_required() {
        let good = "befd8f0c-8f83-4b29-8c4b-9c5a7694a328";
        assert_eq!(checked_canonical_uuid(good).unwrap().to_string(), good);
        for bad in [
            "",
            "00000000-0000-0000-0000-000000000000",
            "BEFD8F0C-8F83-4B29-8C4B-9C5A7694A328",
            "befd8f0c-8f83-4b29-8c4b-9c5a7694a328-more",
        ] {
            assert!(checked_canonical_uuid(bad).is_err());
        }
        assert!(checked_identifier("tenant-a", 128));
        for bad in ["", " root", "tenant\nother", "x "] {
            assert!(!checked_identifier(bad, 128));
        }
    }
}
