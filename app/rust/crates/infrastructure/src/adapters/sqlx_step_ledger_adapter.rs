use sqlx::PgPool;
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, RuntimeProtoPayload,
};
use primitives::errors::DomainError;
use primitives::hash::{blake3_hex, content_hash_v1};

mod rows {
    #[derive(Debug)]
    pub(super) struct StepExecutionRow {
        pub(super) status: String,
    }

    #[derive(Debug)]
    pub(super) struct StepOutputBlobRow {
        pub(super) payload_type: String,
        pub(super) payload_bytes: Vec<u8>,
    }
}

mod queries {
    use super::rows::{StepExecutionRow, StepOutputBlobRow};
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn fetch_step_execution_id(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar!(
            r#"
            SELECT step_execution_id
            FROM pipeline.step_executions
            WHERE run_id = $1 AND step_name = $2 AND idempotency_key = $3
            LIMIT 1
            "#,
            run_id,
            step_name,
            idempotency_key
        )
        .fetch_optional(pool)
        .await
    }

    pub(super) async fn next_attempt_no(
        pool: &PgPool,
        step_execution_id: i64,
    ) -> Result<i32, sqlx::Error> {
        sqlx::query_scalar!(
            r#"
            SELECT COALESCE(MAX(attempt_no), 0) + 1 AS "next_attempt_no!"
            FROM pipeline.step_attempts
            WHERE step_execution_id = $1
            "#,
            step_execution_id
        )
        .fetch_one(pool)
        .await
    }

    pub(super) async fn fetch_completed_step(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
    ) -> Result<Option<StepExecutionRow>, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT status
            FROM pipeline.step_executions
            WHERE run_id = $1 AND step_name = $2 AND idempotency_key = $3
            LIMIT 1
            "#,
            run_id,
            step_name,
            idempotency_key
        )
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| StepExecutionRow { status: r.status }))
    }

    pub(super) async fn fetch_output_blob(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
    ) -> Result<Option<StepOutputBlobRow>, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT payload_type, payload_bytes
            FROM pipeline.step_payload_blobs
            WHERE run_id = $1 AND step_name = $2 AND idempotency_key = $3 AND payload_kind = 'output'
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            run_id,
            step_name,
            idempotency_key
        )
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| StepOutputBlobRow {
            payload_type: r.payload_type,
            payload_bytes: r.payload_bytes,
        }))
    }
}

