use super::*;

/// The source stage consumes an explicit scope and produces only evidence
/// receipts. Later semantic/truth/planning stages use these typed outputs,
/// without assuming that acquisition is the owner's whole site-building plan.
pub(super) struct SourceAcquisitionResult {
    pub(super) ingest: SerpIngestOutputPayload,
    pub(super) crawled: CrawlSourcesOutputPayload,
}

pub(super) async fn acquire_source_evidence(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    site_input: &SeoSiteBuildInputPayload,
    scope: &SeoScopePayload,
) -> WorkflowResult<SourceAcquisitionResult> {
    ctx.state_mut(|s| s.phase = "serp_ingest".to_string());
    let ingest = ctx
        .start_activity(
            AlegriaActivities::run_serp_ingest_step,
            SerpIngestInputPayload {
                run_id: run_id.to_string(),
                query_batch_key: site_input.query_batch_key.clone(),
                scope: Some(scope.clone()),
                queries: site_input.queries.clone(),
            },
            db_opts(1800),
        )
        .await?;
    ctx.wait_condition(|s| !s.paused).await?;

    ctx.state_mut(|s| s.phase = "crawl_sources".to_string());
    let mut crawl_input = CrawlSourcesInputPayload {
        run_id: run_id.to_string(),
        query_batch_key: ingest.query_batch_key.clone(),
        limit: 5,
        emit_qdrant: true,
        sweep_index: 0,
    };
    let mut crawled = ctx
        .start_activity(
            AlegriaActivities::run_crawl_sources_step,
            crawl_input.clone(),
            db_opts(600),
        )
        .await?;
    ctx.wait_condition(|s| !s.paused).await?;
    // Keep one durable workflow and claim all URLs, including multiple pages
    // from the same domain. A single 25-item activity is not full coverage.
    while crawled.status == "more" || crawled.status == "pending_retry" {
        if crawled.status == "pending_retry" {
            // A durable Temporal timer survives worker restarts and delayed retries.
            // Do not end the workflow or start a second coordinator for pending URLs.
            ctx.timer(Duration::from_secs(60)).await;
            ctx.wait_condition(|s| !s.paused).await?;
        }
        advance_crawl_sweep(&mut crawl_input)?;
        let next = ctx
            .start_activity(
                AlegriaActivities::run_crawl_sources_step,
                crawl_input.clone(),
                db_opts(600),
            )
            .await?;
        if crawled.status == "more" && next.claimed_count == 0 {
            return Err(super::super::runtime::workflow_contract_failure(
                "crawl backlog is not claimable",
            ));
        }
        crawled.claimed_count = crawled.claimed_count.saturating_add(next.claimed_count);
        crawled.crawled_count = crawled.crawled_count.saturating_add(next.crawled_count);
        crawled.failed_count = crawled.failed_count.saturating_add(next.failed_count);
        crawled.raw_page_count = crawled.raw_page_count.saturating_add(next.raw_page_count);
        crawled.raw_section_count = crawled
            .raw_section_count
            .saturating_add(next.raw_section_count);
        crawled.qdrant_event_count = crawled
            .qdrant_event_count
            .saturating_add(next.qdrant_event_count);
        crawled.raw_page_ids.extend(next.raw_page_ids);
        crawled.failed_urls.extend(next.failed_urls);
        crawled.status = next.status;
        ctx.wait_condition(|s| !s.paused).await?;
    }
    if crawled.status != "done" && crawled.status != "partial" {
        return Err(super::super::runtime::workflow_contract_failure(format!(
            "unexpected crawl completion status: {}",
            crawled.status
        )));
    }
    if crawled.failed_count > 0 {
        crawled.status = "partial".to_string();
    }

    Ok(SourceAcquisitionResult { ingest, crawled })
}

// Every claimed batch has a different persisted step identity. Reusing the
// same input would replay the first completed step and spin the workflow.
pub(super) fn advance_crawl_sweep(input: &mut CrawlSourcesInputPayload) -> WorkflowResult<()> {
    input.sweep_index = input.sweep_index.checked_add(1).ok_or_else(|| {
        super::super::runtime::workflow_contract_failure("crawl sweep index exhausted")
    })?;
    Ok(())
}
