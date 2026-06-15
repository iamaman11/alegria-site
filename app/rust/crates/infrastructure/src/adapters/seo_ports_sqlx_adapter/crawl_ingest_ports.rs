#[async_trait]
impl CrawlIngestRepository for SqlxSeoRuntimeRepository<'_> {
    async fn crawl_sources(
        &self,
        input: &CrawlSourcesInputPayload,
    ) -> Result<CrawlSourcesOutputPayload, DomainError> {
        let limit = if input.limit == 0 { 25 } else { input.limit } as i64;
        let items = raw_crawl_adapter::claim_pending_crawl_batch(
            self.pool,
            &input.run_id,
            &input.query_batch_key,
            limit,
        )
        .await?;
        let claimed_count = items.len() as u32;
        let mut crawled_count = 0u32;
        let mut failed_count = 0u32;
        let mut raw_page_count = 0u32;
        let mut raw_section_count = 0u32;
        let mut qdrant_event_count = 0u32;
        let mut raw_page_ids = Vec::new();
        let mut failed_urls = Vec::new();

        for item in items {
            let robots_trace = match raw_crawl_adapter::evaluate_robots_policy(&item.url).await {
                Ok(trace) if trace.allowed => trace,
                Ok(trace) => {
                    raw_crawl_adapter::mark_crawl_failed(
                        self.pool,
                        &item.url_norm,
                        None,
                        &format!(
                            "robots disallow: robots_url={}; matched_rule={:?}; fetch_error={:?}",
                            trace.robots_url, trace.matched_rule, trace.fetch_error
                        ),
                    )
                    .await?;
                    failed_count += 1;
                    failed_urls.push(item.url.clone());
                    continue;
                }
                Err(err) => {
                    raw_crawl_adapter::mark_crawl_failed(
                        self.pool,
                        &item.url_norm,
                        None,
                        &format!("robots evaluation failed: {err}"),
                    )
                    .await?;
                    failed_count += 1;
                    failed_urls.push(item.url.clone());
                    continue;
                }
            };

            match raw_crawl_adapter::fetch_html(&item.url).await {
                Ok(fetched) if (200..400).contains(&fetched.status_code) => {
                    match raw_crawl_adapter::save_crawled_html(
                        self.pool,
                        &fetched.source_url,
                        &item.source_domain,
                        &fetched.final_url,
                        &item.dtype,
                        fetched.status_code,
                        &fetched.content_type,
                        &fetched.body,
                        &fetched.redirect_chain,
                        &robots_trace,
                    )
                    .await
                    {
                        Ok(saved) => {
                            let emitted = if input.emit_qdrant {
                                raw_crawl_adapter::emit_raw_section_qdrant_events(
                                    self.pool,
                                    &input.run_id,
                                    saved.page_id,
                                )
                                .await?
                            } else {
                                0
                            };
                            raw_crawl_adapter::mark_crawl_done(
                                self.pool,
                                &item.url_norm,
                                fetched.status_code,
                                &format!(
                                    "source_type={}; source_url={}; final_url={}; redirect_hops={}; page_id={}; sections={}; qdrant_events={}; hash={}",
                                    item.source_type,
                                    fetched.source_url,
                                    fetched.final_url,
                                    fetched.redirect_chain.len().saturating_sub(1),
                                    saved.page_id,
                                    saved.section_count,
                                    emitted,
                                    saved.content_hash
                                ),
                            )
                            .await?;
                            crawled_count += 1;
                            raw_page_count += 1;
                            raw_section_count += saved.section_count as u32;
                            qdrant_event_count += emitted as u32;
                            raw_page_ids.push(saved.page_id);
                        }
                        Err(err) => {
                            raw_crawl_adapter::mark_crawl_failed(
                                self.pool,
                                &item.url_norm,
                                Some(fetched.status_code),
                                &format!("persist failed: {err}"),
                            )
                            .await?;
                            failed_count += 1;
                            failed_urls.push(item.url.clone());
                        }
                    }
                }
                Ok(fetched) => {
                    let error = format!(
                        "http_status={}; content_type={}; source_url={}; final_url={}",
                        fetched.status_code,
                        fetched.content_type,
                        fetched.source_url,
                        fetched.final_url
                    );
                    if raw_crawl_adapter::should_retry_http_status(fetched.status_code)
                        && raw_crawl_adapter::should_retry_crawl_attempt(item.attempt_count)
                    {
                        raw_crawl_adapter::mark_crawl_retry(
                            self.pool,
                            &item.url_norm,
                            Some(fetched.status_code),
                            &error,
                            item.attempt_count,
                        )
                        .await?;
                    } else {
                        raw_crawl_adapter::mark_crawl_failed(
                            self.pool,
                            &item.url_norm,
                            Some(fetched.status_code),
                            &error,
                        )
                        .await?;
                    }
                    failed_count += 1;
                    failed_urls.push(item.url.clone());
                }
                Err(err) => {
                    let error = format!("fetch failed: {err}");
                    if raw_crawl_adapter::should_retry_crawl_attempt(item.attempt_count) {
                        raw_crawl_adapter::mark_crawl_retry(
                            self.pool,
                            &item.url_norm,
                            None,
                            &error,
                            item.attempt_count,
                        )
                        .await?;
                    } else {
                        raw_crawl_adapter::mark_crawl_failed(
                            self.pool,
                            &item.url_norm,
                            None,
                            &error,
                        )
                        .await?;
                    }
                    failed_count += 1;
                    failed_urls.push(item.url.clone());
                }
            }
        }

        Ok(CrawlSourcesOutputPayload {
            claimed_count,
            crawled_count,
            failed_count,
            raw_page_count,
            raw_section_count,
            qdrant_event_count,
            status: if failed_count > 0 {
                "partial".to_string()
            } else {
                "done".to_string()
            },
            raw_page_ids,
            failed_urls,
        })
    }

    async fn ingest_raw_knowledge(
        &self,
        input: &RawKnowledgeIngestionInputPayload,
    ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            self.pool,
            &input.run_id,
            &input.context_key,
            &input.raw_page_ids,
        )
        .await?;
        Ok(RawKnowledgeIngestionOutputPayload {
            raw_page_count: report.raw_page_count as u32,
            raw_section_count: report.raw_section_count as u32,
            extracted_rule_count: report.extracted_rule_count as u32,
            verified_rule_count: report.verified_rule_count as u32,
            outbox_event_count: report.outbox_event_count as u32,
            changed_truth_keys: report.changed_truth_keys,
            status: if input.raw_page_ids.is_empty() {
                "skipped:no_raw_pages".to_string()
            } else if report.extraction_provider_unavailable {
                "blocked:no_truth_extraction_provider".to_string()
            } else if report.needs_hitl_candidate_count > 0 {
                "pending_review:needs_truth_adjudication".to_string()
            } else if report.verified_rule_count == 0 {
                "empty:no_admissible_verified_rules".to_string()
            } else {
                "done".to_string()
            },
        })
    }
}

