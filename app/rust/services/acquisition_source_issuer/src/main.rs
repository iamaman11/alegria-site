//! Dedicated, opt-in Acquisition source issuer. No Temporal SDK, worker queue,
//! HTTP crawler, provider command or general DAG Start authority is linked.
//!
//! A separate SQL login is authenticated by the database tenant-role mapping;
//! it may READ an already ingested tenant raw page and INSERT one source proof,
//! but cannot ingest pages, create business roots, or Start a Workflow.
//! External input bytes are derived *here* from the pinned HTML, never
//! accepted from the caller's claimed payload hash or title.

mod private_file;
mod issue_json;

use std::{env, path::Path, time::{SystemTime, UNIX_EPOCH}};

use acquisition::{
    operations::{acquisition_pure_contracts, ClassifySourcePage},
    trusted_source_transform_v1::{
        classify_source_page_payload_from_pinned_html_v1, CapturedHtmlPageV1,
    },
};
use anyhow::{ensure, Context, Result};
use operation_protocol::{
    ArtifactBinding, ArtifactId, Operation, OperationDagRunV6, PortId,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

const ENABLE_ENV: &str = "ALEGRIA_ENABLE_ACQUISITION_SOURCE_ISSUER_V1";
const DSN_FILE_ENV: &str = "ACQUISITION_SOURCE_ISSUER_DATABASE_URL_FILE";
const MAX_ISSUANCE_FUTURE_SECONDS: i64 = 600;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceIssuanceIntentV1 {
    schema_revision: u16,
    run: OperationDagRunV6,
    context_key: String,
    source_page_id: i64,
    expires_at_unix_seconds: i64,
}

struct CheckedSource<'a> {
    artifact_id: &'a ArtifactId,
    record_id: Uuid,
}

fn checked_record_id(provenance: &str) -> Result<Uuid> {
    let value = provenance.strip_prefix("source-record-v1:")
        .context("Acquisition issuer requires an existing source-record identity")?;
    let id = Uuid::parse_str(value).context("source-record identity is not UUID")?;
    ensure!(!id.is_nil() && id.to_string() == value, "source-record UUID must be canonical and nonnil");
    Ok(id)
}

fn validate_source_intent(intent: &SourceIssuanceIntentV1, now: i64) -> Result<CheckedSource<'_>> {
    ensure!(intent.schema_revision == 1, "unsupported source issuance intent revision");
    ensure!(intent.source_page_id > 0, "source requires a persistent positive page ID");
    ensure!(
        !intent.context_key.is_empty() && intent.context_key.len() <= 512
            && intent.context_key == intent.context_key.trim()
            && !intent.context_key.chars().any(char::is_control),
        "source requires one bounded business context"
    );
    ensure!(
        intent.expires_at_unix_seconds > now
            && intent.expires_at_unix_seconds <= now + MAX_ISSUANCE_FUTURE_SECONDS,
        "source issuance must have a finite ten-minute window"
    );
    ensure!(
        Uuid::parse_str(&intent.run.plan.scope.run_id)
            .map(|id| !id.is_nil() && id.to_string() == intent.run.plan.scope.run_id)
            .unwrap_or(false),
        "source issuance requires canonical run UUID"
    );
    ensure!(
        intent.run.plan.nodes.len() == 1
            && intent.run.plan.external_artifacts.len() == 1
            && intent.run.admitted.len() == 1
            && intent.run.pinned_routes.len() == 1
            && intent.run.plan.allowed_effects.is_empty(),
        "source issuer only supports a single pure Acquisition operation"
    );
    let node = &intent.run.plan.nodes[0];
    let installed = ClassifySourcePage::contract();
    // The caller cannot silently replace pinned contracts or present a
    // self-consistent but unsealed plan. The operator independently rechecks
    // these constraints before Temporal Start using its private root catalog.
    intent.run.plan.validate(&acquisition_pure_contracts()?)?;
    ensure!(
        node.operation == installed.reference
            && node.after.is_empty()
            && node.permitted_effects.is_empty()
            && node.inputs.len() == 1
            && intent.run.pinned_routes[0].operation == installed.reference
            && intent.run.pinned_routes[0].owner == installed.owner
            && intent.run.pinned_routes[0].contract_hash == installed.fingerprint()?,
        "source issuer cannot attest any foreign operation or effect"
    );
    let artifact_id = match node.inputs.get(&PortId::from("input")) {
        Some(ArtifactBinding::External { artifact }) => artifact,
        _ => anyhow::bail!("Acquisition input must be an external pinned artifact"),
    };
    let planned = intent.run.plan.external_artifacts.get(artifact_id)
        .context("source external artifact missing from immutable plan")?;
    let admitted = intent.run.admitted.get(artifact_id)
        .context("source artifact missing from immutable admission")?;
    ensure!(
        installed.inputs.get(&PortId::from("input")) == Some(&planned.schema)
            && planned.schema == admitted.schema
            && planned.scope == intent.run.plan.scope
            && admitted.scope == intent.run.plan.scope
            && admitted.provenance == planned.provenance,
        "source artifact does not match native Acquisition typed schema and scope"
    );
    let record_id = checked_record_id(&admitted.provenance)?;
    Ok(CheckedSource { artifact_id, record_id })
}

