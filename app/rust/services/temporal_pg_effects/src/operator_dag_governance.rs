//! Platform-scoped, separately credentialed issuance of a generic native DAG
//! run. Housed in the PG operator binary only as an opt-in pilot command: it
//! does NOT use PostgreSQL effect-owner credentials, decode PG/Qdrant commands
//! or authorize any provider Activity. Neither a submitted plan nor a
//! Temporal Workflow Type string is a bearer capability.

use std::{collections::BTreeSet, env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id, OperationContract, OperationDagRunV6,
    PinnedOwnerRouteV1, TrustedRouteCatalog, OPERATION_DAG_PARENT_QUEUE,
    OPERATION_DAG_PARENT_TYPE,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::operator_v4::{fetch_exact_history, first_started_payload};

const MAX_GOVERNANCE_SECONDS: u32 = 3_600;

fn read_private_operator_file(path: &Path, purpose: &str) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "{purpose} path must be absolute");
    ensure!(
        path.components().all(|component| !matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "{purpose} path cannot traverse untrusted relative components"
    );
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("stat private {purpose} {}",path.display()))?;
    ensure!(meta.file_type().is_file(),
        "{purpose} must be regular, never symlink");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(meta.permissions().mode() & 0o077 == 0,
            "{purpose} file must be private (0600/0400)");
        let file_owner = meta.uid();
        for ancestor in path.ancestors().skip(1) {
            let parent_meta = fs::symlink_metadata(ancestor)
                .with_context(|| format!("stat private {purpose} parent {}",ancestor.display()))?;
            ensure!(
                parent_meta.file_type().is_dir()
                    && parent_meta.permissions().mode() & 0o022 == 0
                    && (parent_meta.uid() == 0 || parent_meta.uid() == file_owner),
                "{purpose} parent is symlinked, writable by group/world or unowned"
            );
        }
    }
    ensure!(meta.len() <= 2 * 1024 * 1024,
        "{purpose} private file exceeds limit");
    fs::read(path).with_context(|| format!("read private {purpose} {}",path.display()))
}

