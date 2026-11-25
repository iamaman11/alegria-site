//! Explicit, separately credentialed PostgreSQL V3 operator commands.
//!
//! This module cannot run from an Activity, a Workflow or an HTTP request.
//! It never starts/signals Temporal and cannot substitute for independently
//! established tenant ownership: that requires a canonical tenant model/RLS.

use std::{collections::BTreeSet, env, fs, path::Path, time::Duration};

use anyhow::{bail, ensure, Context, Result};
use postgres_store::truth_review_effect_v3::{
    contract_fingerprint, execution_key, ReconcileTruthReviewsInput, CANDIDATE_SET_HASH_SQL,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const MAX_ADMISSION_SECONDS: u32 = 300;
const MAX_REVOCATION_BUSY_RETRIES: usize = 3;

#[derive(Debug)]
struct RevocationBusy;

impl std::fmt::Display for RevocationBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "V3 admission effect in flight; revocation not confirmed")
    }
}

impl std::error::Error for RevocationBusy {}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequest {
    command: ReconcileTruthReviewsInput,
    workflow_id: String,
    workflow_run_id: String,
    expires_in_seconds: u32,
}

impl IssueRequest {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_ADMISSION_SECONDS).contains(&self.expires_in_seconds),
            "admission expiry must be 1..={MAX_ADMISSION_SECONDS} seconds"
        );
        // The canonical typed command, not an arbitrary digest supplied in a
        // manifest, determines the V3 execution identity.
        execution_key(&self.workflow_id, &self.workflow_run_id, &self.command)
            .context("invalid typed PG V3 command or Temporal execution identity")?;
        Ok(())
    }
}

/// The environment tenant is only a secondary scope check. Actual authority
/// comes from a DBA-provisioned PostgreSQL LOGIN→tenant binding plus canonical
/// run/context/source tenant rows; no self-asserted plan or GUC grants access.
fn enforce_operator_tenant(configured: &str, claimed: &str) -> Result<()> {
    ensure!(
        !configured.is_empty() && configured.trim() == configured && configured == claimed,
        "operator tenant policy does not match typed command scope"
    );
    Ok(())
}

