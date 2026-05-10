use anyhow::Result;
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::sqlx_serp_adapter::{self, RawSnapshotRecord};

/// Сохранить raw SERP snapshot (один URL из поисковой выдачи).
/// Идемпотентен по (run_id, job_id).
pub async fn save_raw_snapshot(pool: &AlegriaPgPool, record: &RawSnapshotRecord) -> Result<()> {
    sqlx_serp_adapter::save_raw_snapshot(pool, record).await
}

/// Добавить URL в очередь краулинга.
/// Конфликт по url_norm — берём MAX приоритет.
pub async fn enqueue_crawl(pool: &AlegriaPgPool, url_norm: &str, priority: i32) -> Result<()> {
    sqlx_serp_adapter::enqueue_crawl(pool, url_norm, priority).await
}

/// Массовый ingest JSONL-батча.
/// На boundary допускаются готовые JSON-строки, но use_case не оперирует ad hoc JSON объектами.
pub async fn ingest_batch(pool: &AlegriaPgPool, items: &[RawSnapshotRecord]) -> Result<usize> {
    let mut written = 0usize;
    for item in items {
        if item.run_id.is_empty() || item.job_id.is_empty() || item.url_norm.is_empty() {
            continue;
        }
        save_raw_snapshot(pool, item).await?;
        written += 1;
    }
    Ok(written)
}
