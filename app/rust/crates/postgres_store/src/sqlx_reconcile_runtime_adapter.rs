use sqlx::PgPool;

use super::sqlx_error::classify_sqlx;
use primitives::errors::DomainError;
use primitives::hash::blake3_hex;
use runtime_models::{reconcile_wire, ReconcileSummary, ReconcileTargetReportRecord};

fn encoded_payload(bytes: Vec<u8>) -> (Vec<u8>, String) {
    let hash = blake3_hex(&bytes);
    (bytes, hash)
}

mod rows {
    #[derive(Debug)]
    pub(super) struct ReconcileRunIdRow {
        pub(super) reconcile_run_id: i64,
    }
}

mod commands {
    use super::rows::ReconcileRunIdRow;
    use sqlx::PgPool;

    pub(super) async fn insert_reconcile_run(
        pool: &PgPool,
        summary_type: &str,
        schema_version: i32,
        summary_bytes: &[u8],
        summary_hash: &str,
    ) -> Result<ReconcileRunIdRow, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            INSERT INTO pipeline.reconcile_runs (status, summary_type, schema_version, summary_bytes, summary_hash)
            VALUES ('running', $1, $2, $3, $4)
            RETURNING reconcile_run_id
            "#,
            summary_type,
            schema_version,
            summary_bytes,
            summary_hash
        )
        .fetch_one(pool)
        .await?;
        Ok(ReconcileRunIdRow {
            reconcile_run_id: row.reconcile_run_id,
        })
    }

    pub(super) async fn insert_reconcile_action(
        pool: &PgPool,
        reconcile_run_id: i64,
        action_type: &str,
        target_system: Option<&str>,
        target_key: Option<&str>,
        details_type: &str,
        schema_version: i32,
        details_bytes: &[u8],
        details_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO pipeline.reconcile_actions
            (reconcile_run_id, action_type, target_system, target_key, details_type, schema_version, details_bytes, details_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            reconcile_run_id,
            action_type,
            target_system,
            target_key,
            details_type,
            schema_version,
            details_bytes,
            details_hash
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn update_reconcile_run(
        pool: &PgPool,
        reconcile_run_id: i64,
        status: &str,
        summary_type: &str,
        schema_version: i32,
        summary_bytes: &[u8],
        summary_hash: &str,
        error_message: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            UPDATE pipeline.reconcile_runs
            SET status = $1,
                summary_type = $2,
                schema_version = $3,
                summary_bytes = $4,
                summary_hash = $5,
                error_message = $6,
                finished_at = now()
            WHERE reconcile_run_id = $7
            "#,
            status,
            summary_type,
            schema_version,
            summary_bytes,
            summary_hash,
            error_message,
            reconcile_run_id
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub async fn begin_reconcile_run(pool: &PgPool) -> std::result::Result<i64, DomainError> {
    let empty = ReconcileSummary::default();
    let (summary_bytes, summary_hash) = encoded_payload(reconcile_wire::encode_summary(&empty));
    let row = commands::insert_reconcile_run(
        pool,
        reconcile_wire::RECONCILE_SUMMARY_PAYLOAD_TYPE,
        reconcile_wire::RECONCILE_SCHEMA_VERSION,
        &summary_bytes,
        &summary_hash,
    )
    .await
    .map_err(classify_sqlx)?;
    Ok(row.reconcile_run_id)
}

pub async fn append_reconcile_target_action(
    pool: &PgPool,
    reconcile_run_id: i64,
    action_type: &str,
    target_system: Option<&str>,
    target_key: Option<&str>,
    details: &ReconcileTargetReportRecord,
) -> std::result::Result<(), DomainError> {
    let (details_bytes, details_hash) =
        encoded_payload(reconcile_wire::encode_target_report(details));
    commands::insert_reconcile_action(
        pool,
        reconcile_run_id,
        action_type,
        target_system,
        target_key,
        reconcile_wire::RECONCILE_TARGET_PAYLOAD_TYPE,
        reconcile_wire::RECONCILE_SCHEMA_VERSION,
        &details_bytes,
        &details_hash,
    )
    .await
    .map_err(classify_sqlx)
}

pub async fn append_reconcile_summary_action(
    pool: &PgPool,
    reconcile_run_id: i64,
    action_type: &str,
    target_system: Option<&str>,
    target_key: Option<&str>,
    details: &ReconcileSummary,
) -> std::result::Result<(), DomainError> {
    let (details_bytes, details_hash) = encoded_payload(reconcile_wire::encode_summary(details));
    commands::insert_reconcile_action(
        pool,
        reconcile_run_id,
        action_type,
        target_system,
        target_key,
        reconcile_wire::RECONCILE_SUMMARY_PAYLOAD_TYPE,
        reconcile_wire::RECONCILE_SCHEMA_VERSION,
        &details_bytes,
        &details_hash,
    )
    .await
    .map_err(classify_sqlx)
}

pub async fn finish_reconcile_run(
    pool: &PgPool,
    reconcile_run_id: i64,
    status: &str,
    summary: &ReconcileSummary,
    error_message: Option<&str>,
) -> std::result::Result<(), DomainError> {
    let (summary_bytes, summary_hash) = encoded_payload(reconcile_wire::encode_summary(summary));
    commands::update_reconcile_run(
        pool,
        reconcile_run_id,
        status,
        reconcile_wire::RECONCILE_SUMMARY_PAYLOAD_TYPE,
        reconcile_wire::RECONCILE_SCHEMA_VERSION,
        &summary_bytes,
        &summary_hash,
        error_message,
    )
    .await
    .map_err(classify_sqlx)
}
