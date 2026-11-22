// Retained processing is an explicit property of a registered canonical run.
// Acquisition receipts and immutable raw snapshots remain owned by the original
// run. Never infer reuse from a matching keyword, or fall back to paid search.
pub async fn load_terminal_retained_source(
    pool: &PgPool,
    run_id: &str,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    let id = Uuid::parse_str(run_id)
        .map_err(|_| validation_failure("retained source run must be a UUID"))?;
    let eligible: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            WHERE r.run_id = $1 AND r.workflow_type = 'seo_site_build'
              AND r.status IN ('done','failed')
              AND NOT EXISTS (
                  SELECT 1 FROM pipeline.step_executions e
                  WHERE e.run_id = r.run_id AND e.status = 'running'
              )
         )",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    if !eligible {
        return Err(validation_failure("retained source run must be terminal with no running steps"));
    }
    let source = load_seo_site_build_input(pool, run_id).await?;
    if !source.retained_source_run_id.is_empty() {
        return Err(validation_failure("select the original acquisition run, not a retained copy"));
    }
    Ok(source)
}

pub fn verify_retained_scope(
    current: &SeoSiteBuildInputPayload,
    source: &SeoSiteBuildInputPayload,
) -> Result<(), DomainError> {
    if current.run_id == source.run_id
        || current.retained_source_run_id != source.run_id
        || current.context_key != source.context_key
        || current.scope.is_none()
        || current.scope != source.scope
        || current.queries != source.queries
        || current.query_batch_key == source.query_batch_key
        || !source.retained_source_run_id.is_empty()
    {
        return Err(validation_failure("retained corpus identity, scope or demand differs from original acquisition"));
    }
    Ok(())
}

async fn retained_acquisition_owner(
    pool: &PgPool,
    run_id: &str,
    batch: &str,
) -> Result<Option<SeoSiteBuildInputPayload>, DomainError> {
    let id = Uuid::parse_str(run_id)
        .map_err(|_| validation_failure("invalid acquisition run UUID"))?;
    let stored = sqlx::query(
        "SELECT payload_type, payload_bytes, payload_hash
         FROM pipeline.execution_run_blobs
         WHERE run_id = $1 AND field_name = 'input_payload'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    // Standalone diagnostic activities have no registered site input.
    let Some(row) = stored else {
        let canonical: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pipeline.execution_runs WHERE run_id=$1 AND workflow_type='seo_site_build')",
        ).bind(id).fetch_one(pool).await.map_err(classify_sqlx)?;
        if canonical {
            return Err(contract_violation("registered site run has no acquisition input"));
        }
        return Ok(None);
    };
    let ty: String = row.get("payload_type");
    let bytes: Vec<u8> = row.get("payload_bytes");
    let hash: String = row.get("payload_hash");
    if ty != SeoSiteBuildInputPayload::payload_type()
        || primitives::hash::blake3_hex(&bytes) != hash
    {
        return Err(contract_violation("invalid registered acquisition input"));
    }
    let current = SeoSiteBuildInputPayload::decode_payload_bytes(&bytes)?;
    if current.retained_source_run_id.is_empty() {
        return Ok(None);
    }
    if current.run_id != run_id || current.query_batch_key != batch {
        return Err(validation_failure("retained acquisition request differs from registered input"));
    }
    let source = load_terminal_retained_source(pool, &current.retained_source_run_id).await?;
    verify_retained_scope(&current, &source)?;
    Ok(Some(source))
}

