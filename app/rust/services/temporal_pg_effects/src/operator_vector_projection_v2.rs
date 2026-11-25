//! Exact V2 parent / V2-scoped child SQL admission for the PostgreSQL vector
//! source owner. V1 business-run admission/Histories remain frozen separately.

use std::{env, fs, path::Path};

use anyhow::{ensure, Context, Result};
use operation_protocol::{operation_dag_parent_workflow_id_v2, Operation};
use postgres_store::vector_projection_owner_v1::PgReadVectorProjectionV1;
use serde::Deserialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::operator_v4::sha256_pg;
use super::operator_vector_projection_v1::{
    attest_from_server_v2, signal_after_commit, IssueRequest, VerifiedHistory,
};

async fn v2_projection_pool() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_PG_VECTOR_PROJECTION_OPERATOR_V2")
            .ok()
            .as_deref()
            == Some("true"),
        "PG V2 projection operator is disabled"
    );
    let url = env::var("PG_VECTOR_PROJECTION_V2_OPERATOR_DATABASE_URL")
        .context("PG V2 projection requires its dedicated operator DSN")?;
    ensure!(
        !url.is_empty() && url == url.trim(),
        "invalid PG V2 projection DSN"
    );
    let pool = PgPool::connect(&url)
        .await
        .context("connect independent PG V2 projection operator login")?;
    let role = sqlx::query(
        "SELECT current_user::text effective,session_user::text login,
                r.rolcanlogin login_enabled,
                (r.rolsuper OR r.rolcreaterole OR r.rolcreatedb
                 OR r.rolreplication OR r.rolbypassrls) elevated,
                EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m
                       WHERE m.member=r.oid) memberships
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&pool)
    .await?;
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("elevated")
            && !role.get::<bool, _>("memberships"),
        "PG V2 projection operator must be a genuine restricted tenant LOGIN"
    );
    let required: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,
                   'pipeline.vector_projection_sources_v1','SELECT')
                AND has_table_privilege(current_user,
                   'pipeline.operation_dag_business_runs_v1','SELECT')
                AND has_table_privilege(current_user,
                   'pipeline.pg_vector_projection_admissions_v1','INSERT')
                AND has_function_privilege(current_user,
                   'pipeline.operation_dag_governance_matches_v2(uuid,text,text,text,text,text,text,boolean)',
                   'EXECUTE')
                AND NOT has_table_privilege(current_user,
                   'pipeline.operation_dag_business_runs_v1','INSERT')
                AND NOT has_table_privilege(current_user,
                   'pipeline.operation_dag_run_governance_v2','INSERT')"
    ).fetch_one(&pool).await?;
    ensure!(
        required,
        "PG V2 operator lacks own admission/neutral governance or can mint root"
    );
    let foreign_mutation: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname||'.'||c.relname
         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname!='information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind IN('r','p','v','m','f')
           AND (n.nspname,c.relname)<>
               ('pipeline','pg_vector_projection_admissions_v1')
           AND (has_any_column_privilege(current_user,c.oid,'INSERT')
                OR has_any_column_privilege(current_user,c.oid,'UPDATE')
                OR has_table_privilege(current_user,c.oid,'DELETE')
                OR has_table_privilege(current_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    ensure!(
        foreign_mutation.is_none(),
        "PG V2 owner issuer has foreign writes: {foreign_mutation:?}"
    );
    Ok(pool)
}

async fn insert_v2_admission(
    pool: &PgPool,
    req: &IssueRequest,
    history: &VerifiedHistory,
    tenant: &str,
) -> Result<()> {
    ensure!(
        history.command.scope.tenant_id == tenant,
        "PG V2 source tenant does not match independent operator policy"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("operation-dag-business|{}", req.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("operation-dag-canonical|{}", req.canonical_run_id))
        .execute(&mut *tx)
        .await?;
    let root = sqlx::query(
        "SELECT tenant_id,context_key,plan_hash,parent_workflow_id,
                expires_at,revoked_at
         FROM pipeline.operation_dag_business_runs_v1 WHERE run_id=$1",
    )
    .bind(req.canonical_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("no independent V2 trusted business-run root")?;
    ensure!(
        root.get::<String, _>("tenant_id") == tenant
            && root.get::<String, _>("context_key") == history.command.context_key
            && root.get::<String, _>("plan_hash") == history.command.plan_hash
            && root.get::<String, _>("parent_workflow_id") == req.parent_workflow_id
            && root
                .get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at")
                .is_none()
            && root.get::<chrono::DateTime<chrono::Utc>, _>("expires_at") > chrono::Utc::now(),
        "V2 source does not match immutable authority plan/scope/parent"
    );
    let parent_sha = sha256_pg(&mut tx, &history.parent_start_bytes).await?;
    let governed: bool = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_governance_matches_v2(
            $1,$2,$3,$4,$5,$6,$7,TRUE)",
    )
    .bind(req.canonical_run_id)
    .bind(tenant)
    .bind(&history.command.context_key)
    .bind(&req.parent_workflow_id)
    .bind(&req.parent_workflow_run_id)
    .bind(&history.command.plan_hash)
    .bind(&parent_sha)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        governed,
        "PG V2 exact parent History/governance missing/expired"
    );
    let source = sqlx::query(
        "SELECT vector FROM pipeline.vector_projection_sources_v1
         WHERE run_id=$1 AND tenant_id=$2 AND context_key=$3 AND keyword_key=$4",
    )
    .bind(req.canonical_run_id)
    .bind(tenant)
    .bind(&history.command.context_key)
    .bind(&history.command.keyword_key)
    .fetch_optional(&mut *tx)
    .await?
    .context("trusted PG precomputed vector source is absent")?;
    let vector: serde_json::Value = source.try_get("vector")?;
    let source_hash: String = sqlx::query_scalar(
        "SELECT encode(public.digest(convert_to(
             $1::text||'|'||$2::text||'|'||$3::text||'|'||
             $4::text||'|'||$5::jsonb::text,'UTF8'),'sha256'),'hex')",
    )
    .bind(tenant)
    .bind(req.canonical_run_id.to_string())
    .bind(&history.command.context_key)
    .bind(&history.command.keyword_key)
    .bind(vector)
    .fetch_one(&mut *tx)
    .await?;
    let invocation_sha = sha256_pg(&mut tx, &history.child_start_bytes).await?;
    let contract_hash = PgReadVectorProjectionV1::contract().fingerprint()?;
    let inserted = sqlx::query(
        "INSERT INTO pipeline.pg_vector_projection_admissions_v1
            (admission_id,run_id,tenant_id,context_key,keyword_key,
             operation_instance,plan_hash,contract_fingerprint,
             dispatch_execution_key,parent_workflow_id,parent_workflow_run_id,
             child_workflow_id,child_workflow_run_id,invocation_sha256,
             generic_parent_start_sha256,source_hash,issued_by,expires_at,
             parent_protocol_revision)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                current_user,clock_timestamp()+($17::integer*interval '1 second'),2
         WHERE pipeline.operation_dag_governance_matches_v2(
                $2,$3,$4,$10,$11,$7,$15,TRUE)",
    )
    .bind(req.admission_id)
    .bind(req.canonical_run_id)
    .bind(tenant)
    .bind(&history.command.context_key)
    .bind(&history.command.keyword_key)
    .bind(&history.command.operation_instance)
    .bind(&history.command.plan_hash)
    .bind(contract_hash.as_str())
    .bind(&history.invocation.checked_dispatch.execution_key)
    .bind(&req.parent_workflow_id)
    .bind(&req.parent_workflow_run_id)
    .bind(&req.child_workflow_id)
    .bind(&req.child_workflow_run_id)
    .bind(invocation_sha)
    .bind(parent_sha)
    .bind(source_hash)
    .bind(req.expires_in_seconds as i32)
    .execute(&mut *tx)
    .await?;
    ensure!(
        inserted.rows_affected() == 1,
        "PG V2 producer admission was not inserted"
    );
    tx.commit().await?;
    println!(
        "{}",
        serde_json::json!({
            "operation":"issue_pg_vector_projection_v2",
            "status":"issued",
            "admission_id":req.admission_id,
            "child_workflow_run_id":req.child_workflow_run_id,
            "dispatch_execution_key":history.invocation.checked_dispatch.execution_key,
        })
    );
    Ok(())
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("read PG V2 projection locator {}", path.display()))?;
    let req: IssueRequest =
        serde_json::from_slice(&bytes).context("decode locator-only PG V2 projection request")?;
    ensure!(
        (1..=300).contains(&req.expires_in_seconds)
            && !req.admission_id.is_nil()
            && !req.canonical_run_id.is_nil()
            && req.parent_workflow_id
                == operation_dag_parent_workflow_id_v2(&req.canonical_run_id.to_string())
            && Uuid::parse_str(&req.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&req.child_workflow_run_id).is_ok()
            && req
                .child_workflow_id
                .starts_with("alegria-owner-operation-v2-"),
        "PG V2 producer locator is not an exact versioned Temporal execution"
    );
    let build_id = env::var("PG_OWNER_EXPECTED_BUILD_ID")
        .context("PG V2 operator requires immutable trusted Build ID")?;
    ensure!(
        build_id.len() == 40 && build_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "PG V2 owner build_id is not a 40-hex source SHA"
    );
    let history = attest_from_server_v2(&req, &build_id).await?;
    let tenant = env::var("PG_EFFECT_OPERATOR_TENANT_ID")
        .context("PG V2 owner operator requires explicit tenant")?;
    let pool = v2_projection_pool().await?;
    insert_v2_admission(&pool, &req, &history, &tenant).await?;
    signal_after_commit(&req).await
}
