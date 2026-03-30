use sqlx::PgPool;
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, RuntimeProtoPayload,
};
use primitives::errors::DomainError;

mod commands {
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn insert_hitl_decision(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        task_id: i64,
        decision_status: &str,
        decision_type: &str,
        schema_version: i32,
        decision_bytes: &[u8],
        decision_hash: &str,
        resolved_by: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO pipeline.hitl_decisions
            (run_id, step_name, task_id, decision_status, decision_type, schema_version, decision_bytes, decision_hash, resolved_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            run_id,
            step_name,
            task_id,
            decision_status,
            decision_type,
            schema_version,
            decision_bytes,
            decision_hash,
            resolved_by
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub async fn write_hitl_decision_typed<T>(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    task_id: i64,
    decision_status: &str,
    decision_payload: &T,
    resolved_by: Option<&str>,
) -> std::result::Result<(), DomainError>
where
    T: RuntimeProtoPayload,
{
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (decision_bytes, decision_hash) = encode_runtime_payload(decision_payload)?;
    commands::insert_hitl_decision(
        pool,
        uuid,
        step_name,
        task_id,
        decision_status,
        T::payload_type(),
        T::schema_version(),
        &decision_bytes,
        &decision_hash,
        resolved_by,
    )
    .await
    .map_err(classify_sqlx)
}