async fn retained_step_outputs<T: RuntimeProtoPayload>(
    pool: &PgPool,
    source_run: &str,
    step_name: &str,
) -> Result<Vec<T>, DomainError> {
    let id = Uuid::parse_str(source_run)
        .map_err(|_| validation_failure("invalid retained source run UUID"))?;
    let rows = sqlx::query(
        "SELECT b.payload_type, b.payload_bytes, b.payload_hash, e.output_hash
         FROM pipeline.step_executions e
         JOIN pipeline.step_payload_blobs b ON b.run_id=e.run_id
           AND b.step_name=e.step_name AND b.idempotency_key=e.idempotency_key
           AND b.payload_kind='output'
         WHERE e.run_id=$1 AND e.step_name=$2 AND e.status='done'
         ORDER BY e.created_at, e.step_execution_id",
    )
    .bind(id)
    .bind(step_name)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    if rows.is_empty() {
        return Err(validation_failure(format!("retained acquisition has no completed {step_name} receipt")));
    }
    rows.into_iter().map(|row| {
        let ty: String = row.get("payload_type");
        let bytes: Vec<u8> = row.get("payload_bytes");
        let hash: String = row.get("payload_hash");
        let step_hash: Option<String> = row.get("output_hash");
        if ty != T::payload_type()
            || step_hash.as_deref() != Some(hash.as_str())
            || primitives::hash::blake3_hex(&bytes) != hash
        {
            return Err(contract_violation(format!("retained {step_name} receipt hash/type mismatch")));
        }
        T::decode_payload_bytes(&bytes)
    }).collect()
}

pub async fn retained_serp_ingest(
    pool: &PgPool,
    input: &SerpIngestInputPayload,
) -> Result<Option<SerpIngestOutputPayload>, DomainError> {
    let Some(source) = retained_acquisition_owner(pool, &input.run_id, &input.query_batch_key).await? else {
        return Ok(None);
    };
    if input.queries != source.queries || input.scope != source.scope {
        return Err(validation_failure("retained SERP demand or scope mismatch"));
    }
    let prior = retained_step_outputs::<SerpIngestOutputPayload>(
        pool, &source.run_id, "serp_ingest",
    ).await?;
    let [receipt] = prior.as_slice() else {
        return Err(validation_failure("retained SERP receipt is ambiguous"));
    };
    let expected = u32::try_from(source.queries.len())
        .map_err(|_| validation_failure("retained query count overflow"))?;
    if receipt.status != "done" || receipt.query_batch_key != source.query_batch_key
        || receipt.query_count != expected || receipt.persisted_snapshot_count != expected
    {
        return Err(validation_failure("retained SERP acquisition incomplete"));
    }

    // Copy only the exact successful provider results, never a synthetic query
    // observation or an unpaid/uncertain task POST receipt. Preserve provenance.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    for query in &source.queries {
        let rows = sqlx::query(
            "SELECT job_id, raw_result FROM serp.raw_snapshots
             WHERE run_id=$1 AND query=$2 AND job_id NOT LIKE 'dataforseo-standard-task-%'
               AND raw_result->>'status_code'='20000'
               AND raw_result->'tasks'->0->>'status_code'='20000'
               AND jsonb_typeof(raw_result->'tasks'->0->'result')='array'",
        )
        .bind(&source.run_id)
        .bind(query)
        .fetch_all(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        let [row] = rows.as_slice() else {
            return Err(validation_failure("retained query has missing or ambiguous provider result"));
        };
        let source_job: String = row.get("job_id");
        let mut result: Value = row.get("raw_result");
        if result["tasks"][0]["result"].as_array().is_none_or(Vec::is_empty) {
            return Err(validation_failure("retained provider result is empty"));
        }
        result["retained_provenance"] = json!({
            "source_run_id": source.run_id, "source_job_id": source_job,
        });
        let key = seo_domain::artifact_identity::seo_artifact_key(
            "retained_serp_result", &[&input.run_id, &source.run_id, &source_job],
        );
        sqlx::query(
            "INSERT INTO serp.raw_snapshots (run_id,job_id,query,raw_result)
             VALUES ($1,$2,$3,$4) ON CONFLICT (run_id,job_id) DO NOTHING",
        )
        .bind(&input.run_id).bind(&key).bind(query).bind(Json(&result))
        .execute(&mut *tx).await.map_err(classify_sqlx)?;
        let saved: Value = sqlx::query_scalar(
            "SELECT raw_result FROM serp.raw_snapshots WHERE run_id=$1 AND job_id=$2 AND query=$3",
        )
        .bind(&input.run_id).bind(&key).bind(query)
        .fetch_one(&mut *tx).await.map_err(classify_sqlx)?;
        if saved != result {
            return Err(contract_violation("retained SERP result changed on retry"));
        }

        // Preserve the actual ranked results as well as the raw receipt. Do
        // not invoke the live persistence adapter: it would enqueue a crawl.
        let source_rank_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM serp.gemini_top10 WHERE run_id=$1 AND job_id=$2",
        ).bind(&source.run_id).bind(&source_job)
         .fetch_one(&mut *tx).await.map_err(classify_sqlx)?;
        if source_rank_count == 0 {
            return Err(validation_failure("retained SERP has no saved organic ranks"));
        }
        sqlx::query(
            "INSERT INTO serp.gemini_top10
                (run_id,job_id,rank,title,url,url_norm,domain_norm,source_tier,also_in_sources)
             SELECT $1,$2,rank,title,url,url_norm,domain_norm,source_tier,also_in_sources
             FROM serp.gemini_top10 WHERE run_id=$3 AND job_id=$4
             ON CONFLICT (run_id,job_id,rank) DO NOTHING",
        )
        .bind(&input.run_id).bind(&key).bind(&source.run_id).bind(&source_job)
        .execute(&mut *tx).await.map_err(classify_sqlx)?;
        let matches: bool = sqlx::query_scalar(
            "WITH original AS (
                SELECT rank,title,url,url_norm,domain_norm,source_tier,also_in_sources
                FROM serp.gemini_top10 WHERE run_id=$1 AND job_id=$2
             ), retained AS (
                SELECT rank,title,url,url_norm,domain_norm,source_tier,also_in_sources
                FROM serp.gemini_top10 WHERE run_id=$3 AND job_id=$4
             )
             SELECT NOT EXISTS (
                (SELECT * FROM original EXCEPT SELECT * FROM retained)
                UNION ALL
                (SELECT * FROM retained EXCEPT SELECT * FROM original)
             )",
        )
        .bind(&source.run_id).bind(&source_job).bind(&input.run_id).bind(&key)
        .fetch_one(&mut *tx).await.map_err(classify_sqlx)?;
        if !matches {
            return Err(contract_violation("retained organic ranks differ from source"));
        }
    }
    tx.commit().await.map_err(classify_sqlx)?;
    let output = SerpIngestOutputPayload {
        query_batch_key: input.query_batch_key.clone(),
        query_count: expected,
        persisted_snapshot_count: expected,
        status: "done".into(),
    };
    persist_serp_ingest_output(pool, input, &output).await?;
    tracing::info!(run_id=%input.run_id, source_run_id=%source.run_id, query_count=expected,
        "reused acquired SERP results without provider calls");
    Ok(Some(output))
}

