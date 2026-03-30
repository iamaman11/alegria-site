use sqlx::{PgPool, Row};

use super::proto_runtime_payload_store::classify_sqlx;
use primitives::errors::DomainError;
use primitives::hash::content_hash_v1;

pub async fn html_save(
    pool: &PgPool,
    page_id: i64,
    raw_html: &str,
) -> std::result::Result<String, DomainError> {
    let h = content_hash_v1(raw_html);
    let raw_bytes = raw_html.len() as i64;
    sqlx::query(
        "UPDATE raw.pages \
         SET raw_html = $1, raw_html_bytes = $2, content_hash = $3, crawled_at = now() \
         WHERE id = $4",
    )
    .bind(raw_html)
    .bind(raw_bytes)
    .bind(&h)
    .bind(page_id)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(h)
}

pub async fn html_load(
    pool: &PgPool,
    page_id: i64,
) -> std::result::Result<Option<String>, DomainError> {
    let row = sqlx::query("SELECT raw_html FROM raw.pages WHERE id = $1")
        .bind(page_id)
        .fetch_optional(pool)
        .await
        .map_err(classify_sqlx)?;
    Ok(row.and_then(|r| r.get::<Option<String>, _>("raw_html")))
}

pub async fn html_is_changed(
    pool: &PgPool,
    page_id: i64,
    new_html: &str,
) -> std::result::Result<bool, DomainError> {
    let new_h = content_hash_v1(new_html);
    let row = sqlx::query("SELECT content_hash FROM raw.pages WHERE id = $1")
        .bind(page_id)
        .fetch_optional(pool)
        .await
        .map_err(classify_sqlx)?;
    Ok(row.map_or(true, |r| {
        r.get::<Option<String>, _>("content_hash")
            .map_or(true, |h| h != new_h)
    }))
}