mod commands {
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn insert_step_execution(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        schema_version: i32,
        input_hash: &str,
        idempotency_key: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"
            INSERT INTO pipeline.step_executions
            (run_id, step_name, schema_version, input_hash, idempotency_key, status)
            VALUES ($1, $2, $3, $4, $5, 'running')
            ON CONFLICT (step_name, idempotency_key) DO NOTHING
            "#,
            run_id,
            step_name,
            schema_version,
            input_hash,
            idempotency_key
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub(super) async fn insert_step_attempt(
        pool: &PgPool,
        step_execution_id: i64,
        attempt_no: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO pipeline.step_attempts
            (step_execution_id, attempt_no, status)
            VALUES ($1, $2, 'running')
            "#,
            step_execution_id,
            attempt_no
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn update_step_attempt_status(
        pool: &PgPool,
        step_execution_id: i64,
        attempt_no: i32,
        status: &str,
        error_class: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            UPDATE pipeline.step_attempts
            SET status = $1,
                error_class = $2,
                error_message = $3,
                finished_at = now()
            WHERE step_execution_id = $4
              AND attempt_no = $5
            "#,
            status,
            error_class,
            error_message,
            step_execution_id,
            attempt_no
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn update_step_execution_done(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
        output_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            UPDATE pipeline.step_executions
            SET status = 'done',
                output_hash = $1,
                error_class = NULL,
                error_message = NULL,
                updated_at = now()
            WHERE run_id = $2
              AND step_name = $3
              AND idempotency_key = $4
            "#,
            output_hash,
            run_id,
            step_name,
            idempotency_key
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn update_step_execution_failed(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
        terminal_status: &str,
        error_class: &str,
        error_message: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            UPDATE pipeline.step_executions
            SET status = $1,
                error_class = $2,
                error_message = $3,
                updated_at = now()
            WHERE run_id = $4
              AND step_name = $5
              AND idempotency_key = $6
              -- A concurrent attempt may have atomically committed Truth and
              -- marked this step done after another attempt began failing.
              -- A late error must never erase that durable replay fence.
              AND status <> 'done'
            "#,
            terminal_status,
            error_class,
            error_message,
            run_id,
            step_name,
            idempotency_key
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn insert_step_payload_blob(
        pool: &PgPool,
        run_id: Uuid,
        step_name: &str,
        idempotency_key: &str,
        payload_kind: &str,
        payload_type: &str,
        schema_version: i32,
        payload_bytes: &[u8],
        payload_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO pipeline.step_payload_blobs
            (run_id, step_name, idempotency_key, payload_kind, payload_type, schema_version, payload_bytes, payload_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (idempotency_key, payload_kind, payload_hash) DO NOTHING
            "#,
            run_id,
            step_name,
            idempotency_key,
            payload_kind,
            payload_type,
            schema_version,
            payload_bytes,
            payload_hash
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub async fn begin_step_execution(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input_hash: &str,
    idempotency_key: &str,
) -> std::result::Result<bool, DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let affected = commands::insert_step_execution(
        pool,
        uuid,
        step_name,
        schema_version,
        input_hash,
        idempotency_key,
    )
    .await
    .map_err(classify_sqlx)?;
    Ok(affected == 1)
}

pub async fn read_step_execution_id(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
) -> std::result::Result<Option<i64>, DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    queries::fetch_step_execution_id(pool, uuid, step_name, idempotency_key)
        .await
        .map_err(classify_sqlx)
}

pub async fn begin_step_attempt(
    pool: &PgPool,
    step_execution_id: i64,
) -> std::result::Result<i32, DomainError> {
    let attempt_no = queries::next_attempt_no(pool, step_execution_id)
        .await
        .map_err(classify_sqlx)?;
    commands::insert_step_attempt(pool, step_execution_id, attempt_no)
        .await
        .map_err(classify_sqlx)?;
    Ok(attempt_no)
}

pub async fn finish_step_attempt(
    pool: &PgPool,
    step_execution_id: i64,
    attempt_no: i32,
    status: &str,
    error_class: Option<&str>,
    error_message: Option<&str>,
) -> std::result::Result<(), DomainError> {
    commands::update_step_attempt_status(
        pool,
        step_execution_id,
        attempt_no,
        status,
        error_class,
        error_message,
    )
    .await
    .map_err(classify_sqlx)
}

pub async fn load_completed_step_result<T>(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
) -> std::result::Result<Option<T>, DomainError>
where
    T: RuntimeProtoPayload,
{
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let Some(step) = queries::fetch_completed_step(pool, uuid, step_name, idempotency_key)
        .await
        .map_err(classify_sqlx)?
    else {
        return Ok(None);
    };
    if step.status != "done" {
        return Ok(None);
    }
    let Some(blob) = queries::fetch_output_blob(pool, uuid, step_name, idempotency_key)
        .await
        .map_err(classify_sqlx)?
    else {
        return Ok(None);
    };
    if blob.payload_type != T::payload_type() {
        return Err(contract_violation(format!(
            "step output payload_type mismatch: expected {}, got {}",
            T::payload_type(),
            blob.payload_type
        )));
    }
    Ok(Some(T::decode_payload_bytes(&blob.payload_bytes)?))
}

/// Recheck the same durable step while holding the Truth context lock. Two
/// Temporal attempts can both enter execute_step while the first is running;
/// the waiting attempt must return its committed result instead of replaying
/// canonical writes after the first has finished.
pub async fn load_completed_step_result_in_tx<I, O>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input: &I,
) -> std::result::Result<Option<O>, DomainError>
where
    I: RuntimeProtoPayload,
    O: RuntimeProtoPayload,
{
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (input_hash, idempotency_key) = derive_step_keys(run_id, step_name, schema_version, input)?;
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM pipeline.step_executions
         WHERE run_id=$1 AND step_name=$2 AND idempotency_key=$3 AND input_hash=$4",
    )
    .bind(uuid)
    .bind(step_name)
    .bind(&idempotency_key)
    .bind(&input_hash)
    .fetch_optional(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let Some(status) = status else {
        return Err(contract_violation(format!(
            "missing activity step ledger: {step_name} run_id={run_id}"
        )));
    };
    if status != "done" {
        return Ok(None);
    }
    let blob: Option<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT payload_type, payload_bytes FROM pipeline.step_payload_blobs
         WHERE run_id=$1 AND step_name=$2 AND idempotency_key=$3 AND payload_kind='output'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(uuid)
    .bind(step_name)
    .bind(&idempotency_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let (payload_type, payload_bytes) = blob.ok_or_else(|| {
        contract_violation(format!(
            "completed activity step missing output: {step_name} run_id={run_id}"
        ))
    })?;
    if payload_type != O::payload_type() {
        return Err(contract_violation(format!(
            "activity step output payload_type mismatch: expected {}, got {payload_type}",
            O::payload_type()
        )));
    }
    Ok(Some(O::decode_payload_bytes(&payload_bytes)?))
}

pub async fn complete_step_execution(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    output_hash: &str,
) -> std::result::Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    commands::update_step_execution_done(pool, uuid, step_name, idempotency_key, output_hash)
        .await
        .map_err(classify_sqlx)
}

/// Commit an activity's durable result with the canonical SQL mutations it
/// reports. A worker crash after that commit but before its Temporal ACK must
/// replay the stored result, never reapply an older Truth write over a newer
/// run. The ordinary execute_step wrapper already creates this ledger row.
pub async fn complete_step_execution_with_output_in_tx<I, O>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input: &I,
    output: &O,
) -> std::result::Result<(), DomainError>
where
    I: RuntimeProtoPayload,
    O: RuntimeProtoPayload,
{
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (input_hash, idempotency_key) = derive_step_keys(run_id, step_name, schema_version, input)?;
    let payload_bytes = output.encode_payload_bytes()?;
    let output_hash = blake3_hex(&payload_bytes);
    sqlx::query(
        "INSERT INTO pipeline.step_payload_blobs
         (run_id, step_name, idempotency_key, payload_kind, payload_type,
          schema_version, payload_bytes, payload_hash)
         VALUES ($1, $2, $3, 'output', $4, $5, $6, $7)
         ON CONFLICT (idempotency_key, payload_kind, payload_hash) DO NOTHING",
    )
    .bind(uuid)
    .bind(step_name)
    .bind(&idempotency_key)
    .bind(O::payload_type())
    .bind(schema_version)
    .bind(&payload_bytes)
    .bind(&output_hash)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let marked = sqlx::query(
        "UPDATE pipeline.step_executions
         SET status = 'done', output_hash = $5,
             error_class = NULL, error_message = NULL, updated_at = now()
         WHERE run_id = $1 AND step_name = $2 AND idempotency_key = $3
           AND input_hash = $4 AND status IN ('running', 'failed', 'done')",
    )
    .bind(uuid)
    .bind(step_name)
    .bind(&idempotency_key)
    .bind(&input_hash)
    .bind(&output_hash)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    if marked.rows_affected() != 1 {
        return Err(contract_violation(format!(
            "missing or incompatible activity step ledger: {step_name} run_id={run_id}"
        )));
    }
    // A crash after commit must not leave the activity's attempt marked
    // running forever while its canonical state and step result are done.
    sqlx::query(
        "UPDATE pipeline.step_attempts
         SET status='done', finished_at=now()
         WHERE step_execution_id IN (
             SELECT step_execution_id FROM pipeline.step_executions
             WHERE run_id=$1 AND step_name=$2 AND idempotency_key=$3
         ) AND status='running'",
    )
    .bind(uuid)
    .bind(step_name)
    .bind(&idempotency_key)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn complete_step_execution_typed<T: RuntimeProtoPayload>(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    output_hash: &str,
    _result_payload: Option<&T>,
) -> std::result::Result<(), DomainError> {
    complete_step_execution(pool, run_id, step_name, idempotency_key, output_hash).await
}

pub async fn fail_step_execution(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    error_class: &str,
    error_message: &str,
    terminal_status: &str,
) -> std::result::Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    commands::update_step_execution_failed(
        pool,
        uuid,
        step_name,
        idempotency_key,
        terminal_status,
        error_class,
        error_message,
    )
    .await
    .map_err(classify_sqlx)
}

pub fn derive_step_keys<T: RuntimeProtoPayload>(
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input: &T,
) -> std::result::Result<(String, String), DomainError> {
    let payload = input.encode_payload_bytes()?;
    let input_hash = blake3_hex(&payload);
    let idempotency_key = content_hash_v1(&format!(
        "{run_id}|{step_name}|{schema_version}|{input_hash}"
    ));
    Ok((input_hash, idempotency_key))
}

pub async fn write_step_payload_blob_typed<T>(
    pool: &PgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    payload_kind: &str,
    schema_version: i32,
    payload: &T,
) -> std::result::Result<String, DomainError>
where
    T: RuntimeProtoPayload,
{
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (payload_bytes, payload_hash) = encode_runtime_payload(payload)?;
    commands::insert_step_payload_blob(
        pool,
        uuid,
        step_name,
        idempotency_key,
        payload_kind,
        T::payload_type(),
        schema_version.max(T::schema_version()),
        &payload_bytes,
        &payload_hash,
    )
    .await
    .map_err(classify_sqlx)?;
    Ok(payload_hash)
}
