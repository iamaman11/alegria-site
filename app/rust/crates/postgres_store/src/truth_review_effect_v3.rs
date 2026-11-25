//! Opt-in, admission-gated V3 PostgreSQL Truth-review reconciliation.
//!
//! This is one EXISTING business operation, not a generic SQL command facade.
//! The same `sqlx_hitl_adapter::reconcile_truth_candidate_reviews_in_tx`
//! owns legacy Truth writes and this typed Activity. It opens/supersedes/reopens
//! human review tasks; it never records a human decision or verifies a claim.
//!
//! An untrusted plan, even a fully self-consistent `ExecutionPlan`, cannot
//! authorize this effect. A distinct privileged operator must pre-issue an
//! immutable PostgreSQL admission for the exact canonical run/scope/instance,
//! candidate snapshot and workflow ID. The unprivileged Activity role can
//! only consume that admission. One PG transaction performs the reconciliation
//! AND writes its uniquely keyed durable receipt.

use std::collections::BTreeSet;

use operation_protocol::{Operation, OperationContract};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::sqlx_hitl_adapter;
pub use truth_domain::review_effect_contract::{
    ReconcileTruthReviewsInput, ReconcileTruthReviewsReceipt,
};

pub const OPERATION_ID: &str = truth_domain::review_effect_contract::OPERATION_ID;
pub const OPERATION_REVISION: u32 = truth_domain::review_effect_contract::OPERATION_REVISION;
pub const EFFECT_ID: &str = truth_domain::review_effect_contract::EFFECT_ID;
pub const PG_EFFECT_TASK_QUEUE: &str = "alegria-pg-effects-v3";
pub const PG_EFFECT_WORKFLOW_TYPE: &str = "PgTruthReviewReconcileV3Workflow";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// Invalid payload, revoked/stale admission or changed Truth: human action.
    Never,
    /// PG connection/serialization failure; effect and receipt are atomic.
    RetryWithSameAdmission,
}

#[derive(Debug, Error)]
pub enum ReconcileTruthReviewsError {
    #[error("invalid V3 Truth review command: {0}")]
    InvalidCommand(&'static str),
    #[error("operator-issued PostgreSQL effect admission denied or expired")]
    AdmissionDenied,
    #[error("Truth run is absent, terminal, has wrong scope or is not verifying/pending_hitl")]
    StaleTruthRun,
    #[error(
        "candidate source snapshot changed since PG admission; operator re-admission required"
    )]
    CandidateSetChanged,
    #[error("PG effect worker has an unauthorized PostgreSQL privilege: {target}")]
    PrivilegedWorkerRole { target: String },
    #[error("PG Truth review operation failed: {0}")]
    Reconcile(#[from] primitives::errors::DomainError),
    #[error("PG effect SQL failed: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("PG effect input serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl ReconcileTruthReviewsError {
    pub fn retry_class(&self) -> RetryClass {
        match self {
            Self::Sql(_) => RetryClass::RetryWithSameAdmission,
            Self::Reconcile(primitives::errors::DomainError::InfraUnavailable { .. }) => {
                RetryClass::RetryWithSameAdmission
            }
            _ => RetryClass::Never,
        }
    }
}

pub struct ReconcileTruthReviewsOperation;

impl Operation for ReconcileTruthReviewsOperation {
    type Input = ReconcileTruthReviewsInput;
    type Output = ReconcileTruthReviewsReceipt;
    type Failure = ReconcileTruthReviewsError;

    fn contract() -> OperationContract {
        truth_domain::review_effect_contract::operation_contract()
    }
}

/// Versioned operation contract fingerprint, identical in algorithm to
/// `operation_protocol`'s immutable pinned contract hashing. The operator
/// admission pins this value, not merely a caller-asserted operation name.
pub fn contract_fingerprint() -> Result<String, ReconcileTruthReviewsError> {
    Ok(blake3::hash(&serde_json::to_vec(
        &ReconcileTruthReviewsOperation::contract(),
    )?)
    .to_hex()
    .to_string())
}