pub(crate) async fn enforce_authoritative_pg_tenant(
    pool: &PgPool,
    tenant: &str,
) -> Result<()> {
    let actual: Option<String> = sqlx::query_scalar(
        "SELECT pipeline.current_session_tenant_v1()",
    )
    .fetch_one(pool)
    .await
    .context("resolve tenant from authenticated PostgreSQL session LOGIN")?;
    ensure!(
        actual.as_deref() == Some(tenant),
        "operator PostgreSQL LOGIN is not provisioned for requested tenant"
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct OperatorResult {
    operation: &'static str,
    admission_id: Uuid,
    status: &'static str,
    operator_role: String,
}

pub(crate) async fn operator_pool() -> Result<(PgPool, String)> {
    operator_pool_with_enable_flag("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V3").await
}

pub(crate) async fn operator_pool_with_enable_flag(enable_flag: &str) -> Result<(PgPool, String)> {
    ensure!(
        env::var(enable_flag).ok().as_deref() == Some("true"),
        "operator command disabled; set {enable_flag}=true"
    );
    let operator_url = env::var("PG_EFFECT_OPERATOR_DATABASE_URL")
        .context("operator command requires its own PG_EFFECT_OPERATOR_DATABASE_URL")?;
    ensure!(
        !operator_url.trim().is_empty(),
        "operator PostgreSQL DSN cannot be empty"
    );
    if let Ok(worker_url) = env::var("PG_EFFECT_WORKER_DATABASE_URL") {
        ensure!(
            worker_url != operator_url,
            "operator command cannot reuse PG effect worker credentials"
        );
    }
    let pool = PgPool::connect(&operator_url)
        .await
        .context("connect separately credentialed V3 operator")?;
    let role = assert_operator_role(&pool).await?;
    Ok((pool, role))
}

/// The operator may issue/revoke admissions but must not be able to change
/// evidence, Truth, HITL tasks, receipts or application schemas. A login
/// with role memberships is rejected even if NOINHERIT hides SET ROLE access.
async fn assert_operator_role(pool: &PgPool) -> Result<String> {
    let identity = sqlx::query(
        "SELECT current_user::text AS role_name,
                session_user::text AS login_role,
                r.rolcanlogin AS can_login,
                (r.rolsuper OR r.rolcreaterole OR r.rolcreatedb
                 OR r.rolreplication OR r.rolbypassrls) AS elevated,
                EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members AS m
                        WHERE m.member = r.oid) AS has_memberships
         FROM pg_catalog.pg_roles AS r WHERE r.rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    let role: String = identity.get("role_name");
    ensure!(
        role == identity.get::<String, _>("login_role")
            && identity.get::<bool, _>("can_login")
            && !identity.get::<bool, _>("elevated")
            && !identity.get::<bool, _>("has_memberships"),
        "V3 operator role must be its own non-elevated session LOGIN with no SET ROLE memberships"
    );
    let can_issue: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'SELECT')
             AND has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'INSERT')
             AND has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'UPDATE')",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        can_issue,
        "V3 operator role needs SELECT, INSERT and UPDATE on admissions"
    );
    let excess_table: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (
             (has_any_column_privilege(current_user, c.oid, 'INSERT')
              AND NOT (n.nspname = 'pipeline' AND c.relname = 'pg_effect_v3_admissions'))
             OR (has_any_column_privilege(current_user, c.oid, 'UPDATE')
              AND NOT (n.nspname = 'pipeline' AND c.relname = 'pg_effect_v3_admissions'))
             OR has_table_privilege(current_user, c.oid, 'DELETE')
             OR has_table_privilege(current_user, c.oid, 'TRUNCATE')
           )
         ORDER BY n.nspname, c.relname LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    ensure!(
        excess_table.is_none(),
        "V3 operator role has forbidden write authority on {:?}",
        excess_table
    );
    let excess_sequence: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind = 'S'
           AND (has_sequence_privilege(current_user, c.oid, 'USAGE')
             OR has_sequence_privilege(current_user, c.oid, 'UPDATE'))
         ORDER BY n.nspname, c.relname LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    ensure!(
        excess_sequence.is_none(),
        "V3 operator role has forbidden sequence authority on {:?}",
        excess_sequence
    );
    let excess_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname FROM pg_catalog.pg_namespace n
         WHERE n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'
           AND has_schema_privilege(current_user, n.oid, 'CREATE')
         ORDER BY n.nspname LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    ensure!(
        excess_schema.is_none(),
        "V3 operator role has forbidden schema CREATE authority on {:?}",
        excess_schema
    );
    Ok(role)
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes =
        fs::read(path).with_context(|| format!("read operator request {}", path.display()))?;
    let request: IssueRequest =
        serde_json::from_slice(&bytes).context("decode strict V3 operator issue request")?;
    // Tenant is an operationally provisioned issuer scope, not independent
    // PostgreSQL RLS: canonical run/source tables do not yet contain tenant_id.
    let operator_tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("operator must explicitly configure PG_EFFECT_OPERATOR_TENANT_ID")?;
    let (pool, role) = operator_pool().await?;
    let output = issue_with_pool(&pool, &role, &operator_tenant, &request).await?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Separated for isolated real-PostgreSQL role tests. This does not construct
