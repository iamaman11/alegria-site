async fn run_initial_phase<
    R: VerifiedSupportRepository + PlanningRepository + CrawlIngestRepository + SerpSearchPort,
>(
    repo: &R,
    request: &SeoScenarioRequest,
    state: &mut ScenarioState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<(), DomainError> {
    let run_id = request.site_input.run_id.clone();
    match key {
        SeoPhaseKey::LoadVerifiedSupportBundle => {
            state.verified_support = seo_runtime::load_verified_support_bundle(
                repo,
                &VerifiedSupportBundleRequest {
                    run_id,
                    context_key: request.site_input.context_key.clone(),
                    scope_signature: scope_signature(&request.site_input),
                    applicant_profile: applicant_profile(&request.site_input),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                if state.verified_support.is_empty() {
                    "empty"
                } else {
                    "done"
                },
                "",
                format!("support_count={}", state.verified_support.len()),
            ));
        }
        SeoPhaseKey::SerpIngest => {
            let output = planning::run_serp_ingest(
                repo,
                repo,
                &SerpIngestInputPayload {
                    run_id,
                    query_batch_key: request.site_input.query_batch_key.clone(),
                    scope: request.site_input.scope.clone(),
                    queries: request.site_input.queries.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!(
                    "persisted_snapshot_count={}",
                    output.persisted_snapshot_count
                ),
            ));
            state.ingest = Some(output);
        }
        SeoPhaseKey::CrawlSources => {
            let ingest = require_state_ref(&state.ingest, "serp_ingest_required")?;
            let output = crawl_ingest::run_crawl_sources(
                repo,
                &CrawlSourcesInputPayload {
                    run_id,
                    query_batch_key: ingest.query_batch_key.clone(),
                    limit: 25,
                    emit_qdrant: true,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                "",
                format!(
                    "claimed={} crawled={} failed={} raw_pages={}",
                    output.claimed_count,
                    output.crawled_count,
                    output.failed_count,
                    output.raw_page_count
                ),
            ));
            state.crawl_sources = Some(output);
        }
        SeoPhaseKey::RawKnowledgeIngestion => {
            let ingest = require_state_ref(&state.ingest, "serp_ingest_required")?;
            let crawl_sources = require_state_ref(&state.crawl_sources, "crawl_sources_required")?;
            let output = crawl_ingest::run_raw_knowledge_ingestion(
                repo,
                &RawKnowledgeIngestionInputPayload {
                    run_id,
                    context_key: request.site_input.context_key.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    raw_page_ids: crawl_sources.raw_page_ids.clone(),
                    source_policy: "candidate_only_truth_extraction@1".to_string(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                "",
                format!(
                    "verified_rules={} changed_truth_keys={}",
                    output.verified_rule_count,
                    output.changed_truth_keys.len()
                ),
            ));
            state.raw_knowledge = Some(output);
        }
        _ => unreachable!("invalid initial phase"),
    }
    Ok(())
}

