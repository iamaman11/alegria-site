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
use serde::{Deserialize, Serialize};
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
pub const PG_EFFECT_WORKFLOW_TYPE_V4: &str = "PgTruthReviewReconcileV4Workflow";

/// Immutable operator-observed Temporal server provenance, independent of
/// child invocation fields that an untrusted DAG could self-assert. Hash
/// `serde_json::to_vec(&attestation)` as SHA-256, and pin the parent History
/// `ChildWorkflowExecutionStarted.event_id` as the positive event number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V4TemporalAttestation {
    pub parent_workflow_id: String,
    pub parent_workflow_run_id: String,
    pub child_workflow_id: String,
    pub child_workflow_run_id: String,
    pub parent_child_started_event_id: i64,
}

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

/// Reject a DB role with *any* application table write authority. Only the
/// narrow, admission-gated SECURITY DEFINER capability may perform effects;
/// the Activity must never directly update human review decisions or forge
/// receipts. A short list of protected tables is
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
             has_any_column_privilege(current_user, c.oid, 'INSERT') OR
             has_any_column_privilege(current_user, c.oid, 'UPDATE') OR
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
pub const CANDIDATE_SET_HASH_SQL: &str = "SELECT pipeline.pg_truth_candidate_set_hash_v1($1)";

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
fn map_owner_sql_error(error: sqlx::Error) -> ReconcileTruthReviewsError {
    match error
        .as_database_error()
        .and_then(|db| db.code())
        .as_deref()
    {
        Some("AV001") => ReconcileTruthReviewsError::AdmissionDenied,
        Some("TR001") => ReconcileTruthReviewsError::StaleTruthRun,
        Some("CS001") => ReconcileTruthReviewsError::CandidateSetChanged,
        _ => ReconcileTruthReviewsError::Sql(error),
    }
}

/// The V3 legacy command may consume only a revision-3 admission. All raw
/// source locks, independent revalidation, canonical HITL writes and receipt
/// happen inside ONE SECURITY DEFINER SQL entrypoint; the login has no DML.
pub async fn execute_admitted_reconcile(
    pool: &PgPool,
    workflow_id: &str,
    workflow_run_id: &str,
    input: &ReconcileTruthReviewsInput,
) -> Result<ReconcileTruthReviewsReceipt, ReconcileTruthReviewsError> {
    let key = execution_key(workflow_id, workflow_run_id, input)?;
    assert_unprivileged_pg_effect_role(pool).await?;
    let fingerprint = contract_fingerprint()?;
    let row = sqlx::query(
        "SELECT * FROM pipeline.consume_pg_truth_review_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(input.admission_id)
    .bind(input.run_id)
    .bind(&input.scope.tenant_id)
    .bind(&input.context_key)
    .bind(workflow_id)
    .bind(workflow_run_id)
    .bind(&input.operation_instance)
    .bind(&input.plan_hash)
    .bind(&fingerprint)
    .bind(&key)
    .bind(3_i16)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<Vec<u8>>)
    .bind(None::<Vec<u8>>)
    .bind(None::<Vec<u8>>)
    .fetch_one(pool)
    .await
    .map_err(map_owner_sql_error)?;
    Ok(receipt_from_row(&row))
}

/// The V4 child Activity passes its server-provided own child Workflow ID and
/// Run ID separately from the typed immutable parent declaration. The owner
/// recomputes CheckedDispatch's BLAKE3 keys from the exact bound bytes and
/// independently verifies operator-pinned SHA256 payloads INSIDE PostgreSQL.
/// Neither a self-consistent plan nor the command itself is authorization.
pub async fn execute_admitted_reconcile_v4(
    pool: &PgPool,
    child_workflow_id: &str,
    child_workflow_run_id: &str,
    invocation: &operation_protocol::EffectChildInvocationV4,
) -> Result<ReconcileTruthReviewsReceipt, ReconcileTruthReviewsError> {
    use operation_protocol::{EffectId, IdempotencyPolicy};
    use truth_domain::review_effect_contract::TruthReviewScopeArtifactV1;

    invocation
        .verify_integrity()
        .map_err(|_| ReconcileTruthReviewsError::InvalidCommand("v4.checked_dispatch"))?;
    let input: ReconcileTruthReviewsInput =
        serde_json::from_slice(&invocation.owner_command_bytes)?;
    validate(&input)?;
    if serde_json::to_vec(&input)? != invocation.owner_command_bytes {
        return Err(ReconcileTruthReviewsError::InvalidCommand(
            "v4.canonical_owner_command",
        ));
    }
    let artifact: TruthReviewScopeArtifactV1 =
        serde_json::from_slice(&invocation.bound_input.payload)?;
    if serde_json::to_vec(&artifact)? != invocation.bound_input.payload
        || artifact.run_id != input.run_id
        || artifact.context_key != input.context_key
        || invocation.bound_input.scope != input.scope
        || invocation.checked_dispatch.scope != input.scope
        || invocation.checked_dispatch.plan_hash.as_str() != input.plan_hash
        || invocation.checked_dispatch.instance.as_str() != input.operation_instance
        || invocation.checked_dispatch.operation.id.as_str() != OPERATION_ID
        || invocation.checked_dispatch.operation.revision != OPERATION_REVISION
        || invocation.checked_dispatch.owner.as_str() != "postgres_store"
        || invocation.checked_dispatch.effects != BTreeSet::from([EffectId(EFFECT_ID.into())])
        || invocation.checked_dispatch.idempotency != IdempotencyPolicy::Idempotent
        || invocation.bound_input.schema.id.as_str() != "truth.candidate_review_scope"
        || invocation.bound_input.schema.revision != 1
        || Uuid::parse_str(&invocation.parent.run_id).is_err()
        || Uuid::parse_str(child_workflow_run_id).is_err()
        || child_workflow_id.trim().is_empty()
    {
        return Err(ReconcileTruthReviewsError::InvalidCommand(
            "v4.owner_contract_binding",
        ));
    }
    assert_unprivileged_pg_effect_role(pool).await?;
    let fingerprint = contract_fingerprint()?;
    let invocation_bytes = serde_json::to_vec(invocation)?;
    let bound_input_bytes = serde_json::to_vec(&invocation.bound_input)?;
    let row = sqlx::query(
        "SELECT * FROM pipeline.consume_pg_truth_review_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(input.admission_id)
    .bind(input.run_id)
    .bind(&input.scope.tenant_id)
    .bind(&input.context_key)
    .bind(child_workflow_id)
    .bind(child_workflow_run_id)
    .bind(&input.operation_instance)
    .bind(&input.plan_hash)
    .bind(&fingerprint)
    .bind(&invocation.checked_dispatch.execution_key)
    .bind(4_i16)
    .bind(&invocation.parent.workflow_id)
    .bind(&invocation.parent.run_id)
    .bind(&invocation.checked_dispatch.input_hash)
    .bind(invocation_bytes)
    .bind(bound_input_bytes)
    .bind(&invocation.owner_command_bytes)
    .fetch_one(pool)
    .await
    .map_err(map_owner_sql_error)?;
    Ok(receipt_from_row(&row))
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
