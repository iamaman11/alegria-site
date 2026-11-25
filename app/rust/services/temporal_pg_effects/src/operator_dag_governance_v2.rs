//! Independent V2 Temporal DAG run governance, with business-run authority
//! issued by a DIFFERENT tenant-authenticated SQL LOGIN. No SEO phase, PG
//! or Qdrant operation is a special case here. V1 issuer remains frozen.

use std::{env, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, OperationDagRunV6,
    OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::operator_dag_governance::{
    prove_real_generic_parent_for_type, read_operator_owned_catalog,
    read_private_operator_file, GovernanceLocator,
};

async fn v2_governance_pool() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V2")
            .ok().as_deref() == Some("true"),
        "V2 native DAG governance issuer is disabled"
    );
    let inline = env::var("OPERATION_DAG_GOVERNANCE_V2_OPERATOR_DATABASE_URL").ok();
    let file = env::var("OPERATION_DAG_GOVERNANCE_V2_OPERATOR_DATABASE_URL_FILE").ok();
    let url = match (inline, file) {
        (Some(_), Some(_)) => bail!("choose only one V2 governance DSN source"),
        (Some(value), None) => value,
        (None, Some(path)) => String::from_utf8(read_private_operator_file(
            Path::new(&path), "V2 governance DSN secret"
        )?)?.trim_end_matches(|c| c == '\r' || c == '\n').to_owned(),
        (None, None) => bail!("V2 governance issuer requires dedicated SQL credentials"),
    };
    ensure!(!url.trim().is_empty() && url.trim() == url, "invalid V2 governance DSN");
    let pool = PgPool::connect(&url).await
        .context("connect distinct V2 governance issuer database LOGIN")?;
    // No hard-coded Qdrant/Neo4j table knowledge is needed for this platform
    // authority. Inspect every application relation grant, including future
    // providers, and allow INSERT only on the V2 governance table.
    let row = sqlx::query(
        "SELECT current_user::text effective, session_user::text login,
                r.rolcanlogin login_enabled,
                r.rolsuper privileged, r.rolbypassrls bypass,
                r.rolcreaterole role_admin, r.rolcreatedb db_admin,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_run_governance_v2','INSERT') can_issue,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_business_runs_v1','INSERT') can_mint_root,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_business_runs_v1','UPDATE') can_revoke_root
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user"
    ).fetch_one(&pool).await?;
    ensure!(
        row.get::<String,_>("effective") == row.get::<String,_>("login")
        && row.get::<bool,_>("login_enabled")
        && !row.get::<bool,_>("privileged")
        && !row.get::<bool,_>("bypass")
        && !row.get::<bool,_>("role_admin")
        && !row.get::<bool,_>("db_admin")
        && row.get::<bool,_>("can_issue")
        && !row.get::<bool,_>("can_mint_root")
        && !row.get::<bool,_>("can_revoke_root"),
        "V2 governance issuer may not mint/revoke its own business-run root"
    );
    let foreign_mutation: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname NOT IN ('information_schema')
           AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r','p','v','m','f')
           AND (n.nspname,c.relname) <>
               ('pipeline','operation_dag_run_governance_v2')
           AND (has_any_column_privilege(current_user,c.oid,'INSERT')
                OR has_any_column_privilege(current_user,c.oid,'UPDATE')
                OR has_table_privilege(current_user,c.oid,'DELETE')
                OR has_table_privilege(current_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1"
    ).fetch_optional(&pool).await?;
    ensure!(
        foreign_mutation.is_none(),
        "V2 governance issuer has unexpected mutation privilege: {foreign_mutation:?}"
    );
    Ok(pool)
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes = read_private_operator_file(path, "V2 governance strict locator")?;
    let req: GovernanceLocator = serde_json::from_slice(&bytes)
        .context("decode locator-only V2 governance request")?;
    ensure!(
        !req.canonical_run_id.is_nil()
            && (1..=3600).contains(&req.expires_in_seconds)
            && req.parent_workflow_id
                == operation_dag_parent_workflow_id_v2(&req.canonical_run_id.to_string())
            && Uuid::parse_str(&req.parent_workflow_run_id).is_ok()
            && !req.context_key.is_empty()
            && req.context_key == req.context_key.trim(),
        "V2 parent locator canonical ID/scope invalid"
    );
    let tenant = env::var("OPERATION_DAG_GOVERNANCE_TENANT_ID")
        .context("V2 governance operator requires exact authenticated tenant")?;
    ensure!(!tenant.is_empty() && tenant == tenant.trim(), "invalid V2 tenant");
    let catalog_file = env::var("OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE")
        .context("V2 governance requires operator-owned trust registry")?;
    let catalog = read_operator_owned_catalog(Path::new(&catalog_file))?;
    // Server History is verified BEFORE touching any issue capability.
    let input_bytes = prove_real_generic_parent_for_type(
        &req,&tenant,&catalog,OPERATION_DAG_PARENT_TYPE_V2,OPERATION_DAG_PARENT_QUEUE_V2
    ).await?;
    let run: OperationDagRunV6 = serde_json::from_slice(&input_bytes)?;
    let pool = v2_governance_pool().await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='5s'").execute(&mut *tx).await?;
    sqlx::query("SET LOCAL statement_timeout='30s'").execute(&mut *tx).await?;
    // Root issuer revocation, governance issuance and owner first-start share
    // business then canonical lock order and the post-lock READ COMMITTED view.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("operation-dag-business|{}",req.canonical_run_id))
        .execute(&mut *tx).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("operation-dag-canonical|{}",req.canonical_run_id))
        .execute(&mut *tx).await?;
    let root = sqlx::query(
        "SELECT context_key,tenant_id,plan_hash,parent_workflow_id,
                revoked_at,expires_at
         FROM pipeline.operation_dag_business_runs_v1
         WHERE run_id=$1"
    ).bind(req.canonical_run_id)
        .fetch_optional(&mut *tx).await?
        .context("independent trusted V2 business-run root was not issued")?;
    ensure!(
        root.get::<String,_>("context_key") == req.context_key
            && root.get::<String,_>("tenant_id") == tenant
            && root.get::<String,_>("plan_hash") == run.plan.hash.as_str()
            && root.get::<String,_>("parent_workflow_id") == req.parent_workflow_id
            && root.get::<Option<chrono::DateTime<chrono::Utc>>,_>("revoked_at").is_none()
            && root.get::<chrono::DateTime<chrono::Utc>,_>("expires_at") > chrono::Utc::now(),
        "V2 business root does not authorize this plan/parent/tenant or was revoked"
    );
    let sha: String = sqlx::query_scalar(
        "SELECT encode(public.digest($1::bytea,'sha256'),'hex')"
    ).bind(&input_bytes).fetch_one(&mut *tx).await?;
    let inserted = sqlx::query(
        "INSERT INTO pipeline.operation_dag_run_governance_v2
          (canonical_run_id,tenant_id,context_key,parent_workflow_id,
           parent_workflow_run_id,parent_workflow_type,parent_task_queue,
           plan_hash,parent_start_sha256,expires_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,
                clock_timestamp()+($10::integer*interval '1 second'))"
    )
    .bind(req.canonical_run_id)
    .bind(&tenant)
    .bind(&req.context_key)
    .bind(&req.parent_workflow_id)
    .bind(&req.parent_workflow_run_id)
    .bind(OPERATION_DAG_PARENT_TYPE_V2)
    .bind(OPERATION_DAG_PARENT_QUEUE_V2)
    .bind(run.plan.hash.as_str())
    .bind(&sha)
    .bind(req.expires_in_seconds as i32)
    .execute(&mut *tx).await?;
    ensure!(inserted.rows_affected()==1,"V2 governance admission was not inserted");
    tx.commit().await?;
    println!("{}",serde_json::json!({
        "operation":"admit_operation_dag_run_v2",
        "status":"issued",
        "business_run_id":req.canonical_run_id,
        "parent_workflow_run_id":req.parent_workflow_run_id,
        "plan_hash":run.plan.hash.as_str(),
        "parent_start_sha256":sha,
    }));
    Ok(())
}
