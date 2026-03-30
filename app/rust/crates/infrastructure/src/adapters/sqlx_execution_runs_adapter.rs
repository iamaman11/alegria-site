use sqlx::PgPool;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::proto_runtime_payload_store::{
    classify_sqlx, contract_violation, encode_runtime_payload, RuntimeProtoPayload,
};
use primitives::errors::DomainError;
use runtime_models::{ExecutionRun, ExecutionRunBlob};

mod rows {
    use sqlx::types::time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Debug)]
    pub(super) struct ExecutionRunBlobRow {
        pub(super) payload_type: String,
        pub(super) schema_version: i32,
        pub(super) payload_bytes: Vec<u8>,
    }

    #[derive(Debug)]
    pub(super) struct ExecutionRunHeadRow {
        pub(super) run_id: Uuid,
        pub(super) context_key: String,
        pub(super) status: String,
        pub(super) created_at: OffsetDateTime,
        pub(super) updated_at: OffsetDateTime,
    }
}

mod queries {
    use super::rows::{ExecutionRunBlobRow, ExecutionRunHeadRow};
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn fetch_execution_run_blob(
        pool: &PgPool,
        run_id: Uuid,
        field_name: &str,
    ) -> Result<Option<ExecutionRunBlobRow>, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT payload_type, schema_version, payload_bytes
            FROM pipeline.execution_run_blobs
            WHERE run_id = $1 AND field_name = $2
            LIMIT 1
            "#,
            run_id,
            field_name
        )
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| ExecutionRunBlobRow {
            payload_type: r.payload_type,
            schema_version: r.schema_version,
            payload_bytes: r.payload_bytes,
        }))
    }

    pub(super) async fn fetch_execution_run_head(
        pool: &PgPool,
        run_id: Uuid,
    ) -> Result<ExecutionRunHeadRow, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT run_id, context_key, status, created_at, updated_at
            FROM pipeline.execution_runs
            WHERE run_id = $1
            "#,
            run_id
        )
        .fetch_one(pool)
        .await?;
        Ok(ExecutionRunHeadRow {
            run_id: row.run_id,
            context_key: row.context_key,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn offset_to_utc(value: OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(value.unix_timestamp(), value.nanosecond())
        .unwrap_or_else(chrono::Utc::now)
}

mod commands {
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) async fn update_execution_run_status(
        pool: &PgPool,
        run_id: Uuid,
        new_status: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            UPDATE pipeline.execution_runs
            SET status = $1, updated_at = now()
            WHERE run_id = $2
            "#,
            new_status,
            run_id
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub(super) async fn upsert_execution_run_blob(
        pool: &PgPool,
        run_id: Uuid,
        field_name: &str,
        payload_type: &str,
        schema_version: i32,
        payload_bytes: &[u8],
        payload_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (run_id, field_name)
            DO UPDATE SET payload_type = EXCLUDED.payload_type,
                          schema_version = EXCLUDED.schema_version,
                          payload_bytes = EXCLUDED.payload_bytes,
                          payload_hash = EXCLUDED.payload_hash,
                          updated_at = now()
            "#,
            run_id,
            field_name,
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

async fn read_execution_run_blob(
    pool: &PgPool,
    run_id: Uuid,
    field_name: &str,
) -> std::result::Result<Option<ExecutionRunBlob>, DomainError> {
    let row = queries::fetch_execution_run_blob(pool, run_id, field_name)
        .await
        .map_err(classify_sqlx)?;
    Ok(row.map(|r| ExecutionRunBlob {
        payload_type: r.payload_type,
        schema_version: r.schema_version,
        payload_bytes: r.payload_bytes,
    }))
}

pub async fn read_execution_run(pool: &PgPool, run_id: &str) -> std::result::Result<ExecutionRun, DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let row = queries::fetch_execution_run_head(pool, uuid)
        .await
        .map_err(classify_sqlx)?;
    Ok(ExecutionRun {
        run_id: row.run_id,
        context_key: row.context_key,
        status: row.status,
        input_payload: read_execution_run_blob(pool, uuid, "input_payload").await?,
        extracted_payload: read_execution_run_blob(pool, uuid, "extracted_payload").await?,
        verify_report: read_execution_run_blob(pool, uuid, "verify_report").await?,
        generation_result: read_execution_run_blob(pool, uuid, "generation_result").await?,
        persist_report: read_execution_run_blob(pool, uuid, "persist_report").await?,
        created_at: offset_to_utc(row.created_at),
        updated_at: offset_to_utc(row.updated_at),
    })
}

pub async fn advance_execution_run(
    pool:        &PgPool,
    run_id:      &str,
    new_status:  &str,
    data_column: Option<&str>,
    payload_type: Option<&str>,
    schema_version: Option<i32>,
    payload_bytes:  Option<&[u8]>,
    payload_hash:  Option<&str>,
) -> std::result::Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    commands::update_execution_run_status(pool, uuid, new_status)
        .await
        .map_err(classify_sqlx)?;

    if let (Some(field_name), Some(payload_type), Some(schema_version), Some(payload_bytes), Some(payload_hash)) =
        (data_column, payload_type, schema_version, payload_bytes, payload_hash)
    {
        commands::upsert_execution_run_blob(
            pool,
            uuid,
            field_name,
            payload_type,
            schema_version,
            payload_bytes,
            payload_hash,
        )
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(())
}

pub async fn advance_execution_run_typed<T: RuntimeProtoPayload>(
    pool: &PgPool,
    run_id: &str,
    new_status: &str,
    data_column: Option<&str>,
    data_value: Option<&T>,
) -> std::result::Result<(), DomainError> {
    let encoded = match data_value {
        Some(v) => Some(encode_runtime_payload(v)?),
        None => None,
    };
    advance_execution_run(
        pool,
        run_id,
        new_status,
        data_column,
        encoded.as_ref().map(|_| T::payload_type()),
        encoded.as_ref().map(|_| T::schema_version()),
        encoded.as_ref().map(|(bytes, _)| bytes.as_slice()),
        encoded.as_ref().map(|(_, hash)| hash.as_str()),
    )
    .await
}
