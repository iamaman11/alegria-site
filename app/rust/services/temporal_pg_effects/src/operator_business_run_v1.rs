//! Tenant-rooted, independently issued generic business authority for V2 DAG.
//! A private operator-approved intent file and a separate tenant-bound SQL
//! LOGIN authorize the scope/plan before Temporal provider effects. This role
//! cannot issue generic governance, PostgreSQL or Qdrant owner admissions.

use std::{env,path::Path};

use anyhow::{ensure,Context,Result};
use operation_protocol::operation_dag_parent_workflow_id_v2;
use serde::Deserialize;
use sqlx::{PgPool,Row};
use uuid::Uuid;

use super::operator_dag_governance::read_private_operator_file;

#[derive(Debug,Deserialize)]
#[serde(deny_unknown_fields)]
struct BusinessRunIntentV1 {
    run_id: Uuid,
    tenant_id: String,
    context_key: String,
    plan_hash: String,
    parent_workflow_id: String,
    expires_in_seconds: u32,
}

async fn root_issuer_pool() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_BUSINESS_RUN_ISSUER_V1")
            .ok().as_deref()==Some("true"),
        "generic business-run root issuer is not enabled"
    );
    let url=env::var("OPERATION_DAG_BUSINESS_RUN_ISSUER_DATABASE_URL")
        .context("generic business root requires dedicated operator LOGIN")?;
    ensure!(!url.is_empty() && url==url.trim(),"invalid business root DSN");
    let pool=PgPool::connect(&url).await
        .context("connect separately authenticated tenant business-run issuer")?;
    let identity=sqlx::query(
        "SELECT current_user::text effective,session_user::text login,
                r.rolcanlogin login_enabled,
                (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication) elevated,
                EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m
                       WHERE m.member=r.oid) memberships
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user"
    ).fetch_one(&pool).await?;
    ensure!(
        identity.get::<String,_>("effective")==identity.get::<String,_>("login")
            && identity.get::<bool,_>("login_enabled")
            && !identity.get::<bool,_>("elevated")
            && !identity.get::<bool,_>("memberships"),
        "business root issuer must use restricted standalone tenant LOGIN"
    );
    let grants: bool=sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,
                   'pipeline.operation_dag_business_runs_v1','INSERT')
                AND NOT has_table_privilege(current_user,
                   'pipeline.operation_dag_run_governance_v2','INSERT')
                AND NOT has_table_privilege(current_user,
                   'pipeline.pg_vector_projection_admissions_v1','INSERT')"
    ).fetch_one(&pool).await?;
    ensure!(grants,"business issuer cannot mint its own governance/provider effect");
    let foreign_mutation: Option<String>=sqlx::query_scalar(
        "SELECT n.nspname||'.'||c.relname
         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname!='information_schema' AND n.nspname !~ '^pg_'
           AND c.relkind IN('r','p','v','m','f')
           AND (n.nspname,c.relname)<>
               ('pipeline','operation_dag_business_runs_v1')
           AND (has_any_column_privilege(current_user,c.oid,'INSERT')
                OR has_any_column_privilege(current_user,c.oid,'UPDATE')
                OR has_table_privilege(current_user,c.oid,'DELETE')
                OR has_table_privilege(current_user,c.oid,'TRUNCATE'))
         ORDER BY n.nspname,c.relname LIMIT 1"
    ).fetch_optional(&pool).await?;
    ensure!(foreign_mutation.is_none(),
        "root issuer has unrelated provider/platform mutation authority: {foreign_mutation:?}");
    Ok(pool)
}

pub async fn issue_from_file(path:&Path)->Result<()> {
    let bytes=read_private_operator_file(path,"business-run signed/authorized intent")?;
    let intent: BusinessRunIntentV1=serde_json::from_slice(&bytes)
        .context("decode strict business-run intent; no arbitrary commands or SQL")?;
    ensure!(
        !intent.run_id.is_nil()
            && (1..=86400).contains(&intent.expires_in_seconds)
            && intent.tenant_id==intent.tenant_id.trim()
            && !intent.tenant_id.is_empty()
            && intent.context_key==intent.context_key.trim()
            && !intent.context_key.is_empty()
            && intent.plan_hash.len()==64
            && intent.plan_hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && intent.parent_workflow_id
                ==operation_dag_parent_workflow_id_v2(&intent.run_id.to_string()),
        "business root intent must pin exact tenant/context/plan/parent and TTL"
    );
    let expected_tenant=env::var("OPERATION_DAG_BUSINESS_RUN_ISSUER_TENANT_ID")
        .context("business root issuer requires configured policy tenant")?;
    ensure!(expected_tenant==intent.tenant_id,"operator intent tenant is not authenticated tenant");
    let pool=root_issuer_pool().await?;
    let mut tx=pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='5s'").execute(&mut *tx).await?;
    sqlx::query("SET LOCAL statement_timeout='30s'").execute(&mut *tx).await?;
    let inserted=sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES($1,$2,$3,$4,$5,
                clock_timestamp()+($6::integer*interval '1 second'))"
    ).bind(intent.run_id).bind(&intent.tenant_id)
        .bind(&intent.context_key).bind(&intent.plan_hash)
        .bind(&intent.parent_workflow_id)
        .bind(intent.expires_in_seconds as i32)
        .execute(&mut *tx).await?;
    ensure!(inserted.rows_affected()==1,"business root was not uniquely issued");
    tx.commit().await?;
    println!("{}",serde_json::json!({
        "operation":"issue_operation_dag_business_run_v1",
        "status":"issued",
        "run_id":intent.run_id,
        "tenant_id":intent.tenant_id,
        "parent_workflow_id":intent.parent_workflow_id,
        "plan_hash":intent.plan_hash,
    }));
    Ok(())
}
