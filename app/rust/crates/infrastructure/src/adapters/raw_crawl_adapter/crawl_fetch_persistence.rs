pub async fn claim_pending_crawl_batch(
    pool: &PgPool,
    run_id: &str,
    query_batch_key: &str,
    limit: i64,
) -> std::result::Result<Vec<CrawlQueueItem>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "WITH per_domain AS (
             SELECT DISTINCT ON (coalesce(nullif(source_domain, ''), url_norm))
                    url_norm,
                    first_seen_at,
                    CASE
                        WHEN dtype IN ('official', 'government') THEN 0
                        WHEN dtype IN ('vfs', 'delegated_authority', 'official_publisher') THEN 1
                        WHEN dtype IN ('agency', 'editorial') THEN 3
                        WHEN dtype IN ('forum') THEN 4
                        ELSE 2
                    END AS source_priority
             FROM serp.crawl_queue
             WHERE status = 'pending'
               AND ($2 = '' OR first_seen_run_id = $2)
               AND ($3 = '' OR query_batch_key = $3)
               AND next_attempt_at <= now()
               AND (locked_until IS NULL OR locked_until <= now())
             ORDER BY coalesce(nullif(source_domain, ''), url_norm), source_priority, first_seen_at
         ),
         candidates AS (
             SELECT q.url_norm
             FROM serp.crawl_queue q
             JOIN per_domain d ON d.url_norm = q.url_norm
             ORDER BY d.source_priority, q.first_seen_at
             LIMIT $1
             FOR UPDATE OF q SKIP LOCKED
         )
         UPDATE serp.crawl_queue q
         SET status = 'processing',
             crawl_attempt_count = q.crawl_attempt_count + 1,
             locked_until = now() + interval '10 minutes',
             notes = concat_ws('; ', nullif(q.notes, ''), 'claimed_by=raw_crawl_adapter')
         FROM candidates c
         WHERE q.url_norm = c.url_norm
         RETURNING q.url, q.url_norm, coalesce(q.source_domain, '') AS source_domain,
                   q.source_type, coalesce(q.dtype, '') AS dtype, q.crawl_attempt_count",
    )
    .bind(limit.max(1))
    .bind(run_id)
    .bind(query_batch_key)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| CrawlQueueItem {
            url: row.get("url"),
            url_norm: row.get("url_norm"),
            source_domain: row.get("source_domain"),
            source_type: row.get("source_type"),
            dtype: row.get("dtype"),
            attempt_count: row.get("crawl_attempt_count"),
        })
        .collect())
}

