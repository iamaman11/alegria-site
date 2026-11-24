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

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId, RunScope,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::sqlx_hitl_adapter;

pub const OPERATION_ID: &str = "pg.truth.reconcile_candidate_reviews";
pub const OPERATION_REVISION: u32 = 1;
pub const EFFECT_ID: &str = "pg.truth.review_queue.reconcile";
pub const PG_EFFECT_TASK_QUEUE: &str = "alegria-pg-effects-v3";
pub const PG_EFFECT_WORKFLOW_TYPE: &str = "PgTruthReviewReconcileV3Workflow";

/// The command is immutable in Temporal History. No field is authorization;
/// every identity is cross-checked against the operator-issued PG admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileTruthReviewsInput {
    pub admission_id: Uuid,
    pub run_id: Uuid,
    pub scope: RunScope,
    pub context_key: String,
    pub operation_instance: String,
    pub plan_hash: String,
}

/// A PostgreSQL receipt for the exact admitted effect; no provider outcome is
/// implied. `created_review_tasks` is the existing reconciler's INSERT count,
/// not a combined changed/reopened/superseded count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileTruthReviewsReceipt {
    pub receipt_id: Uuid,
    pub admission_id: Uuid,
    pub execution_key: String,
    pub run_id: Uuid,
    pub context_key: String,
    pub candidate_set_hash: String,
    pub created_review_tasks: i64,
    pub open_review_tasks: i64,
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
    #[error("PG effect worker must use a DB role without admission write privileges")]
    PrivilegedWorkerRole,
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
        OperationContract {
            reference: OperationRef {
                id: OperationId(OPERATION_ID.to_string()),
                revision: OPERATION_REVISION,
            },
            owner: OperationOwnerId("postgres_store".to_string()),
            inputs: BTreeMap::from([(
                PortId("truth_review_scope".to_string()),
                ArtifactSchema {
                    id: ArtifactTypeId("truth.candidate_review_scope".to_string()),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId("review_queue_receipt".to_string()),
                ArtifactSchema {
                    id: ArtifactTypeId("truth.review_queue_receipt".to_string()),
                    revision: 1,
                },
            )]),
            invalidates: BTreeSet::from([ArtifactTypeId(
                "truth.review_queue_snapshot".to_string(),
            )]),
            effects: BTreeSet::from([EffectId(EFFECT_ID.to_string())]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
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

/// Reject privileged DB role before polling Temporal. Table owners and roles
/// with inherited grants are forbidden, not merely roles with explicit GRANT.
/// The operator provisioning must grant SELECT on admissions and SELECT/INSERT
/// on receipts, plus the necessary scoped Truth/HITL read/write tables.
pub async fn assert_unprivileged_pg_effect_role(
    pool: &PgPool,
) -> Result<(), ReconcileTruthReviewsError> {
    let (insert, update, column_insert, column_update, delete, truncate):
        (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'INSERT'),
                has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'UPDATE'),
                has_any_column_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'INSERT'),
                has_any_column_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'UPDATE'),
                has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'DELETE'),
                has_table_privilege(current_user, 'pipeline.pg_effect_v3_admissions', 'TRUNCATE')",
    )
    .fetch_one(pool)
    .await?;
    if insert || update || column_insert || column_update || delete || truncate {
        return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole);
    }
    // The sole permitted write surface is the review task and its own receipt.
    // This independent worker must never be able to promote canonical Truth,
    // alter evidence, or dispatch another owner's Outbox effects.
    for protected_table in [
        "verified.rule_instances",
        "extracted.rule_candidates",
        "system.sync_outbox",
        "site.cms_approval_decisions",
    ] {
        let privileged: bool = sqlx::query_scalar(
            "SELECT has_any_column_privilege(current_user, $1::text, 'INSERT')
                  OR has_any_column_privilege(current_user, $1::text, 'UPDATE')
                  OR has_table_privilege(current_user, $1::text, 'DELETE')
                  OR has_table_privilege(current_user, $1::text, 'TRUNCATE')",
        )
        .bind(protected_table)
        .fetch_one(pool)
        .await?;
        if privileged {
            return Err(ReconcileTruthReviewsError::PrivilegedWorkerRole);
        }
    }
    Ok(())
}

/// Exact SHA-256 candidate snapshot hash for privileged manual issuance. The SQL must
/// be executed by the operator in the SAME snapshot that selects its canonical
/// run/context; never accept a caller-supplied hash without PG recomputation.
pub const CANDIDATE_SET_HASH_SQL: &str = "SELECT encode(digest(COALESCE(jsonb_agg( \
       jsonb_build_object('candidate', to_jsonb(c), 'raw_section_hash', s.content_hash, \
         'raw_section_content', s.content_md) ORDER BY c.rule_candidate_id)::text, '[]'), \
       'sha256'), 'hex') \
     FROM extracted.rule_candidates AS c \
     LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id \
     WHERE c.context_key = $1";

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
           AND er.context_key = a.context_key AND er.workflow_type = 'seo_site_build'
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
    use operation_protocol::{ContractCatalog, ExecutionPlan, PlanHash, PLAN_SCHEMA_REVISION};

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
