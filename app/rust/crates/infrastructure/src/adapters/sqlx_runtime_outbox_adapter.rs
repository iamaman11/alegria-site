use sqlx::types::time::OffsetDateTime;
use sqlx::PgPool;
use uuid::Uuid;

use super::proto_runtime_payload_store::{classify_sqlx, contract_violation};
use super::sqlx_outbox_adapter::OutboxEnvelope;
use primitives::errors::DomainError;
use runtime_models::OutboxEventStatus;

mod rows {
    use sqlx::types::time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Debug)]
    pub(super) struct OutboxEventIdRow {
        pub(super) event_id: Uuid,
    }

    #[derive(Debug)]
    pub(super) struct OutboxEventStatusRow {
        pub(super) event_id: Uuid,
        pub(super) status: String,
        pub(super) retry_count: i32,
        pub(super) next_retry_at: OffsetDateTime,
        pub(super) last_error: Option<String>,
        pub(super) updated_at: OffsetDateTime,
    }
}

mod queries {
    use super::rows::{OutboxEventIdRow, OutboxEventStatusRow};
    use sqlx::PgPool;
    use sqlx::Row;
    use uuid::Uuid;

    use super::OutboxEnvelope;

    pub(super) async fn insert_or_touch_outbox_event(
        pool: &PgPool,
        event: &OutboxEnvelope,
        payload_hash: &str,
    ) -> Result<OutboxEventIdRow, sqlx::Error> {
        let row = sqlx::query(
            r#"
            INSERT INTO system.sync_outbox
            (run_id, aggregate_type, aggregate_key, target_system, event_type, payload_type, schema_version, idempotency_key, payload_bytes, payload_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (aggregate_key, event_type, idempotency_key) WHERE idempotency_key <> ''
            DO UPDATE SET updated_at = now()
            RETURNING event_id
            "#,
        )
        .bind(&event.run_id)
        .bind(&event.aggregate_type)
        .bind(&event.aggregate_key)
        .bind(&event.target_system)
        .bind(&event.event_type)
        .bind(&event.payload_type)
        .bind(event.schema_version)
        .bind(&event.idempotency_key)
        .bind(event.payload_bytes())
        .bind(payload_hash)
        .fetch_one(pool)
        .await?;
        Ok(OutboxEventIdRow {
            event_id: row.get("event_id"),
        })
    }

    pub(super) async fn fetch_outbox_event_status(
        pool: &PgPool,
        event_id: Uuid,
    ) -> Result<Option<OutboxEventStatusRow>, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT event_id, status, retry_count, next_retry_at, last_error, updated_at
            FROM system.sync_outbox
            WHERE event_id = $1
            "#,
            event_id
        )
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| OutboxEventStatusRow {
            event_id: r.event_id,
            status: r.status,
            retry_count: r.retry_count,
            next_retry_at: r.next_retry_at,
            last_error: r.last_error,
            updated_at: r.updated_at,
        }))
    }
}

fn offset_to_utc(value: OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(value.unix_timestamp(), value.nanosecond())
        .unwrap_or_else(chrono::Utc::now)
}

pub async fn outbox_emit_many(
    pool: &PgPool,
    events: &[OutboxEnvelope],
) -> std::result::Result<i64, DomainError> {
    if events.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let mut emitted: i64 = 0;
    for event in events {
        let payload_hash = primitives::hash::blake3_hex(event.payload_bytes());

        let res = sqlx::query(
            "INSERT INTO system.sync_outbox \
             (run_id, aggregate_type, aggregate_key, target_system, event_type, payload_type, schema_version, idempotency_key, payload_bytes, payload_hash) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (aggregate_key, event_type, idempotency_key) WHERE idempotency_key <> '' DO NOTHING",
        )
        .bind(&event.run_id)
        .bind(&event.aggregate_type)
        .bind(&event.aggregate_key)
        .bind(&event.target_system)
        .bind(&event.event_type)
        .bind(&event.payload_type)
        .bind(event.schema_version)
        .bind(&event.idempotency_key)
        .bind(event.payload_bytes())
        .bind(&payload_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        emitted += res.rows_affected() as i64;
    }
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(emitted)
}

pub async fn outbox_emit_one(
    pool: &PgPool,
    event: &OutboxEnvelope,
) -> std::result::Result<String, DomainError> {
    let payload_hash = primitives::hash::blake3_hex(event.payload_bytes());

    let row = queries::insert_or_touch_outbox_event(pool, event, &payload_hash)
        .await
        .map_err(classify_sqlx)?;

    Ok(row.event_id.to_string())
}

pub async fn outbox_get_event_status(
    pool: &PgPool,
    event_id: &str,
) -> std::result::Result<Option<OutboxEventStatus>, DomainError> {
    let eid = Uuid::parse_str(event_id)
        .map_err(|e| contract_violation(format!("invalid event_id: {e}")))?;
    let row = queries::fetch_outbox_event_status(pool, eid)
        .await
        .map_err(classify_sqlx)?;

    Ok(row.map(|r| OutboxEventStatus {
        event_id: r.event_id.to_string(),
        status: r.status,
        retry_count: r.retry_count,
        next_retry_at: Some(offset_to_utc(r.next_retry_at).to_rfc3339()),
        last_error: r.last_error,
        updated_at: offset_to_utc(r.updated_at).to_rfc3339(),
    }))
}