pub async fn fetch_html(url: &str) -> Result<FetchedHtml> {
    let source_url = url.to_string();
    let redirect_chain = Arc::new(Mutex::new(vec![source_url.clone()]));
    let redirect_chain_for_policy = Arc::clone(&redirect_chain);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .redirect(Policy::custom(move |attempt| {
            if let Ok(mut chain) = redirect_chain_for_policy.lock() {
                chain.clear();
                chain.extend(
                    attempt
                        .previous()
                        .iter()
                        .map(|previous| previous.as_str().to_string()),
                );
                chain.push(attempt.url().as_str().to_string());
            }
            if attempt.previous().len() > MAX_REDIRECT_HOPS {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .context("build crawl client")?;
    let response = client
        .get(&source_url)
        .header(reqwest::header::USER_AGENT, CRAWL_USER_AGENT)
        .send()
        .await
        .with_context(|| format!("fetch source HTML: {source_url}"))?;
    let final_url = response.url().to_string();
    let status_code = response.status().as_u16() as i32;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();
    let body = response
        .text()
        .await
        .with_context(|| format!("read source HTML body: {source_url}"))?;
    let mut redirect_chain = redirect_chain
        .lock()
        .map(|chain| chain.clone())
        .unwrap_or_else(|_| vec![source_url.clone()]);
    if redirect_chain.is_empty() {
        redirect_chain.push(source_url.clone());
    }
    if redirect_chain.last().map(|value| value.as_str()) != Some(final_url.as_str()) {
        redirect_chain.push(final_url.clone());
    }
    Ok(FetchedHtml {
        source_url,
        final_url,
        status_code,
        content_type,
        body,
        redirect_chain,
    })
}

pub async fn evaluate_robots_policy(source_url: &str) -> Result<RobotsDecisionTrace> {
    let source =
        Url::parse(source_url).with_context(|| format!("parse source url: {source_url}"))?;
    let mut robots_url = source.clone();
    robots_url.set_path("/robots.txt");
    robots_url.set_query(None);
    robots_url.set_fragment(None);

    let mut trace = RobotsDecisionTrace {
        source_url: source_url.to_string(),
        robots_url: robots_url.as_str().to_string(),
        user_agent: CRAWL_USER_AGENT.to_string(),
        allowed: true,
        ..RobotsDecisionTrace::default()
    };

    let client = reqwest_adapter::new_default_client(20)?;
    let response = match client
        .get(robots_url.as_str())
        .header(reqwest::header::USER_AGENT, CRAWL_USER_AGENT)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            trace.allowed = false;
            trace.fetch_error = Some(format!("robots fetch failed: {err}"));
            return Ok(trace);
        }
    };

    let status = response.status();
    trace.robots_status = Some(status.as_u16() as i32);
    if status.as_u16() == 404 || status.as_u16() == 410 {
        return Ok(trace);
    }
    if !status.is_success() {
        trace.allowed = false;
        trace.fetch_error = Some(format!("robots http_status={}", status.as_u16()));
        return Ok(trace);
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("read robots body: {robots_url}"))?;
    let parsed = parse_robots_rules(&body, CRAWL_USER_AGENT, source.path());
    trace.allow_rules = parsed.allow_rules;
    trace.disallow_rules = parsed.disallow_rules;
    trace.allowed = parsed.allowed;
    trace.matched_rule = parsed.matched_rule;
    Ok(trace)
}

pub async fn save_crawled_html(
    pool: &PgPool,
    source_url: &str,
    source_domain: &str,
    final_url: &str,
    dtype: &str,
    status_code: i32,
    content_type: &str,
    raw_html: &str,
    redirect_chain: &[String],
    robots_trace: &RobotsDecisionTrace,
) -> std::result::Result<SavedRawPage, primitives::errors::DomainError> {
    let meta = extract_meta_typed(raw_html);
    let sections = extract_sections_typed(raw_html, source_url);
    let content_hash = content_hash_v1(raw_html);
    let domain = if source_domain.trim().is_empty() {
        domain_norm(source_url)
    } else {
        domain_norm(source_domain)
    };
    let dtype = if dtype.trim().is_empty() {
        "organic_competitor"
    } else {
        dtype.trim()
    };
    let content = json!({
        "extractor": "primitives::html_sections@1",
        "source_url": source_url,
        "final_url": final_url,
        "redirect_chain": redirect_chain,
        "robots_trace": robots_trace,
        "section_count": sections.len(),
        "content_hash": content_hash,
    });
    let trace = CrawlObservationTrace {
        source_url: source_url.to_string(),
        final_url: final_url.to_string(),
        redirect_hops: redirect_chain.len().saturating_sub(1),
        redirect_chain: redirect_chain.to_vec(),
        robots: robots_trace.clone(),
    };
    let trace_json = serde_json::to_value(&trace).map_err(|err| {
        primitives::errors::DomainError::InfraUnavailable {
            message: format!("serialize crawl trace: {err}"),
        }
    })?;
    let redirect_chain_json = Json(json!(redirect_chain));
    let robots_trace_json = Json(robots_trace.clone());
    let source_observation_json = Json(trace_json.clone());

    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let existing = sqlx::query(
        "SELECT id
         FROM raw.pages
         WHERE url = $1
           AND CAST(snapshot_at AT TIME ZONE 'UTC' AS DATE) = CAST(now() AT TIME ZONE 'UTC' AS DATE)
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(source_url)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify_sqlx)?;

    let page_id = if let Some(row) = existing {
        let page_id: i64 = row.get("id");
        sqlx::query(
            "UPDATE raw.pages
             SET domain = $2,
                 dtype = $3,
                 status_code = $4,
                 content_type = $5,
                 crawled_at = now(),
                 final_url = $6,
                 redirect_chain = $7,
                 robots_trace = $8,
                 source_observation = $9,
                 title = $10,
                 meta_desc = $11,
                 canonical = $12,
                 word_count = $13,
                 content = $14,
                 raw_html = $15,
                 raw_html_bytes = $16,
                 content_hash = $17,
                 processed = false
             WHERE id = $1",
        )
        .bind(page_id)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
        .bind(final_url)
        .bind(redirect_chain_json.clone())
        .bind(robots_trace_json.clone())
        .bind(source_observation_json.clone())
        .bind(&meta.title)
        .bind(&meta.meta_desc)
        .bind(&meta.canonical)
        .bind(meta.word_count as i32)
        .bind(Json(content.clone()))
        .bind(raw_html)
        .bind(raw_html.len() as i32)
        .bind(&content_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        page_id
    } else {
        let row = sqlx::query(
            "INSERT INTO raw.pages
             (url, domain, dtype, status_code, content_type, crawled_at, final_url, redirect_chain, robots_trace,
              source_observation, title, meta_desc, canonical, word_count, content, raw_html, raw_html_bytes,
              content_hash, processed)
             VALUES ($1, $2, $3, $4, $5, now(), $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, false)
             RETURNING id",
        )
        .bind(source_url)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
        .bind(final_url)
        .bind(redirect_chain_json.clone())
        .bind(robots_trace_json.clone())
        .bind(source_observation_json.clone())
        .bind(&meta.title)
        .bind(&meta.meta_desc)
        .bind(&meta.canonical)
        .bind(meta.word_count as i32)
        .bind(Json(content.clone()))
        .bind(raw_html)
        .bind(raw_html.len() as i32)
        .bind(&content_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        row.get("id")
    };

    sqlx::query("DELETE FROM raw.sections WHERE page_id = $1")
        .bind(page_id)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;

    for section in &sections {
        sqlx::query(
            "INSERT INTO raw.sections
             (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(page_id)
        .bind(&section.heading_path)
        .bind(section.heading_level as i16)
        .bind(section.section_order as i32)
        .bind(&section.section_type)
        .bind(&section.content_md)
        .bind(&section.content_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }

    tx.commit().await.map_err(classify_sqlx)?;
    let _ = record_content_hash_alias_linkage(pool, page_id, source_url, final_url, &content_hash)
        .await;
    Ok(SavedRawPage {
        page_id,
        section_count: sections.len(),
        content_hash,
    })
}

pub async fn record_content_hash_alias_linkage(
    pool: &PgPool,
    alias_page_id: i64,
    source_url: &str,
    final_url: &str,
    content_hash: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    if content_hash.trim().is_empty() {
        return Ok(());
    }
    let Some(row) = sqlx::query(
        "SELECT id
         FROM raw.pages
         WHERE content_hash = $1
           AND id <> $2
         ORDER BY id ASC
         LIMIT 1",
    )
    .bind(content_hash)
    .bind(alias_page_id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    else {
        return Ok(());
    };
    let canonical_page_id: i64 = row.get("id");
    if canonical_page_id == alias_page_id {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO raw.page_content_aliases
         (alias_page_id, canonical_page_id, source_url, final_url, content_hash)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (alias_page_id) DO UPDATE
         SET canonical_page_id = EXCLUDED.canonical_page_id,
             source_url = EXCLUDED.source_url,
             final_url = EXCLUDED.final_url,
             content_hash = EXCLUDED.content_hash,
             updated_at = now()",
    )
    .bind(alias_page_id)
    .bind(canonical_page_id)
    .bind(source_url)
    .bind(final_url)
    .bind(content_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}
