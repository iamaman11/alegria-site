use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DraftAssembleInputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, EditorialDraftGenerateInputPayload,
    EditorialDraftGenerateOutputPayload, FinalizePublishInputPayload, FinalizePublishOutputPayload,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, HitlPauseInfo,
    HitlResolutionInput, IaBuildInputPayload, IaBuildOutputPayload, LinkRecommendInputPayload,
    LinkRecommendOutputPayload, LinkRecommendationState, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    RawKnowledgeIngestionInputPayload, RawKnowledgeIngestionOutputPayload,
    RebuildDetectInputPayload, RebuildDetectOutputPayload, RenderPreviewValidateInputPayload,
    RenderPreviewValidateOutputPayload, SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    activities, ActivityContext, ActivityError,
};
use infrastructure::adapters::{
    dataforseo_serp_adapter, editorial_llm_adapter, raw_crawl_adapter, semantic_search_adapter,
    sqlx_adapter::AlegriaPgPool, sqlx_seo_adapter, sqlx_seo_cms_adapter, sqlx_serp_adapter,
};
use primitives::errors::DomainError;

mod content_generation;
mod fact_extraction;
mod operations;
mod runtime;
mod step_catalog;

pub struct AlegriaActivities {
    pub pool: Arc<AlegriaPgPool>,
}

fn linkable_state(state: &str) -> bool {
    !matches!(state, "blocked" | "deprecated" | "stale" | "needs_rebuild")
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn strict_projection_barrier_enabled() -> bool {
    std::env::var("SEO_STRICT_PROJECTION_BARRIER")
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "strict")
        })
        .unwrap_or(false)
}

async fn observe_projection_barrier(
    pool: &AlegriaPgPool,
    checkpoint: &str,
) -> Result<(), DomainError> {
    let statuses = sqlx_seo_adapter::read_projection_sync_status(pool).await?;
    let blocked_events: i64 = statuses
        .iter()
        .map(|status| status.blocking_event_count())
        .sum();
    let max_lag_ms = statuses
        .iter()
        .map(|status| status.max_open_lag_ms)
        .max()
        .unwrap_or(0);
    if blocked_events == 0 {
        tracing::info!(checkpoint, "projection barrier clear");
        return Ok(());
    }

    for status in &statuses {
        if status.blocking_event_count() == 0 {
            continue;
        }
        tracing::warn!(
            checkpoint,
            target_system = %status.target_system,
            pending_events = status.pending_events,
            processing_events = status.processing_events,
            failed_events = status.failed_events,
            max_open_lag_ms = status.max_open_lag_ms,
            oldest_open_aggregate_key = status.oldest_open_aggregate_key.as_deref().unwrap_or(""),
            latest_failed_aggregate_key = status.latest_failed_aggregate_key.as_deref().unwrap_or(""),
            "projection barrier has open or failed events"
        );
    }

    if strict_projection_barrier_enabled() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "projection barrier blocked at {checkpoint}: blocked_events={blocked_events}, max_lag_ms={max_lag_ms}"
            ),
        });
    }
    Ok(())
}

