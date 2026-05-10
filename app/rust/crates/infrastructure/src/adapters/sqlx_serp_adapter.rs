use anyhow::{Context, Result};
use sqlx::PgPool;

use super::dataforseo_serp_adapter::DataForSeoOrganicResult;

#[derive(Debug, Clone)]
pub struct RawSnapshotRecord {
    pub run_id: String,
    pub job_id: String,
    pub url_norm: String,
    pub query: String,
    pub raw_payload_utf8: String,
}

pub async fn save_raw_snapshot(pool: &PgPool, record: &RawSnapshotRecord) -> Result<()> {
    sqlx::query(
        "INSERT INTO serp.raw_snapshots
         (run_id, job_id, query, raw_result)
         VALUES ($1, $2, $3, $4::jsonb)
         ON CONFLICT (run_id, job_id) DO UPDATE
         SET query      = EXCLUDED.query,
             raw_result = EXCLUDED.raw_result",
    )
    .bind(&record.run_id)
    .bind(&record.job_id)
    .bind(if record.query.trim().is_empty() {
        &record.url_norm
    } else {
        &record.query
    })
    .bind(&record.raw_payload_utf8)
    .execute(pool)
    .await
    .context("save raw serp snapshot")?;
    Ok(())
}

pub async fn save_dataforseo_organic_results_and_enqueue(
    pool: &PgPool,
    run_id: &str,
    job_id: &str,
    query_batch_key: &str,
    results: &[DataForSeoOrganicResult],
) -> Result<usize> {
    let mut tx = pool.begin().await.context("begin serp organic tx")?;
    let mut written = 0usize;
    for result in results {
        sqlx::query(
            "INSERT INTO serp.gemini_top10
             (run_id, job_id, rank, title, url, url_norm, domain_norm, also_in_sources)
             VALUES ($1, $2, $3, $4, $5, $6, $7, false)
             ON CONFLICT (run_id, job_id, rank) DO UPDATE
             SET title       = EXCLUDED.title,
                 url         = EXCLUDED.url,
                 url_norm    = EXCLUDED.url_norm,
                 domain_norm = EXCLUDED.domain_norm",
        )
        .bind(run_id)
        .bind(job_id)
        .bind(result.rank)
        .bind(&result.title)
        .bind(&result.url)
        .bind(&result.url_norm)
        .bind(&result.domain_norm)
        .execute(&mut *tx)
        .await
        .context("save DataForSEO organic result")?;

        sqlx::query(
            "INSERT INTO serp.crawl_queue
             (url, url_norm, source_type, dtype, first_seen_run_id, first_seen_job_id, query_batch_key, status, notes)
             VALUES ($1, $2, 'top10', 'organic_competitor', $3, $4, $5, 'pending', $6)
             ON CONFLICT (url_norm) DO UPDATE
             SET status = CASE
                     WHEN serp.crawl_queue.status IN ('done','processing') THEN serp.crawl_queue.status
                     ELSE 'pending'
                 END,
                 query_batch_key = CASE
                     WHEN serp.crawl_queue.query_batch_key = '' THEN EXCLUDED.query_batch_key
                     ELSE serp.crawl_queue.query_batch_key
                 END,
                 notes = EXCLUDED.notes",
        )
        .bind(&result.url)
        .bind(&result.url_norm)
        .bind(run_id)
        .bind(job_id)
        .bind(query_batch_key)
        .bind(format!(
            "dataforseo_rank={}; title={}; snippet_hash={}",
            result.rank,
            result.title.chars().take(120).collect::<String>(),
            primitives::hash::content_hash_v1(&result.snippet)
        ))
        .execute(&mut *tx)
        .await
        .context("enqueue DataForSEO organic crawl")?;
        written += 1;
    }
    tx.commit().await.context("commit serp organic tx")?;
    Ok(written)
}

pub async fn enqueue_crawl(pool: &PgPool, url_norm: &str, priority: i32) -> Result<()> {
    sqlx::query(
        "INSERT INTO serp.crawl_queue
         (url, url_norm, source_type, first_seen_run_id, first_seen_job_id, status, notes)
         VALUES ($1, $1, 'manual', 'manual', 'manual', 'pending', $2)
         ON CONFLICT (url_norm) DO UPDATE
         SET status = CASE
                 WHEN serp.crawl_queue.status IN ('done','processing') THEN serp.crawl_queue.status
                 ELSE 'pending'
             END,
             notes = EXCLUDED.notes",
    )
    .bind(url_norm)
    .bind(format!("priority={priority}"))
    .execute(pool)
    .await
    .context("enqueue serp crawl")?;
    Ok(())
}
