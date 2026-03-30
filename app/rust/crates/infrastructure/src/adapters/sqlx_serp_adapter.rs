use anyhow::{Context, Result};
use primitives::hash::blake3_hex;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct RawSnapshotRecord {
    pub run_id: String,
    pub job_id: String,
    pub url_norm: String,
    pub raw_payload_utf8: String,
}

pub async fn save_raw_snapshot(pool: &PgPool, record: &RawSnapshotRecord) -> Result<()> {
    let payload_hash = blake3_hex(record.raw_payload_utf8.as_bytes());
    sqlx::query(
        "INSERT INTO serp.raw_snapshots
         (run_id, job_id, url_norm, payload_json, payload_hash)
         VALUES ($1, $2, $3, $4::jsonb, $5)
         ON CONFLICT (run_id, job_id) DO UPDATE
         SET payload_json = EXCLUDED.payload_json,
             payload_hash = EXCLUDED.payload_hash,
             updated_at   = now()",
    )
    .bind(&record.run_id)
    .bind(&record.job_id)
    .bind(&record.url_norm)
    .bind(&record.raw_payload_utf8)
    .bind(&payload_hash)
    .execute(pool)
    .await
    .context("save raw serp snapshot")?;
    Ok(())
}

pub async fn enqueue_crawl(pool: &PgPool, url_norm: &str, priority: i32) -> Result<()> {
    sqlx::query(
        "INSERT INTO serp.crawl_queue (url_norm, priority)
         VALUES ($1, $2)
         ON CONFLICT (url_norm) DO UPDATE
         SET priority   = GREATEST(serp.crawl_queue.priority, EXCLUDED.priority),
             updated_at = now()",
    )
    .bind(url_norm)
    .bind(priority)
    .execute(pool)
    .await
    .context("enqueue serp crawl")?;
    Ok(())
}