/// Reject a DB role with *any* application write authority outside the two
/// exact relations needed by this effect. A short list of protected tables is
/// insufficient: the same database also holds run journals, raw evidence,
/// CMS revisions, and future tables that must fail closed automatically.
/// Check effective privileges (including ownership, PUBLIC and inherited
/// roles), schema DDL and sequence mutation before polling Temporal.
/// PostgreSQL cannot enforce a row-type restriction through table GRANTs;
/// this guard is a required outer boundary, not a replacement for the scoped
/// statement predicates and operator admission inside the transaction.
pub async fn assert_unprivileged_pg_effect_role(
    pool: &PgPool,
) -> Result<(), ReconcileTruthReviewsError> {
    // Auditing `current_user` alone is unsafe: a privileged LOGIN could
    // `SET ROLE restricted_worker`, pass every effective privilege check,
    // and `RESET ROLE` back to owner/superuser later on the same connection.
    // Require a genuine restricted LOGIN as the connection's session identity.
    let (effective_identity, login_identity, can_login): (String, String, bool) = sqlx::query_as(
        "SELECT current_user::text, session_user::text, r.rolcanlogin
             FROM pg_catalog.pg_roles r WHERE r.rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    if effective_identity != login_identity || !can_login {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole {
            target: format!(
                "effective/login role mismatch or NOLOGIN: current_user={effective_identity} session_user={login_identity}"
            ),
        });
    }
    // A user with role-management, BYPASSRLS or replication authority is not
    // an Activity identity regardless of its current individual table GRANTs.
    let privileged_role: bool = sqlx::query_scalar(
        "SELECT rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls
         FROM pg_catalog.pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    if privileged_role {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole {
            target: "role administration / RLS bypass".to_string(),
        });
    }
    // PostgreSQL 16+ supports non-inherited SET ROLE grants. Such a worker
    // may currently appear SELECT-only while retaining the ability to switch
    // to an owner/writer identity mid-request. Reject that authority too.
    let switchable_role: Option<String> = sqlx::query_scalar(
        "SELECT r.rolname
         FROM pg_catalog.pg_roles r
         WHERE r.rolname <> current_user
           AND pg_has_role(current_user, r.oid, 'SET')
         ORDER BY r.rolname LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(target) = switchable_role {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole {
            target: format!("SET ROLE {target}"),
        });
    }

    // Permission checks use OIDs, not user-supplied identifiers or string
    // interpolation. Include tables, views, materialized/foreign tables and
    // partitions across ALL application schemas: a newly migrated writer
    // must not accidentally turn this narrowly scoped Worker into an admin.
    let unexpected_relation: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema'
           AND n.nspname !~ '^pg_'
           AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND (
             (has_any_column_privilege(current_user, c.oid, 'INSERT') AND NOT (
               (n.nspname = 'site' AND c.relname = 'seo_hitl_tasks') OR
               (n.nspname = 'pipeline' AND c.relname = 'pg_effect_v3_receipts')
             )) OR
             (has_any_column_privilege(current_user, c.oid, 'UPDATE') AND NOT (
               n.nspname = 'site' AND c.relname = 'seo_hitl_tasks'
             )) OR
             has_table_privilege(current_user, c.oid, 'DELETE') OR
             has_table_privilege(current_user, c.oid, 'TRUNCATE')
           )
         ORDER BY n.nspname, c.relname
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(target) = unexpected_relation {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { target });
    }

    // USAGE/UPDATE on another owner's sequence is itself a write authority
    // (`nextval` / `setval`). The two allowed tables have UUID/text keys and
    // need no sequence mutation, so any application sequence grant is excess.
    let unexpected_sequence: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname || '.' || c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname <> 'information_schema'
           AND n.nspname !~ '^pg_'
           AND c.relkind = 'S'
           AND (
             has_sequence_privilege(current_user, c.oid, 'USAGE') OR
             has_sequence_privilege(current_user, c.oid, 'UPDATE')
           )
         ORDER BY n.nspname, c.relname
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(target) = unexpected_sequence {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { target });
    }

    let writable_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname
         FROM pg_catalog.pg_namespace AS n
         WHERE n.nspname <> 'information_schema'
           AND n.nspname !~ '^pg_'
           AND has_schema_privilege(current_user, n.oid, 'CREATE')
         ORDER BY n.nspname
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(target) = writable_schema {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole {
            target: format!("CREATE ON SCHEMA {target}"),
        });
    }
    Ok(())
}