pub(super) fn validate_local_temporal_endpoint(value: &str) -> Result<()> {
    let endpoint = url::Url::parse(value)
        .context("parse independently configured governance Temporal URL")?;
    ensure!(
        endpoint.scheme() == "http"
            && matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost"))
            && endpoint.port().is_some()
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.path() == "/"
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "generic governance issuer requires exact local HTTP Temporal host/port without userinfo, path, query or fragment"
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GovernanceLocator {
    canonical_run_id: Uuid,
    context_key: String,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    expires_in_seconds: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OperatorInstalledCatalog {
    contracts: Vec<OperationContract>,
    routes: Vec<PinnedOwnerRouteV1>,
}

fn read_operator_owned_catalog(path: &Path) -> Result<TrustedRouteCatalog> {
    let bytes = read_private_operator_file(path, "installed governance catalog")?;
    let installed: OperatorInstalledCatalog = serde_json::from_slice(&bytes)
        .context("decode exact provider-neutral operator trust catalog")?;
    ensure!(serde_json::to_vec(&installed)? == bytes,
        "operator trust catalog is not canonical versioned JSON");
    let mut catalog = TrustedRouteCatalog::default();
    let mut routes = installed.routes;
    routes.sort_by(|a,b| a.operation.cmp(&b.operation));
    ensure!(routes.windows(2).all(|pair| pair[0].operation != pair[1].operation),
        "trusted owner routing catalog duplicated an operation");
    for contract in installed.contracts {
        let position = routes.iter().position(|route| route.operation == contract.reference)
            .context("operator-installed contract has no matching static owner route")?;
        catalog.register(contract, routes.remove(position))
            .context("operator-installed owner contract fingerprint/type/queue invalid")?;
    }
    ensure!(routes.is_empty(), "operator-installed owner route has no versioned contract");
    Ok(catalog)
}

async fn prove_real_generic_parent(
    request: &GovernanceLocator,
    tenant: &str,
    catalog: &TrustedRouteCatalog,
) -> Result<Vec<u8>> {
    let url = env::var("OPERATION_DAG_GOVERNANCE_TEMPORAL_URL")
        .context("governance issuer requires separately configured Temporal endpoint")?;
    let namespace = env::var("OPERATION_DAG_GOVERNANCE_TEMPORAL_NAMESPACE")
        .context("governance issuer requires separately configured namespace")?;
    validate_local_temporal_endpoint(&url)?;
    ensure!(namespace == namespace.trim() && !namespace.is_empty(),
        "invalid independently configured Temporal governance namespace");
    let client = temporal_gateway::connect_client(
        &url, format!("alegria-platform-dag-governance-{}", std::process::id()),
        &namespace,
    ).await.context("connect independent Temporal governance proof client")?;
    let history = fetch_exact_history(
        &client, &namespace,
        &request.parent_workflow_id, &request.parent_workflow_run_id,
    ).await?;
    let bytes = first_started_payload(
        &history, OPERATION_DAG_PARENT_TYPE, OPERATION_DAG_PARENT_QUEUE)?;
    let run: OperationDagRunV6 = serde_json::from_slice(&bytes)
        .context("real Temporal parent History lacks typed OperationDagRunV6")?;
    ensure!(
        run.schema_revision == 6
            && serde_json::to_vec(&run)? == bytes
            && run.plan.scope.run_id == request.canonical_run_id.to_string()
            && run.plan.scope.tenant_id == tenant,
        "Temporal start input schema, scope or canonical bytes invalid"
    );
    run.plan.validate(catalog.catalog())
        .context("native Temporal parent plan is not validated against installed contracts")?;
    catalog.validate_run_routes(&run)
        .context("real Temporal parent pinned routes differ from installed trust catalog")?;
    let node_instances: BTreeSet<_> =
        run.plan.nodes.iter().map(|node| node.instance.clone()).collect();
    ensure!(
        run.owner_commands.keys().cloned().collect::<BTreeSet<_>>() == node_instances,
        "generic parent command map must match all planned effect nodes"
    );
    for (instance, command) in &run.owner_commands {
        let node = run.plan.nodes.iter().find(|node| &node.instance == instance)
            .context("generic owner command refers to absent DAG node")?;
        let contract = catalog.catalog().contract(&command.operation)
            .context("generic owner command operation not installed")?;
        ensure!(
            command.operation == node.operation
                && command.owner == contract.owner
                && command.contract_hash == contract.fingerprint()?
                && !command.command_bytes.is_empty(),
            "parent start owner command not pinned to trusted static typed operation"
        );
    }
    Ok(bytes)
}

async fn governance_pool() -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V1")
            .ok().as_deref() == Some("true"),
        "operator-issued generic run governance is explicitly disabled"
    );
    let inline = env::var("OPERATION_DAG_GOVERNANCE_OPERATOR_DATABASE_URL").ok();
    let secret_file = env::var("OPERATION_DAG_GOVERNANCE_OPERATOR_DATABASE_URL_FILE").ok();
    let url = match (inline,secret_file) {
        (Some(_),Some(_)) => bail!(
            "configure exactly one governance DSN source, not both"),
        (Some(url),None) => url,
        (None,Some(path)) => String::from_utf8(read_private_operator_file(
            Path::new(&path), "governance operator DSN secret")?
        )?.trim_end_matches(|c| c == '\r' || c == '\n').to_owned(),
        (None,None) => bail!(
            "separate governance DSN (_FILE or inline) required; no PG effect fallback"),
    };
    ensure!(!url.trim().is_empty() && url == url.trim(),
        "governance operator DSN invalid");
    let pool = PgPool::connect(&url).await
        .context("connect dedicated generic run governance PostgreSQL LOGIN")?;
    let row = sqlx::query(
        "SELECT current_user::text AS effective,session_user::text AS login,
                r.rolcanlogin AS can_login,r.rolsuper AS super,
                r.rolbypassrls AS bypass,
                has_table_privilege(session_user,
                    'pipeline.operation_dag_run_governance_v1','INSERT') AS can_issue,
                has_table_privilege(session_user,
                    'pipeline.pg_effect_v3_admissions','INSERT') AS can_pg_issue,
                has_table_privilege(session_user,
                    'pipeline.qdrant_projection_v1_admissions','INSERT') AS can_qdrant_v1_issue,
                has_table_privilege(session_user,
                    'pipeline.qdrant_projection_v2_admissions','INSERT') AS can_qdrant_v2_issue,
                has_table_privilege(session_user,
                    'pipeline.qdrant_projection_v3_admissions','INSERT') AS can_qdrant_v3_issue
         FROM pg_roles r WHERE r.rolname = current_user",
    ).fetch_one(&pool).await?;
    ensure!(
        row.get::<String,_>("effective") == row.get::<String,_>("login")
            && row.get::<bool,_>("can_login")
            && !row.get::<bool,_>("super")
            && !row.get::<bool,_>("bypass")
            && row.get::<bool,_>("can_issue")
            && !row.get::<bool,_>("can_pg_issue")
            && !row.get::<bool,_>("can_qdrant_v1_issue")
            && !row.get::<bool,_>("can_qdrant_v2_issue")
            && !row.get::<bool,_>("can_qdrant_v3_issue"),
        "governance issuer LOGIN must have ONLY separate generic admission write authority"
    );
    Ok(pool)
}

pub async fn issue_from_file(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!(
        "read strict generic run governance locator {}", path.display()))?;
    let req: GovernanceLocator = serde_json::from_slice(&bytes)
        .context("decode generic run governance locator; no supplied credentials or SHA")?;
    ensure!(
        !req.canonical_run_id.is_nil()
            && (1..=MAX_GOVERNANCE_SECONDS).contains(&req.expires_in_seconds)
            && req.parent_workflow_id
                == operation_dag_parent_workflow_id(&req.canonical_run_id.to_string())
            && Uuid::parse_str(&req.parent_workflow_run_id).is_ok()
            && !req.context_key.trim().is_empty()
            && req.context_key == req.context_key.trim(),
        "generic run governance locator has invalid canonical scope or Temporal identity"
    );
    let tenant = env::var("OPERATION_DAG_GOVERNANCE_TENANT_ID")
        .context("governance issuer requires configured authoritative tenant")?;
    ensure!(!tenant.trim().is_empty() && tenant == tenant.trim(),
        "invalid independent governance tenant policy");
    let catalog_path = env::var("OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE")
        .context("governance issuer requires independently installed owner contract/route catalog")?;
    let catalog = read_operator_owned_catalog(Path::new(&catalog_path))?;
    // IMPORTANT: do the independent Temporal server proof before obtaining
    // any PostgreSQL capability or reserving a governance identity.
    let parent_start_bytes = prove_real_generic_parent(&req, &tenant, &catalog).await?;
    let run: OperationDagRunV6 = serde_json::from_slice(&parent_start_bytes)?;
    let pool = governance_pool().await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout='5s'").execute(&mut *tx).await?;
    sqlx::query("SET LOCAL statement_timeout='30s'").execute(&mut *tx).await?;
    // Operator issuance/owner first-start/revocation share this exact lock
    // order and cannot cross an active governance decision.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-run-status|{}", req.canonical_run_id))
        .execute(&mut *tx).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("operation-dag-canonical|{}", req.canonical_run_id))
        .execute(&mut *tx).await?;
    let source_run: Option<String> = sqlx::query_scalar(
        "SELECT pipeline.operation_dag_canonical_source_run_id_v1($1,$2,$3)")
        .bind(req.canonical_run_id)
        .bind(&tenant)
        .bind(&req.context_key)
        .fetch_one(&mut *tx).await?;
    let source_run = source_run.context(
        "canonical site-build source absent, terminal or not owned by configured tenant")?;
    ensure!(source_run != req.parent_workflow_run_id,
        "generic parent must be different from old canonical site-build Workflow run");
    let sha: String = sqlx::query_scalar(
        "SELECT encode(public.digest($1::bytea,'sha256'),'hex')")
        .bind(&parent_start_bytes)
        .fetch_one(&mut *tx).await?;
    let inserted = sqlx::query(
        "INSERT INTO pipeline.operation_dag_run_governance_v1
            (canonical_run_id,tenant_id,context_key,parent_workflow_id,
             parent_workflow_run_id,parent_workflow_type,parent_task_queue,
             plan_hash,parent_start_sha256,canonical_source_workflow_run_id,
             expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
             clock_timestamp() + ($11::integer * interval '1 second'))")
        .bind(req.canonical_run_id)
        .bind(&tenant)
        .bind(&req.context_key)
        .bind(&req.parent_workflow_id)
        .bind(&req.parent_workflow_run_id)
        .bind(OPERATION_DAG_PARENT_TYPE)
        .bind(OPERATION_DAG_PARENT_QUEUE)
        .bind(run.plan.hash.as_str())
        .bind(&sha)
        .bind(&source_run)
        .bind(req.expires_in_seconds as i32)
        .execute(&mut *tx).await?;
    ensure!(inserted.rows_affected() == 1,
        "generic run governance was not uniquely inserted");
    tx.commit().await?;
    println!("{}",serde_json::json!({
        "operation":"admit_operation_dag_run_v1",
        "status":"issued",
        "canonical_run_id":req.canonical_run_id,
        "parent_workflow_id":req.parent_workflow_id,
        "parent_workflow_run_id":req.parent_workflow_run_id,
        "plan_hash":run.plan.hash.as_str(),
        "parent_start_sha256":sha
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_business_workflow_id_is_not_a_generic_parent() {
        let id = Uuid::new_v4();
        assert_ne!(id.to_string(),operation_dag_parent_workflow_id(&id.to_string()));
    }

    #[test]
    fn local_temporal_endpoint_rejects_userinfo_host_spoof_and_extra_components() {
        assert!(validate_local_temporal_endpoint("http://127.0.0.1:7233").is_ok());
        assert!(validate_local_temporal_endpoint("http://localhost:7233").is_ok());
        for spoof in [
            "http://127.0.0.1:pw@evil.example:7233",
            "http://user@127.0.0.1:7233",
            "http://localhost:7233/other",
            "http://localhost:7233?token=x",
            "http://localhost:7233#f",
            "https://localhost:7233",
            "http://localhost",
        ] {
            assert!(validate_local_temporal_endpoint(spoof).is_err(),
                "endpoint must fail closed: {spoof}");
        }
    }
}