async fn enrich_semantic_link_recommendations(
    input: &LinkRecommendInputPayload,
    output: &mut LinkRecommendOutputPayload,
) {
    if std::env::var("VOYAGE_API_KEY").is_err() {
        return;
    }
    let max_semantic_links = input.max_links_per_page.max(3) as usize;
    let node_by_key = input
        .page_nodes
        .iter()
        .cloned()
        .map(|node| (node.page_node_key.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut seen = output
        .link_recommendations
        .iter()
        .map(|link| (link.source_page_key.clone(), link.target_page_key.clone()))
        .collect::<HashSet<_>>();

    for source in &input.page_nodes {
        if !linkable_state(&source.lifecycle_state) {
            continue;
        }
        let query = format!(
            "{} {} {} {}",
            source.canonical_url_path,
            source.page_type_key,
            source.dominant_intent,
            source.menu_group
        );
        let Ok(results) =
            semantic_search_adapter::search_by_text(&query, "seo_link_targets", 12).await
        else {
            continue;
        };
        let mut added = 0usize;
        for candidate in results {
            if added >= max_semantic_links {
                break;
            }
            let Some(target) = node_by_key.get(&candidate.entity_key) else {
                continue;
            };
            if source.page_node_key == target.page_node_key
                || !linkable_state(&target.lifecycle_state)
                || seen.contains(&(source.page_node_key.clone(), target.page_node_key.clone()))
            {
                continue;
            }
            let same_scope = source.scope_signature == target.scope_signature;
            let same_family = source.canonical_url_family == target.canonical_url_family;
            if !same_scope && !same_family {
                continue;
            }
            let journey_bonus = if source.parent_page_node_key == target.page_node_key
                || target.parent_page_node_key == source.page_node_key
            {
                0.12
            } else {
                0.0
            };
            let hub_bonus =
                if source.page_type_key.contains("hub") || target.page_type_key.contains("hub") {
                    0.08
                } else {
                    0.0
                };
            let orphan_bonus = if target.parent_page_node_key.is_empty() {
                0.05
            } else {
                0.0
            };
            let anchor_strategy = if journey_bonus > 0.0 {
                "journey_contextual"
            } else {
                "semantic_contextual"
            };
            let link_role = if same_family {
                "semantic_family"
            } else {
                "semantic_contextual"
            };
            let semantic_score = clamp01(
                (candidate.score as f64 * 0.7)
                    + if same_scope { 0.12 } else { 0.0 }
                    + if same_family { 0.08 } else { 0.0 }
                    + journey_bonus
                    + hub_bonus
                    + orphan_bonus,
            );
            output.link_recommendations.push(LinkRecommendationState {
                link_recommendation_key: primitives::seo::seo_artifact_key(
                    "link_recommendation",
                    &[
                        &source.scope_signature,
                        &source.page_node_key,
                        &target.page_node_key,
                        link_role,
                        anchor_strategy,
                        "semantic@1",
                    ],
                ),
                scope_signature: source.scope_signature.clone(),
                source_page_key: source.page_node_key.clone(),
                target_page_key: target.page_node_key.clone(),
                link_role: link_role.to_string(),
                anchor_strategy: anchor_strategy.to_string(),
                required_flag: false,
                score: semantic_score,
                status: "candidate".to_string(),
            });
            seen.insert((source.page_node_key.clone(), target.page_node_key.clone()));
            added += 1;
        }
    }
}

#[allow(dead_code)]
#[activities]
impl AlegriaActivities {
    #[activity]
    pub async fn extract_facts(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "extract_facts", 1, &run_id, || async {
            fact_extraction::extract_facts_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn verify_rules(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "verify_rules", 1, &run_id, || async {
            fact_extraction::verify_rules_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn prepare_hitl_pause(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<HitlPauseInfo, ActivityError> {
        self.execute_step(&run_id, "prepare_hitl_pause", 1, &run_id, || async {
            fact_extraction::prepare_hitl_pause_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn apply_hitl_resolution(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: HitlResolutionInput,
    ) -> Result<String, ActivityError> {
        let run_id = input
            .meta
            .as_ref()
            .map(|m| m.run_id.clone())
            .unwrap_or_default();
        self.execute_step(&run_id, "apply_hitl_resolution", 1, &input, || async {
            fact_extraction::apply_hitl_resolution_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[activity]
    pub async fn persist_and_emit(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "persist_and_emit", 1, &run_id, || async {
            fact_extraction::persist_and_emit_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn generate_content(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "generate_content", 1, &run_id, || async {
            content_generation::generate_content_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn validate_blocks(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "validate_blocks", 1, &run_id, || async {
            content_generation::validate_blocks_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_layer_router_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::layer_router_step::LayerRouterInput,
    ) -> Result<use_cases::layer_router_step::LayerRouterOutput, ActivityError> {
        Ok(step_catalog::run_layer_router(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_entity_span_detection_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::entity_span_detection_step::EntitySpanInput,
    ) -> Result<use_cases::entity_span_detection_step::EntitySpanOutput, ActivityError> {
        Ok(step_catalog::run_entity_span_detection(
            self.as_ref(),
            &input,
        ))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_canonical_mapping_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::canonical_mapping_step::CanonicalMappingInput,
    ) -> Result<use_cases::canonical_mapping_step::CanonicalMappingOutput, ActivityError> {
        Ok(step_catalog::run_canonical_mapping(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_procedural_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::procedural_extraction_step::ProceduralExtractionInput,
    ) -> Result<use_cases::procedural_extraction_step::ProceduralExtractionOutput, ActivityError>
    {
        Ok(step_catalog::run_procedural_extraction(
            self.as_ref(),
            &input,
        ))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_operational_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::operational_extraction_step::OperationalExtractionInput,
    ) -> Result<use_cases::operational_extraction_step::OperationalExtractionOutput, ActivityError>
    {
        Ok(step_catalog::run_operational_extraction(
            self.as_ref(),
            &input,
        ))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::editorial_extraction_step::EditorialExtractionInput,
    ) -> Result<use_cases::editorial_extraction_step::EditorialExtractionOutput, ActivityError>
    {
        Ok(step_catalog::run_editorial_extraction(
            self.as_ref(),
            &input,
        ))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_triple_builder_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::triple_builder_step::TripleBuilderInput,
    ) -> Result<use_cases::triple_builder_step::TripleBuilderOutput, ActivityError> {
        Ok(step_catalog::run_triple_builder(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_completeness_judge_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::completeness_judge_step::CompletenessJudgeInput,
    ) -> Result<use_cases::completeness_judge_step::CompletenessJudgeOutput, ActivityError> {
        Ok(step_catalog::run_completeness_judge(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_contradiction_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::contradiction_gate_step::ContradictionGateInput,
    ) -> Result<use_cases::contradiction_gate_step::ContradictionGateOutput, ActivityError> {
        Ok(step_catalog::run_contradiction_gate(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_hitl_decision_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::hitl_decision_step::HitlDecisionInput,
    ) -> Result<use_cases::hitl_decision_step::HitlDecisionOutput, ActivityError> {
        Ok(step_catalog::run_hitl_decision(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_neo4j_backwrite_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::neo4j_backwrite_step::Neo4jBackwriteInput,
    ) -> Result<use_cases::neo4j_backwrite_step::Neo4jBackwriteOutput, ActivityError> {
        step_catalog::run_neo4j_backwrite(self.as_ref(), &input)
            .await
            .map_err(Self::into_activity_error)
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_seo_site_build_input(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<SeoSiteBuildInputPayload, ActivityError> {
        self.execute_step(&run_id, "load_seo_site_build_input", 1, &run_id, || async {
            sqlx_seo_adapter::load_seo_site_build_input(&self.pool, &run_id).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_verified_support_bundle(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_input: String,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, ActivityError> {
        let mut parts = workflow_input.splitn(3, '|');
        let run_id = parts.next().unwrap_or_default().to_string();
        let context_key = parts.next().unwrap_or_default().to_string();
        let scope_signature = parts.next().unwrap_or_default().to_string();
        let timer = crate::metrics::ActivityTimer::start("load_verified_support_bundle");
        match sqlx_seo_adapter::load_verified_support_bundle(
            &self.pool,
            &run_id,
            &context_key,
            &scope_signature,
        )
        .await
        {
            Ok(bundle) => {
                crate::metrics::global()
                    .support_bundle_events_total
                    .with_label_values(&[if bundle.is_empty() { "empty" } else { "loaded" }])
                    .inc();
                timer.record_success();
                Ok(bundle)
            }
            Err(err) => {
                let class = err.class();
                let error_text = format!("{err:?}");
                let outcome = if error_text.contains("empty") {
                    "empty"
                } else {
                    "failed"
                };
                crate::metrics::global()
                    .support_bundle_events_total
                    .with_label_values(&[outcome])
                    .inc();
                timer.record_failure(class.as_str());
                if outcome == "empty" {
                    return Ok(Vec::new());
                }
                Err(Self::activity_error_from_domain(&err))
            }
        }
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_serp_ingest_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: SerpIngestInputPayload,
    ) -> Result<SerpIngestOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "serp_ingest", 1, &input, || async {
            let mut output = step_catalog::run_serp_ingest(self.as_ref(), &input);
            if let Some(config) = dataforseo_serp_adapter::DataForSeoConfig::from_env_with_locale(
                input.scope.as_ref().map(|scope| scope.locale.as_str()),
            ) {
                let client = dataforseo_serp_adapter::DataForSeoSerpClient::from_config(config)
                    .map_err(|err| DomainError::InfraUnavailable {
                        message: format!("configure DataForSEO client: {err}"),
                    })?;
                let mut persisted_snapshots = 0u32;
                for (idx, query) in input
                    .queries
                    .iter()
                    .map(|query| query.trim())
                    .filter(|query| !query.is_empty())
                    .enumerate()
                {
                    let job_id = primitives::hash::content_hash_v1(&format!(
                        "{}|{}|{}|{}",
                        input.run_id, output.query_batch_key, idx, query
                    ));
                    let response =
                        client
                            .google_organic_live_advanced(query)
                            .await
                            .map_err(|err| DomainError::InfraUnavailable {
                                message: format!("DataForSEO organic live advanced failed: {err}"),
                            })?;
                    sqlx_serp_adapter::save_raw_snapshot(
                        &self.pool,
                        &sqlx_serp_adapter::RawSnapshotRecord {
                            run_id: input.run_id.clone(),
                            job_id: job_id.clone(),
                            url_norm: query.to_string(),
                            query: query.to_string(),
                            raw_payload_utf8: response.raw_payload_utf8,
                        },
                    )
                    .await
                    .map_err(|err| DomainError::InfraUnavailable {
                        message: format!("save DataForSEO raw snapshot: {err}"),
                    })?;
                    sqlx_serp_adapter::save_dataforseo_organic_results_and_enqueue(
                        &self.pool,
                        &input.run_id,
                        &job_id,
                        &output.query_batch_key,
                        &response.organic_results,
                    )
                    .await
                    .map_err(|err| DomainError::InfraUnavailable {
                        message: format!("save DataForSEO organic results: {err}"),
                    })?;
                    persisted_snapshots += 1;
                }
                output.persisted_snapshot_count = persisted_snapshots;
            }
            sqlx_seo_adapter::persist_serp_ingest_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_crawl_sources_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CrawlSourcesInputPayload,
    ) -> Result<CrawlSourcesOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "crawl_sources", 1, &input, || async {
            let limit = if input.limit == 0 { 25 } else { input.limit } as i64;
            let items = raw_crawl_adapter::claim_pending_crawl_batch(
                &self.pool,
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
                match raw_crawl_adapter::fetch_html(&item.url).await {
                    Ok(fetched) if (200..400).contains(&fetched.status_code) => {
                        match raw_crawl_adapter::save_crawled_html(
                            &self.pool,
                            &fetched.final_url,
                            &item.dtype,
                            fetched.status_code,
                            &fetched.content_type,
                            &fetched.body,
                        )
                        .await
                        {
                            Ok(saved) => {
                                let emitted = if input.emit_qdrant {
                                    raw_crawl_adapter::emit_raw_section_qdrant_events(
                                        &self.pool,
                                        saved.page_id,
                                    )
                                    .await?
                                } else {
                                    0
                                };
                                raw_crawl_adapter::mark_crawl_done(
                                    &self.pool,
                                    &item.url_norm,
                                    fetched.status_code,
                                    &format!(
                                        "source_type={}; page_id={}; sections={}; qdrant_events={}; hash={}",
                                        item.source_type,
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
                                    &self.pool,
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
                        raw_crawl_adapter::mark_crawl_failed(
                            &self.pool,
                            &item.url_norm,
                            Some(fetched.status_code),
                            &format!(
                                "http_status={}; content_type={}",
                                fetched.status_code, fetched.content_type
                            ),
                        )
                        .await?;
                        failed_count += 1;
                        failed_urls.push(item.url.clone());
                    }
                    Err(err) => {
                        raw_crawl_adapter::mark_crawl_failed(
                            &self.pool,
                            &item.url_norm,
                            None,
                            &format!("fetch failed: {err}"),
                        )
                        .await?;
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
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_raw_knowledge_ingestion_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RawKnowledgeIngestionInputPayload,
    ) -> Result<RawKnowledgeIngestionOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "raw_knowledge_ingestion", 1, &input, || async {
            let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
                &self.pool,
                &input.context_key,
                &input.raw_page_ids,
            )
            .await?;
            observe_projection_barrier(&self.pool, "raw_knowledge_ingestion").await?;
            Ok(RawKnowledgeIngestionOutputPayload {
                raw_page_count: report.raw_page_count as u32,
                raw_section_count: report.raw_section_count as u32,
                extracted_rule_count: report.extracted_rule_count as u32,
                verified_rule_count: report.verified_rule_count as u32,
                outbox_event_count: report.outbox_event_count as u32,
                changed_truth_keys: report.changed_truth_keys,
                status: if input.raw_page_ids.is_empty() {
                    "skipped:no_raw_pages".to_string()
                } else if report.extracted_rule_count > 0 && report.verified_rule_count == 0 {
                    "pending_review:no_auto_verified_rules".to_string()
                } else if report.verified_rule_count == 0 {
                    "empty:no_verified_rules".to_string()
                } else {
                    "done".to_string()
                },
            })
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_serp_normalize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: SerpNormalizeInputPayload,
    ) -> Result<SerpNormalizeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "serp_normalize", 1, &input, || async {
            let output = step_catalog::run_serp_normalize(self.as_ref(), &input);
            sqlx_seo_adapter::persist_serp_normalize_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_opportunity_build_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: OpportunityBuildInputPayload,
    ) -> Result<OpportunityBuildOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "opportunity_build", 1, &input, || async {
            let output = step_catalog::run_opportunity_build(self.as_ref(), &input);
            sqlx_seo_adapter::persist_opportunity_build_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_ia_build_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: IaBuildInputPayload,
    ) -> Result<IaBuildOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "ia_build", 1, &input, || async {
            let output = step_catalog::run_ia_build(self.as_ref(), &input);
            sqlx_seo_adapter::persist_ia_build_output(&self.pool, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_link_recommend_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: LinkRecommendInputPayload,
    ) -> Result<LinkRecommendOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "link_recommend", 1, &input, || async {
            let mut output = step_catalog::run_link_recommend(self.as_ref(), &input);
            enrich_semantic_link_recommendations(&input, &mut output).await;
            sqlx_seo_adapter::persist_link_recommend_output(&self.pool, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_global_site_reconcile_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: GlobalSiteReconcileInputPayload,
    ) -> Result<GlobalSiteReconcileOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "global_site_reconcile", 1, &input, || async {
            let output = step_catalog::run_global_site_reconcile(self.as_ref(), &input);
            sqlx_seo_adapter::persist_ia_build_output(
                &self.pool,
                &IaBuildOutputPayload {
                    page_nodes: output.page_nodes.clone(),
                    page_blueprints: Vec::new(),
                    cannibalization_conflicts: Vec::new(),
                },
            )
            .await?;
            sqlx_seo_adapter::persist_link_recommend_output(
                &self.pool,
                &LinkRecommendOutputPayload {
                    link_recommendations: output.link_recommendations.clone(),
                },
            )
            .await?;
            let scope = input.scope.as_ref().cloned().unwrap_or_default();
            let navigation = sqlx_seo_adapter::persist_global_navigation_from_active_pages(
                &self.pool,
                &scope.market,
                &scope.locale,
                &input.reconcile_reason,
            )
            .await?;
            tracing::info!(
                navigation_tree_key = %navigation.navigation_tree_key,
                scope_count = navigation.scope_count,
                page_item_count = navigation.page_item_count,
                silo_group_count = navigation.silo_group_count,
                rebuild_plan_count = navigation.rebuild_plan_count,
                "global navigation reconciled"
            );
            observe_projection_barrier(&self.pool, "global_site_reconcile").await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_assemble_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftAssembleInputPayload,
    ) -> Result<DraftAssembleOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_assemble", 1, &input, || async {
            let mut enriched_input = input.clone();
            if enriched_input.section_templates.is_empty() {
                if let Some(blueprint) = enriched_input.page_blueprint.as_ref() {
                    enriched_input.section_templates = sqlx_seo_adapter::load_section_templates(
                        &self.pool,
                        &blueprint.page_type_key,
                        &blueprint.dominant_intent,
                    )
                    .await?;
                }
            }
            if enriched_input.source_context_chunks.is_empty() {
                let page_node = enriched_input.page_node.clone().unwrap_or_default();
                let page_blueprint = enriched_input.page_blueprint.clone().unwrap_or_default();
                let query = format!(
                    "{} {} {} {} {}",
                    page_node.canonical_url_path,
                    page_node.canonical_slug,
                    page_blueprint.page_type_key,
                    page_blueprint.dominant_intent,
                    enriched_input
                        .verified_support
                        .iter()
                        .take(8)
                        .map(|support| support.fragment_text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                enriched_input.source_context_chunks =
                    raw_crawl_adapter::retrieve_source_context_chunks(&self.pool, &query, 12)
                        .await?;
            }
            let output = step_catalog::run_draft_assemble(self.as_ref(), &enriched_input);
            sqlx_seo_adapter::persist_draft_assemble_output(&self.pool, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_normalize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftNormalizeInputPayload,
    ) -> Result<DraftNormalizeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_normalize", 1, &input, || async {
            let output = step_catalog::run_draft_normalize(self.as_ref(), &input);
            sqlx_seo_adapter::persist_draft_normalize_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_draft_generate(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "editorial_draft_generate", 1, &input, || async {
            let output = editorial_llm_adapter::generate_editorial_draft(&input).await?;
            let outcome = if output.provider_key == "deterministic_fixture"
                && output.status.contains("fallback")
            {
                "fallback"
            } else {
                "selected"
            };
            crate::metrics::global()
                .editorial_provider_events_total
                .with_label_values(&[&output.provider_key, outcome])
                .inc();
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_content_contract_validate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: ContentContractValidateInputPayload,
    ) -> Result<ContentContractValidateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "content_contract_validate", 1, &input, || async {
            let output = step_catalog::run_content_contract_validate(self.as_ref(), &input);
            sqlx_seo_adapter::persist_content_contract_validate_output(&self.pool, &input, &output)
                .await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_qa_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftQaInputPayload,
    ) -> Result<DraftQaOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_qa", 1, &input, || async {
            let output = step_catalog::run_draft_qa(self.as_ref(), &input);
            sqlx_seo_adapter::persist_draft_qa_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_cms_publish_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CmsPublishInputPayload,
    ) -> Result<CmsPublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "cms_publish", 1, &input, || async {
            let output = step_catalog::run_cms_publish(self.as_ref(), &input);
            sqlx_seo_cms_adapter::persist_cms_publish_output(&self.pool, &input, &output).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_cms_approval_decision(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_input: String,
    ) -> Result<CmsApprovalDecision, ActivityError> {
        let mut parts = workflow_input.splitn(3, '|');
        let run_id = parts.next().unwrap_or_default().to_string();
        let page_node_key = parts.next().unwrap_or_default().to_string();
        let revision_id = parts.next().unwrap_or_default().to_string();
        self.execute_step(
            &run_id,
            "load_cms_approval_decision",
            1,
            &workflow_input,
            || async {
                sqlx_seo_cms_adapter::load_latest_approval_decision(
                    &self.pool,
                    &page_node_key,
                    &revision_id,
                )
                .await?
                .ok_or_else(|| {
                    primitives::errors::DomainError::ValidationFailure {
                        message: "cms approval decision not found".to_string(),
                    }
                })
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_rebuild_detect_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RebuildDetectInputPayload,
    ) -> Result<RebuildDetectOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "rebuild_detect", 1, &input, || async {
            let mut narrowed_input = input.clone();
            if !input.changed_truth_keys.is_empty() {
                let impacted = sqlx_seo_adapter::resolve_rebuild_impacts(
                    &self.pool,
                    &input.changed_truth_keys,
                )
                .await?;
                if !impacted.is_empty() {
                    let impacted_keys = impacted
                        .iter()
                        .map(|(page_node_key, _, _, _)| page_node_key.clone())
                        .collect::<HashSet<_>>();
                    narrowed_input.page_nodes = input
                        .page_nodes
                        .iter()
                        .filter(|page| impacted_keys.contains(&page.page_node_key))
                        .cloned()
                        .collect();
                }
            }
            let output = step_catalog::run_rebuild_detect(self.as_ref(), &narrowed_input);
            sqlx_seo_adapter::persist_rebuild_detect_output(&self.pool, &input, &output).await?;
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_publish_materialize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: PublishMaterializeInputPayload,
    ) -> Result<PublishMaterializeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "publish_materialize", 1, &input, || async {
            let output = step_catalog::run_publish_materialize(self.as_ref(), &input);
            let persisted = sqlx_seo_cms_adapter::persist_publish_materialize_output(
                &self.pool, &input, &output,
            )
            .await?;
            let manifest_json = persisted
                .publish_artifact
                .as_ref()
                .map(|artifact| artifact.manifest_json.as_str())
                .unwrap_or("");
            let build_scope = if manifest_json.contains("\"build_scope\":\"site\"")
                || manifest_json.contains("\"build_scope\": \"site\"")
            {
                "site"
            } else {
                "page"
            };
            let outcome = if persisted.materialization_status.contains("failed") {
                "failed"
            } else {
                "built"
            };
            crate::metrics::global()
                .publish_build_scope_total
                .with_label_values(&[build_scope, outcome])
                .inc();
            Ok(persisted)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_render_preview_validate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RenderPreviewValidateInputPayload,
    ) -> Result<RenderPreviewValidateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "render_preview_validate", 1, &input, || async {
            let output = step_catalog::run_render_preview_validate(self.as_ref(), &input);
            if output.verdict != "render_ready" {
                if output.blocking_reasons.is_empty() {
                    crate::metrics::global()
                        .render_validation_failures_total
                        .with_label_values(&["blocked"])
                        .inc();
                } else {
                    for reason in &output.blocking_reasons {
                        crate::metrics::global()
                            .render_validation_failures_total
                            .with_label_values(&[reason])
                            .inc();
                    }
                }
            }
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_finalize_publish_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: FinalizePublishInputPayload,
    ) -> Result<FinalizePublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "finalize_publish", 1, &input, || async {
            let output = step_catalog::run_finalize_publish(self.as_ref(), &input);
            sqlx_seo_cms_adapter::persist_finalize_publish_output(&self.pool, &input, &output).await
        })
        .await
    }

    #[activity]
    pub async fn test_step_prepare(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_id: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_prepare_impl(&workflow_id))
    }

    #[activity]
    pub async fn test_step_finalize(
        self: Arc<Self>,
        _ctx: ActivityContext,
        prepared_token: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_finalize_impl(&prepared_token))
    }

    #[activity]
    pub async fn finalize_run(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "finalize_run", 1, &run_id, || async {
            content_generation::finalize_run_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn check_data_freshness(
        self: Arc<Self>,
        _ctx: ActivityContext,
        threshold_input: String,
    ) -> Result<String, ActivityError> {
        let timer = crate::metrics::ActivityTimer::start("check_data_freshness");
        match operations::check_data_freshness_impl(self.as_ref(), &threshold_input).await {
            Ok(report) => {
                timer.record_success();
                Ok(report)
            }
            Err(err) => {
                let class = err.class();
                timer.record_failure(class.as_str());
                Err(Self::activity_error_from_domain(&err))
            }
        }
    }
}