pub async fn retained_crawl_sources(
    pool: &PgPool,
    input: &contracts::generated::alegria::temporal::v1::CrawlSourcesInputPayload,
) -> Result<Option<contracts::generated::alegria::temporal::v1::CrawlSourcesOutputPayload>, DomainError> {
    use contracts::generated::alegria::temporal::v1::CrawlSourcesOutputPayload;
    let Some(source) = retained_acquisition_owner(pool, &input.run_id, &input.query_batch_key).await? else {
        return Ok(None);
    };
    if input.sweep_index != 0 {
        return Err(validation_failure("retained corpus must be loaded in a single sweep"));
    }
    let prior = retained_step_outputs::<CrawlSourcesOutputPayload>(pool, &source.run_id, "crawl_sources").await?;
    let mut ids = BTreeSet::new();
    let mut failed_urls = BTreeSet::new();
    for (index, receipt) in prior.iter().enumerate() {
        let final_receipt = index + 1 == prior.len();
        let valid_status = if final_receipt {
            matches!(receipt.status.as_str(), "done" | "partial")
        } else {
            matches!(receipt.status.as_str(), "more" | "pending_retry")
        };
        if !valid_status || receipt.raw_page_count as usize != receipt.raw_page_ids.len() {
            return Err(validation_failure("retained crawl receipt sequence is incomplete"));
        }
        for id in &receipt.raw_page_ids {
            if *id <= 0 || !ids.insert(*id) {
                return Err(validation_failure("retained crawl contains repeated or invalid page identity"));
            }
        }
        failed_urls.extend(receipt.failed_urls.iter().cloned());
    }
    let mut raw_page_ids = Vec::new();
    let mut raw_section_count = 0u32;
    for id in ids {
        let page = sqlx::query(
            "SELECT url, status_code, raw_html, content_hash FROM raw.pages WHERE id=$1",
        )
        .bind(id).fetch_one(pool).await.map_err(classify_sqlx)?;
        let url: String = page.get("url");
        let status: Option<i16> = page.get("status_code");
        let html: Option<String> = page.get("raw_html");
        let hash: Option<String> = page.get("content_hash");
        let html = html.ok_or_else(|| validation_failure(format!("retained page {id} has no snapshot bytes")))?;
        if hash.as_deref() != Some(primitives::hash::content_hash_v1(&html).as_str()) {
            return Err(contract_violation(format!("retained page {id} snapshot hash mismatch")));
        }
        let sections = sqlx::query(
            "SELECT content_md, content_hash FROM raw.sections WHERE page_id=$1 ORDER BY section_order,id",
        )
        .bind(id).fetch_all(pool).await.map_err(classify_sqlx)?;
        if status != Some(200) || html.trim().is_empty()
            || sections.is_empty()
            || sections.iter().all(|row| row.get::<String,_>("content_md").trim().is_empty())
        {
            failed_urls.insert(url);
            continue;
        }
        for section in &sections {
            let text: String = section.get("content_md");
            let actual: String = section.get("content_hash");
            if !text.trim().is_empty() && primitives::hash::content_hash_v1(&text) != actual {
                return Err(contract_violation(format!("retained section hash mismatch on page {id}")));
            }
        }
        raw_section_count = raw_section_count.saturating_add(sections.len() as u32);
        raw_page_ids.push(id);
    }
    if raw_page_ids.is_empty() {
        return Err(validation_failure("retained corpus has no usable source snapshots"));
    }
    let failed_urls: Vec<_> = failed_urls.into_iter().collect();
    tracing::info!(run_id=%input.run_id, source_run_id=%source.run_id,
        pages=raw_page_ids.len(), failed=failed_urls.len(), "reused immutable raw corpus without crawling");
    Ok(Some(CrawlSourcesOutputPayload {
        claimed_count: 0, crawled_count: 0, failed_count: failed_urls.len() as u32,
        raw_page_count: raw_page_ids.len() as u32, raw_section_count,
        qdrant_event_count: 0,
        status: if failed_urls.is_empty() { "done" } else { "partial" }.into(),
        raw_page_ids, failed_urls,
    }))
}

#[cfg(test)]
mod retained_corpus_identity_tests {
    use super::*;

    #[test]
    fn retained_corpus_requires_exact_context_scope_and_demand() {
        let source = SeoSiteBuildInputPayload {
            run_id: "old".into(), context_key: "original".into(),
            scope: Some(SeoScopePayload { locale: "ru-RU".into(), ..Default::default() }),
            query_batch_key: "old-batch".into(), queries: vec!["real query".into()],
            ..Default::default()
        };
        let mut current = SeoSiteBuildInputPayload {
            run_id: "new".into(), query_batch_key: "new-batch".into(),
            retained_source_run_id: source.run_id.clone(), ..source.clone()
        };
        assert!(verify_retained_scope(&current,&source).is_ok());
        current.context_key="different".into();
        assert!(verify_retained_scope(&current,&source).is_err());
        current.context_key=source.context_key.clone();
        current.queries.push("invented demand".into());
        assert!(verify_retained_scope(&current,&source).is_err());
        current.queries=source.queries.clone();
        current.query_batch_key=source.query_batch_key.clone();
        assert!(verify_retained_scope(&current,&source).is_err());
    }
}