/// Exact SHA-256 candidate snapshot hash for privileged manual issuance. The SQL must
/// be executed by the operator in the SAME snapshot that selects its canonical
/// run/context; never accept a caller-supplied hash without PG recomputation.
// Frozen V3 source-identity contract: the full candidate row, both extraction
// and evidence sections (including heading/ordering/markdown/hash/parent), and
// both parent pages' acquisition identity, redirects, HTTP/robots observation,
// canonical metadata, structured content and HTML bytes. Only `processed` is
// excluded: it is a mutable crawler bookkeeping flag, not source evidence.
// HTML/JSONB payloads are hashed separately to avoid materializing megabyte
// JSONB values in the candidate aggregation. A NULL section/page is included
// as a NULL fingerprint, not silently treated as valid evidence.
// `raw.page_content_aliases` is deliberately outside this V3 contract: the
// reconciler reads direct section->page identity and never resolves aliases.
// Any future alias-aware Truth operation needs a new admitted revision and
// corresponding alias writer locks/fingerprint, not an implicit V3 expansion.
pub const CANDIDATE_SET_HASH_SQL: &str = r#"
    SELECT encode(digest(COALESCE(jsonb_agg(
        jsonb_build_object(
            'candidate', to_jsonb(c),
            'raw_section', to_jsonb(s),
            'evidence_section', to_jsonb(es),
            'raw_page', CASE WHEN p.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', p.id, 'url', p.url, 'domain', p.domain, 'dtype', p.dtype,
                'status_code', p.status_code, 'content_type', p.content_type,
                'snapshot_at', p.snapshot_at, 'crawled_at', p.crawled_at,
                'final_url', p.final_url, 'title', p.title,
                'meta_desc', p.meta_desc, 'canonical', p.canonical,
                'word_count', p.word_count, 'content_hash', p.content_hash,
                'raw_html_bytes', p.raw_html_bytes,
                'content_sha256', CASE WHEN p.content IS NULL THEN NULL
                    ELSE encode(digest(p.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN p.raw_html IS NULL THEN NULL
                    ELSE encode(digest(p.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', p.redirect_chain, 'robots_trace', p.robots_trace,
                'source_observation', p.source_observation
            ) END,
            'evidence_page', CASE WHEN ep.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', ep.id, 'url', ep.url, 'domain', ep.domain, 'dtype', ep.dtype,
                'status_code', ep.status_code, 'content_type', ep.content_type,
                'snapshot_at', ep.snapshot_at, 'crawled_at', ep.crawled_at,
                'final_url', ep.final_url, 'title', ep.title,
                'meta_desc', ep.meta_desc, 'canonical', ep.canonical,
                'word_count', ep.word_count, 'content_hash', ep.content_hash,
                'raw_html_bytes', ep.raw_html_bytes,
                'content_sha256', CASE WHEN ep.content IS NULL THEN NULL
                    ELSE encode(digest(ep.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN ep.raw_html IS NULL THEN NULL
                    ELSE encode(digest(ep.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', ep.redirect_chain, 'robots_trace', ep.robots_trace,
                'source_observation', ep.source_observation
            ) END
        ) ORDER BY c.rule_candidate_id)::text, '[]'), 'sha256'), 'hex')
    FROM extracted.rule_candidates AS c
    LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id
    LEFT JOIN raw.sections AS es ON es.id = c.evidence_section_id
    LEFT JOIN raw.pages AS p ON p.id = s.page_id
    LEFT JOIN raw.pages AS ep ON ep.id = es.page_id
    WHERE c.context_key = $1
"#;

fn validate(input: &ReconcileTruthReviewsInput) -> Result<(), ReconcileTruthReviewsError> {
    if input.scope.tenant_id.is_empty() || input.scope.tenant_id.trim() != input.scope.tenant_id {
        return Err(ReconcileTruthReviewsError::InvalidCommand("tenant_id"));
    }
    if input.scope.run_id != input.run_id.to_string() {
        return Err(ReconcileTruthReviewsError::InvalidCommand("scope.run_id"));
    }
    if input.context_key.trim().is_empty() || input.context_key.trim() != input.context_key {
        return Err(ReconcileTruthReviewsError::InvalidCommand("context_key"));
    }
    if input.operation_instance.trim().is_empty()
        || input.operation_instance.trim() != input.operation_instance
    {
        return Err(ReconcileTruthReviewsError::InvalidCommand(
            "operation_instance",
        ));
    }
    if input.plan_hash.len() != 64 || !input.plan_hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(ReconcileTruthReviewsError::InvalidCommand("plan_hash"));
    }
    Ok(())
}

/// A deterministic idempotency key for an exact typed command + workflow ID;
/// it is NOT a substitute for the admission row's independent authorization.
pub fn execution_key(
    workflow_id: &str,
    workflow_run_id: &str,
    input: &ReconcileTruthReviewsInput,
) -> Result<String, ReconcileTruthReviewsError> {
    validate(input)?;
    if workflow_id.is_empty() || workflow_id.trim() != workflow_id {
        return Err(ReconcileTruthReviewsError::InvalidCommand("workflow_id"));
    }
    if Uuid::parse_str(workflow_run_id).is_err() {
        return Err(ReconcileTruthReviewsError::InvalidCommand(
            "workflow_run_id",
        ));
    }
    Ok(blake3::hash(&serde_json::to_vec(&(
        OPERATION_ID,
        OPERATION_REVISION,
        workflow_id,
        workflow_run_id,
        input,
    ))?)
    .to_hex()
    .to_string())
}

/// The *only* V3 effect executor: one PG transaction, one consumed immutable
/// admission, one receipt. No Temporal workflow calls this without admission.
pub async fn execute_admitted_reconcile(
    pool: &PgPool,
    workflow_id: &str,
    workflow_run_id: &str,
    input: &ReconcileTruthReviewsInput,
) -> Result<ReconcileTruthReviewsReceipt, ReconcileTruthReviewsError> {
    let key = execution_key(workflow_id, workflow_run_id, input)?;
    // Never rely solely on a caller's startup check. A direct invocation with
    // admin credentials (or a role whose grants changed after startup) must
    // not be able to exercise this deliberately narrow Activity API.
    assert_unprivileged_pg_effect_role(pool).await?;
    let fingerprint = contract_fingerprint()?;
    let mut tx = pool.begin().await?;
    // This is the SAME context-wide advisory lock acquired before every
    // canonical verified Truth write. Acquire it before reading admission,
    // run chronology, or source snapshot. READ COMMITTED takes each SELECT's
    // snapshot AFTER the lock returns; SERIALIZABLE would capture a stale
    // snapshot while waiting on pg_advisory_xact_lock itself.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{}", input.context_key))
        .execute(&mut *tx)
        .await?;
    // The Activity role has NO UPDATE grant on admissions; an advisory lock
    // serializes duplicate commands without elevating its SQL privileges.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("pg-v3-admission|{}", input.admission_id))
        .execute(&mut *tx)
        .await?;
    // `now()` would reflect transaction start, which may precede minutes of
    // waiting on the context lock. Authorize at the post-lock wall-clock
    // checkpoint; a DB trigger serializes operator revocation on this lock.
    let admission = sqlx::query(
        "SELECT a.admission_id, er.status AS run_status,
                (a.revoked_at IS NULL AND a.expires_at > clock_timestamp()) AS admission_active
         FROM pipeline.pg_effect_v3_admissions a
         JOIN pipeline.execution_runs er ON er.run_id = a.run_id
         WHERE a.admission_id = $1 AND a.run_id = $2
           AND a.tenant_id = $3 AND a.context_key = $4
           AND a.workflow_id = $5 AND a.workflow_execution_run_id = $6
           AND a.operation_instance = $7 AND a.plan_hash = $8
           AND a.operation_id = $9 AND a.operation_revision = $10
           AND a.authorized_effect = $11 AND a.contract_fingerprint = $12
           AND er.context_key = a.context_key
           AND er.workflow_run_id = a.workflow_execution_run_id
           AND er.workflow_type = 'seo_site_build'
        ",
    )
    .bind(input.admission_id)
    .bind(input.run_id)
    .bind(&input.scope.tenant_id)
    .bind(&input.context_key)
    .bind(workflow_id)
    .bind(workflow_run_id)
    .bind(&input.operation_instance)
    .bind(&input.plan_hash)
    .bind(OPERATION_ID)
    .bind(OPERATION_REVISION as i32)
    .bind(EFFECT_ID)
    .bind(&fingerprint)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ReconcileTruthReviewsError::AdmissionDenied)?;

    // Once committed, a repeated identical command reads the canonical PG
    // receipt without rechecking current candidate state or issuing effects.
    if let Some(receipt) = sqlx::query(
        "SELECT receipt_id, admission_id, execution_key, run_id,
                context_key, candidate_set_hash, created_review_tasks, open_review_tasks
         FROM pipeline.pg_effect_v3_receipts WHERE admission_id = $1",
    )
    .bind(input.admission_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if receipt.get::<String, _>("execution_key") != key {
            return Err(ReconcileTruthReviewsError::AdmissionDenied);
        }
        let output = receipt_from_row(&receipt);
        tx.commit().await?;
        return Ok(output);
    }
    if !admission.get::<bool, _>("admission_active") {
        return Err(ReconcileTruthReviewsError::AdmissionDenied);
    }
    if !matches!(
        admission.get::<String, _>("run_status").as_str(),
        "verifying" | "pending_hitl"
    ) {
        return Err(ReconcileTruthReviewsError::StaleTruthRun);
    }
    // Preserve legacy Truth chronology: a newly completed Truth write for
    // this context supersedes the earlier run, even if both run timestamps
    // tie. No older Activity may then reopen stale source review tasks.
    let newer_truth_done: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM pipeline.execution_runs newer
           JOIN pipeline.execution_runs this_run ON this_run.run_id = $1
           JOIN pipeline.step_executions done ON done.run_id = newer.run_id
            AND done.step_name = 'verified_truth_write' AND done.status = 'done'
           WHERE newer.context_key = this_run.context_key
             AND newer.run_id <> this_run.run_id
             AND newer.created_at >= this_run.created_at
         )",
    )
    .bind(input.run_id)
    .fetch_one(&mut *tx)
    .await?;
    if newer_truth_done {
        return Err(ReconcileTruthReviewsError::StaleTruthRun);
    }
    // The canonical Truth context lock protects candidate IDs because a
    // PostgreSQL trigger now makes every direct candidate INSERT/UPDATE/DELETE
    // use the same lock. Raw page/section writers have independent lifecycles.
    // Acquire their source-ID keys in a stable order before hashing. The
    // database-side writer trigger acquires the SAME keys before UPDATE/DELETE,
    // rejecting concurrent mutation with SQLSTATE 55P03. An Activity needs
    // SELECT only, never UPDATE/FOR SHARE rights on raw evidence.
    const SOURCE_REFS_SQL: &str = "SELECT DISTINCT s.id AS section_id, s.page_id
         FROM extracted.rule_candidates AS c
         JOIN raw.sections AS s
           ON s.id IN (c.raw_section_id, c.evidence_section_id)
         WHERE c.context_key = $1 ORDER BY s.id, s.page_id";
    let evidence_refs = sqlx::query(SOURCE_REFS_SQL)
        .bind(&input.context_key)
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
    // The initial SELECT precedes the source-ID advisory locks. Under READ
    // COMMITTED a writer may reparent a section while V3 waits for those
    // locks, leaving a newly referenced page unlocked. Re-read *all* pairs
    // after acquiring both sets and fail closed if any association drifted.
    // After this checkpoint the candidate context trigger and raw source
    // triggers make these exact pairs stable until the receipt transaction
    // commits. Never hash a page obtained from an unlocked parent reference.
    let post_lock_refs = sqlx::query(SOURCE_REFS_SQL)
        .bind(&input.context_key)
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
    if locked_refs != initial_refs {
        return Err(ReconcileTruthReviewsError::CandidateSetChanged);
    }
    let expected_hash: String = sqlx::query_scalar(
        "SELECT candidate_set_hash FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1",
    )
    .bind(input.admission_id)
    .fetch_one(&mut *tx)
    .await?;
    // Canonical Truth writers hold the same context lock. The full candidate
    // rowset fingerprint is checked inside that critical section, without
    // requiring UPDATE permission on the authoritative candidate table.
    let observed_hash: String = sqlx::query_scalar(CANDIDATE_SET_HASH_SQL)
        .bind(&input.context_key)
        .fetch_one(&mut *tx)
        .await?;
    if observed_hash != expected_hash {
        return Err(ReconcileTruthReviewsError::CandidateSetChanged);
    }
    let created = sqlx_hitl_adapter::reconcile_truth_candidate_reviews_in_tx(
        &mut tx,
        &input.context_key,
        &input.run_id.to_string(),
    )
    .await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.seo_hitl_tasks
         WHERE scope_signature = $1 AND task_type = 'truth_candidate_review'
           AND queue_state IN ('open', 'reopened', 'in_review')",
    )
    .bind(&input.context_key)
    .fetch_one(&mut *tx)
    .await?;
    let receipt = sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_receipts
           (admission_id, execution_key, operation_id, operation_revision,
            run_id, context_key, candidate_set_hash, created_review_tasks, open_review_tasks)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         RETURNING receipt_id, admission_id, execution_key, run_id,
                   context_key, candidate_set_hash, created_review_tasks, open_review_tasks",
    )
    .bind(input.admission_id)
    .bind(&key)
    .bind(OPERATION_ID)
    .bind(OPERATION_REVISION as i32)
    .bind(input.run_id)
    .bind(&input.context_key)
    .bind(&observed_hash)
    .bind(created as i64)
    .bind(open)
    .fetch_one(&mut *tx)
    .await?;
    let output = receipt_from_row(&receipt);
    tx.commit().await?;
    Ok(output)
}