fn now_unix_seconds() -> Result<i64> {
    Ok(i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())?)
}

async fn restricted_source_pool() -> Result<PgPool> {
    let path = env::var(DSN_FILE_ENV).context("missing private source-issuer SQL credential file")?;
    let bytes = private_file::read_private_file(Path::new(&path))?;
    let raw = String::from_utf8(bytes).context("source issuer DSN is not UTF-8")?;
    let dsn = raw.trim_end_matches(['\r', '\n']);
    ensure!(!dsn.is_empty() && dsn.trim() == dsn, "source issuer DSN invalid");
    let pool = PgPool::connect(dsn).await.context("source issuer SQL connection unavailable")?;
    let role = sqlx::query(
        "SELECT current_user::text effective, session_user::text login,
                r.rolcanlogin login_enabled, r.rolinherit inherits_roles,
                r.rolsuper privileged, r.rolbypassrls bypass,
                r.rolcreaterole role_admin, r.rolcreatedb db_admin,
                r.rolreplication replication,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_external_source_attestations_v1','INSERT') can_issue,
                has_table_privilege(session_user,
                  'pipeline.operation_dag_business_runs_v1','INSERT') can_issue_root,
                has_table_privilege(session_user,
                  'raw.pages','INSERT') can_insert_raw,
                has_table_privilege(session_user,
                  'raw.pages','UPDATE') can_update_raw,
                has_table_privilege(session_user,
                  'raw.pages','DELETE') can_delete_raw,
                has_table_privilege(session_user,
                  'raw.pages','TRUNCATE') can_truncate_raw
         FROM pg_catalog.pg_roles r WHERE r.rolname=current_user",
    ).fetch_one(&pool).await?;
    ensure!(
        role.get::<String, _>("effective") == role.get::<String, _>("login")
            && role.get::<bool, _>("login_enabled")
            && !role.get::<bool, _>("inherits_roles")
            && !role.get::<bool, _>("privileged")
            && !role.get::<bool, _>("bypass")
            && !role.get::<bool, _>("role_admin")
            && !role.get::<bool, _>("db_admin")
            && !role.get::<bool, _>("replication")
            && role.get::<bool, _>("can_issue")
            && !role.get::<bool, _>("can_issue_root")
            && !role.get::<bool, _>("can_insert_raw")
            && !role.get::<bool, _>("can_update_raw")
            && !role.get::<bool, _>("can_delete_raw")
            && !role.get::<bool, _>("can_truncate_raw"),
        "issuer SQL login lacks isolated source-only authority"
    );
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT granted.rolname::text
         FROM pg_catalog.pg_auth_members m
         JOIN pg_catalog.pg_roles granted ON granted.oid=m.roleid
         WHERE m.member=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user)
         LIMIT 1",
    ).fetch_optional(&pool).await?;
    ensure!(membership.is_none(), "source issuer SQL LOGIN must not SET ROLE");
    Ok(pool)
}

