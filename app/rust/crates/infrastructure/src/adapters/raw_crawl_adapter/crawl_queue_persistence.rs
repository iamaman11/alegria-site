// Queue state transitions only: acquisition::crawl_policy owns eligibility
// and retry decisions; this adapter persists their already-typed outcomes.
pub async fn mark_crawl_done(
    pool: &PgPool,
    url_norm: &str,
    http_status: i32,
    notes: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'done',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = NULL,
             notes = $3
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(notes)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_failed(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'failed',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = left($3, 2000),
             notes = left($3, 2000)
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_retry(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
    retry_after: acquisition::crawl_policy::RetryDelaySeconds,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'pending',
             http_status = $2,
             next_attempt_at = now() + make_interval(secs => $3::int),
             locked_until = NULL,
             last_error = left($4, 2000),
             notes = concat_ws('; ', nullif(notes, ''), left($4, 1800), concat('retry_in_sec=', $3::text))
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(retry_after.as_i32())
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}