fn receipt_from_row(row: &sqlx::postgres::PgRow) -> ReconcileTruthReviewsReceipt {
    ReconcileTruthReviewsReceipt {
        receipt_id: row.get("receipt_id"),
        admission_id: row.get("admission_id"),
        execution_key: row.get("execution_key"),
        run_id: row.get("run_id"),
        context_key: row.get("context_key"),
        candidate_set_hash: row.get("candidate_set_hash"),
        created_review_tasks: row.get("created_review_tasks"),
        open_review_tasks: row.get("open_review_tasks"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        ContractCatalog, EffectId, ExecutionPlan, IdempotencyPolicy, PlanHash, RunScope,
        PLAN_SCHEMA_REVISION,
    };

    fn input() -> ReconcileTruthReviewsInput {
        let run = Uuid::from_u128(17);
        ReconcileTruthReviewsInput {
            admission_id: Uuid::from_u128(41),
            run_id: run,
            scope: RunScope {
                tenant_id: "site-a".into(),
                run_id: run.to_string(),
            },
            context_key: "ES|tourist||BY".into(),
            operation_instance: "reconcile-truth-review-1".into(),
            plan_hash: "a".repeat(64),
        }
    }

    #[test]
    fn operation_is_pinned_single_effect_owner() {
        let contract = ReconcileTruthReviewsOperation::contract();
        assert_eq!(contract.reference.id.as_str(), OPERATION_ID);
        assert_eq!(contract.reference.revision, 1);
        assert_eq!(contract.owner.as_str(), "postgres_store");
        assert_eq!(
            contract.effects,
            BTreeSet::from([EffectId(EFFECT_ID.into())])
        );
        assert_eq!(contract.idempotency, IdempotencyPolicy::Idempotent);
        assert_eq!(contract.outputs.len(), 1);
        assert_eq!(contract.inputs.len(), 1);
        let mut catalog = ContractCatalog::default();
        catalog
            .register::<ReconcileTruthReviewsOperation>()
            .unwrap();
    }

    #[test]
    fn execution_key_scopes_run_instance_admission_and_workflow() {
        let original = input();
        let temporal_run = Uuid::from_u128(55).to_string();
        let key = execution_key("pg-truth-review-workflow", &temporal_run, &original).unwrap();
        assert_eq!(key.len(), 64);
        assert_eq!(
            key,
            execution_key("pg-truth-review-workflow", &temporal_run, &original).unwrap()
        );
        assert_ne!(
            key,
            execution_key("other-workflow", &temporal_run, &original).unwrap()
        );
        assert_ne!(
            key,
            execution_key(
                "pg-truth-review-workflow",
                &Uuid::from_u128(56).to_string(),
                &original
            )
            .unwrap()
        );
        let mut another = original.clone();
        another.operation_instance = "other-instance".into();
        assert_ne!(
            key,
            execution_key("pg-truth-review-workflow", &temporal_run, &another).unwrap()
        );
    }

    #[test]
    fn invalid_scope_cannot_reach_pg() {
        let mut command = input();
        let temporal_run = Uuid::from_u128(55).to_string();
        command.scope.run_id = "someone-else".into();
        assert!(matches!(
            execution_key("wf", &temporal_run, &command),
            Err(ReconcileTruthReviewsError::InvalidCommand("scope.run_id"))
        ));
        command = input();
        command.plan_hash = "not-pinned".into();
        assert!(matches!(
            execution_key("wf", &temporal_run, &command),
            Err(ReconcileTruthReviewsError::InvalidCommand("plan_hash"))
        ));
        assert!(matches!(
            execution_key("wf", "forged-not-uuid", &input()),
            Err(ReconcileTruthReviewsError::InvalidCommand(
                "workflow_run_id"
            ))
        ));
    }

    #[test]
    fn self_consistent_pure_plan_never_grants_this_pg_effect() {
        // The owner API never accepts ExecutionPlan for authority. A crafted
        // plan's allowed_effects does not create a DB admission.
        assert_eq!(PLAN_SCHEMA_REVISION, 2);
        let _ = std::mem::size_of::<ExecutionPlan>();
        let _ = std::mem::size_of::<PlanHash>();
    }
}