async fn issue_source(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &SourceIssuanceIntentV1,
    checked: &CheckedSource<'_>,
) -> Result<String> {
    let tenant: String = sqlx::query_scalar(
        "SELECT pipeline.current_session_tenant_v1()"
    ).fetch_one(&mut **transaction).await?
        .context("source issuer SQL login is not tenant authenticated")?;
    ensure!(
        tenant == intent.run.plan.scope.tenant_id,
        "source issuance tenant differs from database login tenant"
    );
    let row = sqlx::query(
        "SELECT id,url,final_url,content_type,status_code,raw_html,raw_html_bytes,content_hash
         FROM raw.pages WHERE id=$1 AND tenant_id=$2 FOR SHARE",
    )
    .bind(intent.source_page_id)
    .bind(&tenant)
    .fetch_optional(&mut **transaction)
    .await?
    .context("tenant-authenticated immutable raw HTML page unavailable")?;
    let captured = CapturedHtmlPageV1 {
        page_id: row.try_get("id")?,
        url: row.try_get("url")?,
        final_url: row.try_get("final_url")?,
        content_type: row.try_get("content_type")?,
        status_code: row.try_get("status_code")?,
        raw_html: row.try_get("raw_html")?,
        raw_html_bytes: row.try_get("raw_html_bytes")?,
        content_hash: row.try_get("content_hash")?,
    };
    let derived = classify_source_page_payload_from_pinned_html_v1(&captured)
        .context("pinned raw HTML failed strict typed Acquisition derivation")?;
    let admitted = intent.run.admitted.get(checked.artifact_id)
        .context("typed source not admitted")?;
    ensure!(
        admitted.payload == derived.payload,
        "caller-proposed operation bytes differ from independently derived raw HTML"
    );
    let schema = &intent.run.plan.external_artifacts.get(checked.artifact_id)
        .context("planned source schema missing")?.schema;
    let inserted = sqlx::query(
        "INSERT INTO pipeline.operation_dag_external_source_attestations_v1
            (record_id,run_id,tenant_id,context_key,plan_hash,artifact_id,
             artifact_schema,payload_hash,source_page_id,source_page_content_hash,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,to_timestamp($11::double precision))",
    )
    .bind(checked.record_id)
    .bind(Uuid::parse_str(&intent.run.plan.scope.run_id)?)
    .bind(&tenant)
    .bind(&intent.context_key)
    .bind(intent.run.plan.hash.as_str())
    .bind(checked.artifact_id.as_str())
    .bind(sqlx::types::Json(schema))
    .bind(&derived.payload_hash)
    .bind(intent.source_page_id)
    .bind(&derived.raw_content_hash)
    .bind(intent.expires_at_unix_seconds)
    .execute(&mut **transaction)
    .await
    .context("separately credentialed source record issuance refused")?;
    ensure!(inserted.rows_affected() == 1, "issuer inserted no source proof");
    Ok(derived.payload_hash)
}

async fn execute(path: &Path) -> Result<()> {
    ensure!(
        env::var(ENABLE_ENV).as_deref() == Ok("true"),
        "independent Acquisition source issuance is not operator-enabled"
    );
    let bytes = private_file::read_private_file(path)?;
    let intent = issue_json::decode_issuance_intent(&bytes)?;
    let checked = validate_source_intent(&intent, now_unix_seconds()?)?;
    let pool = restricted_source_pool().await?;
    let mut transaction = pool.begin().await?;
    let payload_hash = issue_source(&mut transaction, &intent, &checked).await?;
    transaction.commit().await.context("durable source issuance commit failed")?;
    println!("{}", issue_json::issued_report(
        &intent.run.plan.scope.run_id,
        checked.record_id,
        checked.artifact_id,
        &payload_hash,
    ));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args_os();
    let _program = args.next();
    let command = args.next();
    let target = args.next();
    ensure!(
        command.as_deref() == Some(std::ffi::OsStr::new("--issue-source-v1"))
            && target.is_some() && args.next().is_none(),
        "usage: acquisition_source_issuer --issue-source-v1 /absolute/private/intent.json"
    );
    match execute(Path::new(target.as_ref().expect("target present"))).await {
        Ok(()) => Ok(()),
        Err(_) => {
            // Refuse before any workflow Start. Never reveal credentials,
            // raw pages, private input bytes or SQL error detail in logs.
            println!("{}", issue_json::unconfirmed_report());
            anyhow::bail!("Acquisition source issuer refused or could not confirm commit; see redacted JSON diagnostic")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_record_uuid_is_exact_and_canonical() {
        let valid = "source-record-v1:0e2dddf2-0f45-46ed-89dd-3dce7e8b247d";
        assert_eq!(checked_record_id(valid).unwrap().to_string(), &valid[17..]);
        for invalid in [
            "", "source-record-v1:",
            "source-record-v1:00000000-0000-0000-0000-000000000000",
            "source-record-v1:0E2DDDF2-0F45-46ED-89DD-3DCE7E8B247D",
            "source-record-v1:0e2dddf2-0f45-46ed-89dd-3dce7e8b247d-more",
        ] {
            assert!(checked_record_id(invalid).is_err(), "{invalid}");
        }
    }
}