/// an operator identity from the command: `role` must come from the already
/// verified current_user of the *same* restricted PG connection.
async fn issue_with_pool(
    pool: &PgPool,
    role: &str,
    operator_tenant: &str,
    request: &IssueRequest,
) -> Result<OperatorResult> {
    request.validate()?;
    enforce_operator_tenant(operator_tenant, &request.command.scope.tenant_id)?;
    enforce_authoritative_pg_tenant(pool, operator_tenant).await?;
    ensure!(
        assert_operator_role(pool).await? == role,
        "operator connection identity changed after role verification"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{}", request.command.context_key))
        .execute(&mut *tx)
        .await?;
    // The run-status writer trigger takes this same per-run key *before*
    // touching the canonical status. Serialize operator issuance against a
    // transition that entered BEFORE UPDATE before its first admission
    // existed: checking only for an already-issued admission in that trigger
    // would otherwise leave a writer-first gap. The trigger uses try-locks
    // because its row lock must never deadlock against the receipt FK.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-run-status|{}", request.command.run_id))
        .execute(&mut *tx)
        .await?;
    let run = sqlx::query(
        "SELECT context_key, workflow_run_id, workflow_type, status
         FROM pipeline.execution_runs WHERE run_id = $1",
    )
    .bind(request.command.run_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("canonical PG run not found for operator request")?;
    ensure!(
        run.get::<String, _>("context_key") == request.command.context_key
            && run.get::<String, _>("workflow_run_id") == request.workflow_run_id
            && run.get::<String, _>("workflow_type") == "seo_site_build"
            && matches!(
                run.get::<String, _>("status").as_str(),
                "verifying" | "pending_hitl"
            ),
        "operator request does not match a live canonical PG Truth run"
    );
    // The canonical context lock freezes candidate membership, but raw
    // page/section writers use separate source-ID locks. Share the Activity's
    // stable lock order when computing the issuance snapshot, without ever
    // granting the operator UPDATE/FOR SHARE on source evidence.
    const SOURCE_REFS_SQL: &str = "SELECT DISTINCT s.id AS section_id, s.page_id
         FROM extracted.rule_candidates AS c
         JOIN raw.sections AS s
           ON s.id IN (c.raw_section_id, c.evidence_section_id)
         WHERE c.context_key = $1 ORDER BY s.id, s.page_id";
    let evidence_refs = sqlx::query(SOURCE_REFS_SQL)
        .bind(&request.command.context_key)
        .fetch_all(&mut *tx)
        .await?;
    let mut initial_refs = BTreeSet::new();
    let mut page_ids = BTreeSet::new();
    let mut section_ids = BTreeSet::new();
    for row in evidence_refs {
        let section_id = row.get::<i64, _>("section_id");
        let page_id = row.get::<i64, _>("page_id");
        initial_refs.insert((section_id, page_id));
        page_ids.insert(page_id);
        section_ids.insert(section_id);
    }
    for page_id in page_ids {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|pages|{page_id}"))
            .execute(&mut *tx)
            .await?;
    }
    for section_id in section_ids {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|sections|{section_id}"))
            .execute(&mut *tx)
            .await?;
    }
    // A section can be reparented while we wait for the old source locks.
    // The new parent would not be locked, so fail closed on *any* drift.
    let post_lock_refs = sqlx::query(SOURCE_REFS_SQL)
        .bind(&request.command.context_key)
        .fetch_all(&mut *tx)
        .await?;
    let locked_refs: BTreeSet<(i64, i64)> = post_lock_refs
        .into_iter()
        .map(|row| {
            (
                row.get::<i64, _>("section_id"),
                row.get::<i64, _>("page_id"),
            )
        })
        .collect();
    ensure!(
        locked_refs == initial_refs,
        "raw evidence parent/source changed while awaiting operator snapshot locks"
    );
    let source_hash: String = sqlx::query_scalar(CANDIDATE_SET_HASH_SQL)
        .bind(&request.command.context_key)
        .fetch_one(&mut *tx)
        .await
        .context("compute authoritative scoped evidence fingerprint")?;
    let tenant_ready: bool = sqlx::query_scalar(
        "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
    )
    .bind(&request.command.scope.tenant_id)
    .bind(&request.command.context_key)
    .bind(request.command.run_id)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        tenant_ready,
        "canonical run, Truth sources or existing HITL rows lack authoritative tenant ownership"
    );
    // Only PostgreSQL determines issued_by, issue time, contract fingerprint,
    // exact source hash and expiry. None are accepted from the JSON file.
    // Run permissions intentionally remain SELECT-only. Recheck the canonical
    // run in this INSERT statement's fresh READ COMMITTED snapshot: SELECT
    // FOR SHARE would require an unjustified UPDATE grant on execution_runs.
    // Even if the run changes immediately after issuance, the Activity's
    // authoritative admission/run/status verification rejects the effect.
    let inserted: Uuid = sqlx::query_scalar(
        "INSERT INTO pipeline.pg_effect_v3_admissions
           (admission_id, run_id, tenant_id, context_key, workflow_id,
            workflow_execution_run_id, operation_instance, plan_hash,
            contract_fingerprint, candidate_set_hash,
            v3_expected_execution_key, issued_by, expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$12,current_user,
                clock_timestamp() + ($11::integer * interval '1 second')
         FROM pipeline.execution_runs AS er
         WHERE er.run_id = $2 AND er.context_key = $4
           AND er.workflow_run_id = $6 AND er.workflow_type = 'seo_site_build'
           AND er.status IN ('verifying', 'pending_hitl')
         RETURNING admission_id",
    )
    .bind(request.command.admission_id)
    .bind(request.command.run_id)
    .bind(&request.command.scope.tenant_id)
    .bind(&request.command.context_key)
    .bind(&request.workflow_id)
    .bind(&request.workflow_run_id)
    .bind(&request.command.operation_instance)
    .bind(&request.command.plan_hash)
    .bind(contract_fingerprint()?)
    .bind(source_hash)
    .bind(request.expires_in_seconds as i32)
    .bind(execution_key(
        &request.workflow_id,
        &request.workflow_run_id,
        &request.command,
    )?)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(OperatorResult {
        operation: "issue_admission",
        admission_id: inserted,
        status: "issued",
        operator_role: role.to_owned(),
    })
}

async fn revoke_once(pool: &PgPool, admission_id: Uuid, tenant: &str) -> Result<&'static str> {
    enforce_authoritative_pg_tenant(pool, tenant).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    // Acquire before *any* key-row UPDATE and before receipt read. The
    // Activity takes the same xact lock before its admission/receipt checks.
    // This prevents the race where a receipt commits after a precheck but
    // before operator UPDATE, which could otherwise falsely report revoke.
    let acquired: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("pg-v3-admission|{admission_id}"))
            .fetch_one(&mut *tx)
            .await?;
    if !acquired {
        return Err(RevocationBusy.into());
    }
    let committed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pipeline.pg_effect_v3_receipts r
          JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id = r.admission_id
          WHERE r.admission_id = $1 AND a.tenant_id = $2)",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        !committed,
        "effect already committed; revocation cannot undo it"
    );
    let revoked: Option<Uuid> = sqlx::query_scalar(
        "UPDATE pipeline.pg_effect_v3_admissions
         SET revoked_at = clock_timestamp()
         WHERE admission_id = $1 AND tenant_id = $2 AND revoked_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM pipeline.pg_effect_v3_receipts r
                           WHERE r.admission_id = $1)
         RETURNING admission_id",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_optional(&mut *tx)
    .await?;
    if revoked.is_some() {
        tx.commit().await?;
        return Ok("revoked");
    }
    let existing: Option<bool> = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL
         FROM pipeline.pg_effect_v3_admissions
         WHERE admission_id = $1 AND tenant_id = $2",
    )
    .bind(admission_id)
    .bind(tenant)
    .fetch_optional(&mut *tx)
    .await?;
    match existing {
        Some(true) => Ok("already_revoked"),
        None => bail!("operator admission not found"),
        Some(false) => bail!("operator admission was not revoked"),
    }
}

