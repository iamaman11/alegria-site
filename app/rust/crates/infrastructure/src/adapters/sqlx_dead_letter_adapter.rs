use sqlx::PgPool;
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, RuntimeProtoPayload,
};
use primitives::errors::{DomainError, ErrorClass};

mod commands {
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn insert_dead_letter(
        pool: &PgPool,
        run_id: Option<Uuid>,
        step_name: &str,
        workflow_id: &str,
        error_class: &str,
        payload_type: &str,
        schema_version: i32,
        payload_bytes: &[u8],
        payload_hash: &str,
        input_hash: &str,
        idempotency_key: &str,
        build_id: &str,
        error_message: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO system.dead_letter_queue
            (run_id, step_name, workflow_id, error_class, retryable, payload_type, schema_version,
             payload_bytes, payload_hash, input_hash, idempotency_key, build_id, error_message)
            VALUES ($1, $2, $3, $4, false, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
            run_id,
            step_name,
            workflow_id,
            error_class,
            payload_type,
            schema_version,
            payload_bytes,
            payload_hash,
            input_hash,
            idempotency_key,
            build_id,
            error_message
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub async fn write_dead_letter(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    workflow_id: &str,
    error_class: ErrorClass,
    payload_type: &str,
    schema_version: i32,
    payload_bytes: &[u8],
    payload_hash: &str,
    input_hash: &str,
    idempotency_key: &str,
    build_id: &str,
    error_message: &str,
) -> std::result::Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    commands::insert_dead_letter(
        pool,
        Some(uuid),
        step_name,
        workflow_id,
        error_class.as_str(),
        payload_type,
        schema_version,
        payload_bytes,
        payload_hash,
        input_hash,
        idempotency_key,
        build_id,
        error_message,
    )
    .await
    .map_err(classify_sqlx)
}

pub async fn write_dead_letter_typed<T>(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    workflow_id: &str,
    error_class: ErrorClass,
    payload: &T,
    input_hash: &str,
    idempotency_key: &str,
    build_id: &str,
    error_message: &str,
) -> std::result::Result<(), DomainError>
where
    T: RuntimeProtoPayload,
{
    let (payload_bytes, payload_hash) = encode_runtime_payload(payload)?;
    write_dead_letter(
        pool,
        run_id,
        step_name,
        workflow_id,
        error_class,
        T::payload_type(),
        T::schema_version(),
        &payload_bytes,
        &payload_hash,
        input_hash,
        idempotency_key,
        build_id,
        error_message,
    )
    .await
}

pub async fn write_external_dead_letter(
    pool: &PgPool,
    step_name: &str,
    workflow_id: &str,
    error_class: ErrorClass,
    payload_type: &str,
    schema_version: i32,
    payload_bytes: &[u8],
    payload_hash: &str,
    idempotency_key: &str,
    build_id: &str,
    error_message: &str,
) -> std::result::Result<(), DomainError> {
    commands::insert_dead_letter(
        pool,
        None,
        step_name,
        workflow_id,
        error_class.as_str(),
        payload_type,
        schema_version,
        payload_bytes,
        payload_hash,
        payload_hash,
        idempotency_key,
        build_id,
        error_message,
    )
    .await
    .map_err(classify_sqlx)
}