fn is_revocation_busy(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<RevocationBusy>().is_some() {
        return true;
    }
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<sqlx::Error>())
        .any(|sql| match sql {
            sqlx::Error::Database(database) => database.code().as_deref() == Some("55P03"),
            _ => false,
        })
}

pub async fn revoke_by_id(id: &str) -> Result<()> {
    let admission_id = Uuid::parse_str(id).context("operator revoke requires canonical UUID")?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("operator revoke requires PG_EFFECT_OPERATOR_TENANT_ID")?;
    ensure!(!tenant.trim().is_empty(), "operator tenant cannot be empty");
    let (pool, role) = operator_pool().await?;
    let output = revoke_with_pool(&pool, &role, &tenant, admission_id).await?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

async fn revoke_with_pool(
    pool: &PgPool,
    role: &str,
    tenant: &str,
    admission_id: Uuid,
) -> Result<OperatorResult> {
    ensure!(
        assert_operator_role(pool).await? == role,
        "operator connection identity changed after role verification"
    );
    for attempt in 0..=MAX_REVOCATION_BUSY_RETRIES {
        match revoke_once(pool, admission_id, tenant).await {
            Ok(status) => {
                return Ok(OperatorResult {
                    operation: "revoke_admission",
                    admission_id,
                    status,
                    operator_role: role.to_owned(),
                });
            }
            Err(error) if is_revocation_busy(&error) => {
                if attempt == MAX_REVOCATION_BUSY_RETRIES {
                    bail!("V3 admission busy: effect may be in flight; no revocation confirmed");
                }
                tokio::time::sleep(Duration::from_millis(25u64 << attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
    bail!("V3 admission revocation did not complete")
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::RunScope;

    fn request() -> IssueRequest {
        let id = Uuid::new_v4();
        IssueRequest {
            command: ReconcileTruthReviewsInput {
                admission_id: Uuid::new_v4(),
                run_id: id,
                scope: RunScope {
                    tenant_id: "test-tenant".into(),
                    run_id: id.to_string(),
                },
                context_key: "test-context".into(),
                operation_instance: "review-reconcile".into(),
                plan_hash: "a".repeat(64),
            },
            workflow_id: "pg-review-child".into(),
            workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 60,
        }
    }

    #[test]
    fn operator_issue_contract_is_strict_and_short_lived() {
        assert!(request().validate().is_ok());
        let mut invalid = request();
        invalid.expires_in_seconds = MAX_ADMISSION_SECONDS + 1;
        assert!(invalid.validate().is_err());
        invalid.expires_in_seconds = 0;
        assert!(invalid.validate().is_err());
        invalid.expires_in_seconds = 1;
        invalid.command.scope.run_id = "forged-run".into();
        assert!(invalid.validate().is_err());
        let mut invalid = request();
        invalid.workflow_run_id = "not-a-server-run-uuid".into();
        assert!(invalid.validate().is_err());
        let mut invalid = request();
        invalid.command.plan_hash = "not-a-pinned-plan".into();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn caller_cannot_supply_operator_identity_or_snapshot_digest() {
        let request = request();
        let mut value = serde_json::json!({
            "command": request.command,
            "workflow_id": request.workflow_id,
            "workflow_run_id": request.workflow_run_id,
            "expires_in_seconds": request.expires_in_seconds
        });
        assert!(serde_json::from_value::<IssueRequest>(value.clone()).is_ok());
        value["issued_by"] = serde_json::json!("admin");
        assert!(serde_json::from_value::<IssueRequest>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove("issued_by");
        value["candidate_set_hash"] = serde_json::json!("forged");
        assert!(serde_json::from_value::<IssueRequest>(value).is_err());
    }

    #[test]
    fn forged_cross_tenant_claim_fails_config_guard_but_is_not_rls() {
        assert!(enforce_operator_tenant("tenant-a", "tenant-a").is_ok());
        assert!(enforce_operator_tenant("tenant-a", "tenant-b").is_err());
        assert!(enforce_operator_tenant("", "tenant-a").is_err());
        assert!(enforce_operator_tenant(" tenant-a", " tenant-a").is_err());
    }

    #[test]
    fn revocation_retry_is_only_for_busy_lock_conflict() {
        let busy: anyhow::Error = RevocationBusy.into();
        assert!(is_revocation_busy(&busy));
        assert!(!is_revocation_busy(&anyhow::anyhow!(
            "effect committed; revocation is forbidden"
        )));
    }

    /// Run ONLY against an ephemeral, independently migrated PostgreSQL V3
    /// database with an admin capable of CREATE ROLE. The admission trigger
    /// deliberately forbids deleting fixtures, so this must never run against
    /// any shared/staging/prod database. This proves SQL GRANTs and real
    /// current_user checks rather than relying on fake privilege mocks.
    ///
    /// PG_EFFECT_OPERATOR_E2E_ADMIN_DATABASE_URL=postgres://... \
    ///   cargo test -p temporal_pg_effects --bin temporal_pg_effects \
    ///   live_pg_operator_role_issuance_revocation_and_denials -- --ignored
    #[tokio::test]
    #[ignore = "requires disposable migrated PG database and CREATE ROLE credentials"]
    async fn live_pg_operator_role_issuance_revocation_and_denials() -> Result<()> {
        use std::str::FromStr;

        use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

        let admin_url = env::var("PG_EFFECT_OPERATOR_E2E_ADMIN_DATABASE_URL")
            .context("this ignored test requires an EPHEMERAL migrated PG admin database URL")?;
        let admin = PgPool::connect(&admin_url).await?;
        // No accepted role may have membership in the powerful admin role.
        assert!(assert_operator_role(&admin).await.is_err());

        let role = format!("alegria_operator_e2e_{}", Uuid::new_v4().simple());
        let password = Uuid::new_v4().simple().to_string();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOINHERIT PASSWORD '{password}'"
        )))
        .execute(&admin)
        .await?;
        let grants = [
            format!("GRANT USAGE ON SCHEMA pipeline, extracted, raw TO {role}"),
            format!(
                "GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions, \
                 pipeline.pg_effect_v3_receipts, extracted.rule_candidates, raw.sections, raw.pages \
                 TO {role}"
            ),
            format!(
                "GRANT INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role}"
            ),
        ];
        for grant in grants {
            sqlx::raw_sql(sqlx::AssertSqlSafe(grant))
                .execute(&admin)
                .await?;
        }
        let options = PgConnectOptions::from_str(&admin_url)?
            .username(&role)
            .password(&password);
        let operator = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;
        assert_eq!(assert_operator_role(&operator).await?, role);
        // A superuser connection can SET ROLE to the restricted operator and
        // appear safe under current_user/table privilege checks, then RESET
        // ROLE back to superuser. The session_user equality check must veto.
        let switched_role = role.clone();
        let escalated_session = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |conn, _meta| {
                let switched_role = switched_role.clone();
                Box::pin(async move {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET ROLE {switched_role}")))
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&admin_url)
            .await?;
        assert!(assert_operator_role(&escalated_session).await.is_err());

        let mut accepted = request();
        accepted.command.run_id = Uuid::new_v4();
        accepted.command.scope.run_id = accepted.command.run_id.to_string();
        accepted.command.context_key = format!("operator-e2e-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO pipeline.execution_runs
             (run_id, workflow_run_id, workflow_type, context_key, status)
             VALUES ($1, $2, 'seo_site_build', $3, 'verifying')",
        )
        .bind(accepted.command.run_id)
        .bind(&accepted.workflow_run_id)
        .bind(&accepted.command.context_key)
        .execute(&admin)
        .await?;

        // Bad tenant, different canonical run, and forged Temporal Run ID
        // are each forbidden before an admission row exists.
        let mut forged = accepted.clone();
        forged.command.scope.tenant_id = "other-tenant".into();
        assert!(issue_with_pool(&operator, &role, "test-tenant", &forged)
            .await
            .is_err());
        let mut forged = accepted.clone();
        forged.command.run_id = Uuid::new_v4();
        forged.command.scope.run_id = forged.command.run_id.to_string();
        assert!(issue_with_pool(&operator, &role, "test-tenant", &forged)
            .await
            .is_err());
        let mut forged = accepted.clone();
        forged.workflow_run_id = Uuid::new_v4().to_string();
        assert!(issue_with_pool(&operator, &role, "test-tenant", &forged)
            .await
            .is_err());

        // The exact restricted role can issue; the DB, not input JSON,
        // provides its identity and independently hashes the source.
        let issued = issue_with_pool(&operator, &role, "test-tenant", &accepted).await?;
        assert_eq!(issued.status, "issued");
        assert_eq!(issued.operator_role, role);
        let issuer: String = sqlx::query_scalar(
            "SELECT issued_by FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1",
        )
        .bind(issued.admission_id)
        .fetch_one(&operator)
        .await?;
        assert_eq!(issuer, role);

        // Neither canonical run state nor receipts are writable by operator.
        let denied_run =
            sqlx::query("UPDATE pipeline.execution_runs SET status = 'done' WHERE run_id = $1")
                .bind(accepted.command.run_id)
                .execute(&operator)
                .await
                .expect_err("operator cannot update canonical run");
        assert!(matches!(denied_run, sqlx::Error::Database(ref db)
            if db.code().as_deref() == Some("42501")));
        let denied_receipt =
            sqlx::query("INSERT INTO pipeline.pg_effect_v3_receipts(admission_id) VALUES ($1)")
                .bind(issued.admission_id)
                .execute(&operator)
                .await
                .expect_err("operator cannot forge a receipt");
        assert!(matches!(denied_receipt, sqlx::Error::Database(ref db)
            if db.code().as_deref() == Some("42501")));

        assert!(
            revoke_with_pool(&operator, &role, "other-tenant", issued.admission_id)
                .await
                .is_err()
        );
        assert_eq!(
            revoke_with_pool(&operator, &role, "test-tenant", issued.admission_id)
                .await?
                .status,
            "revoked"
        );
        assert_eq!(
            revoke_with_pool(&operator, &role, "test-tenant", issued.admission_id)
                .await?
                .status,
            "already_revoked"
        );

        // Simulate an already committed receipt using *admin fixture rights*.
        // This tests the revocation CAS only, not an effect execution proof.
        let mut committed = accepted.clone();
        committed.command.admission_id = Uuid::new_v4();
        committed.command.operation_instance = "already-committed-fixture".into();
        issue_with_pool(&operator, &role, "test-tenant", &committed).await?;
        let snapshot: String = sqlx::query_scalar(
            "SELECT candidate_set_hash FROM pipeline.pg_effect_v3_admissions
             WHERE admission_id = $1",
        )
        .bind(committed.command.admission_id)
        .fetch_one(&operator)
        .await?;
        let key = execution_key(
            &committed.workflow_id,
            &committed.workflow_run_id,
            &committed.command,
        )?;
        sqlx::query(
            "INSERT INTO pipeline.pg_effect_v3_receipts
             (admission_id, execution_key, operation_id, operation_revision,
              run_id, context_key, candidate_set_hash, created_review_tasks, open_review_tasks)
             VALUES ($1,$2,'pg.truth.reconcile_candidate_reviews',1,$3,$4,$5,0,0)",
        )
        .bind(committed.command.admission_id)
        .bind(key)
        .bind(committed.command.run_id)
        .bind(&committed.command.context_key)
        .bind(snapshot)
        .execute(&admin)
        .await?;
        assert!(revoke_with_pool(
            &operator,
            &role,
            "test-tenant",
            committed.command.admission_id
        )
        .await
        .is_err());
        Ok(())
    }
}
