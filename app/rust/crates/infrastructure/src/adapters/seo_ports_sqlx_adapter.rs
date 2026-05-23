use async_trait::async_trait;
use contracts::generated::alegria::read_api::v1::{
    citation_fact, AppointmentRuleParams, CitationFact, ContextBundle, DocumentRequiredParams,
    EligibilityRuleParams, FeeItemParams, FormRequiredParams, RuleRoleTypeV1, SourceCitation,
    StepParams, TimelineItemParams, WhereToApplyParams,
};
use primitives::errors::DomainError;
use runtime_models::RuleParams;
use seo_domain::identity;
use seo_ports::{
    CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest, CmsReviewPort,
    ContextBundleRepository, CrawlIngestRepository, DraftRepository, EditorialGenerationPort,
    GlobalNavigationPersistReport, HitlQueuePort, OrganicSerpResponse, OrganicSerpResult,
    PlanningRepository, ProjectionBarrierStatus, ProjectionStatusRepository,
    PublishArtifactRepository, RebuildDependencyEvidence, RebuildRepository,
    SectionTemplateRepository, SemanticLinkCandidate, SemanticLinkSearchPort,
    SeoBuildInputRepository, SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest,
    SerpSearchPort, SourceContextRepository, VerifiedSupportBundleRequest,
    VerifiedSupportRepository,
};

use super::{
    dataforseo_serp_adapter, editorial_llm_adapter, raw_crawl_adapter, semantic_search_adapter,
    sqlx_adapter::AlegriaPgPool, sqlx_context_bundle_adapter, sqlx_hitl_adapter, sqlx_seo_adapter,
    sqlx_seo_cms_adapter, sqlx_serp_adapter,
};
use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DraftAssembleOutputPayload,
    DraftNormalizeInputPayload, DraftNormalizeOutputPayload, DraftQaInputPayload,
    DraftQaOutputPayload, EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, GlobalSiteReconcileInputPayload,
    GlobalSiteReconcileOutputPayload, HitlDecision, HitlTaskContext, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload,
    SectionTemplateBinding, SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload, SourceContextChunkState,
};
use uuid::Uuid;

pub struct SqlxSeoRuntimeRepository<'a> {
    pool: &'a AlegriaPgPool,
}

impl<'a> SqlxSeoRuntimeRepository<'a> {
    pub fn new(pool: &'a AlegriaPgPool) -> Self {
        Self { pool }
    }
}

fn read_role_type(role_type: &str) -> i32 {
    match role_type {
        "must_provide" => RuleRoleTypeV1::MustProvide as i32,
        "must_pay" => RuleRoleTypeV1::MustPay as i32,
        "must_satisfy" => RuleRoleTypeV1::MustSatisfy as i32,
        "allows" => RuleRoleTypeV1::Allows as i32,
        "forbids" => RuleRoleTypeV1::Forbids as i32,
        "timeline" => RuleRoleTypeV1::Timeline as i32,
        "document_required" => RuleRoleTypeV1::DocumentRequired as i32,
        "eligibility_rule" => RuleRoleTypeV1::EligibilityRule as i32,
        "fee_item" => RuleRoleTypeV1::FeeItem as i32,
        "timeline_item" => RuleRoleTypeV1::TimelineItem as i32,
        "where_to_apply" => RuleRoleTypeV1::WhereToApply as i32,
        "appointment_rule" => RuleRoleTypeV1::AppointmentRule as i32,
        "form_required" => RuleRoleTypeV1::FormRequired as i32,
        "step" => RuleRoleTypeV1::Step as i32,
        _ => RuleRoleTypeV1::Unspecified as i32,
    }
}

fn map_params(params: &RuleParams) -> Option<citation_fact::Params> {
    match params {
        RuleParams::None => None,
        RuleParams::Document {
            severity,
            subtype,
            notarization_required,
            translation_required,
            accepts_alternatives,
            conditions_key,
        } => Some(citation_fact::Params::DocParams(DocumentRequiredParams {
            severity: severity.clone(),
            subtype: subtype.clone().unwrap_or_default(),
            notarization_required: *notarization_required,
            translation_required: *translation_required,
            accepts_alternatives: *accepts_alternatives,
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Fee {
            amount,
            currency,
            severity,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::FeeParams(FeeItemParams {
            amount: *amount,
            currency: currency.clone(),
            severity: severity.clone(),
            channel: channel.clone().unwrap_or_default(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Timeline {
            days,
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::TimelineParams(TimelineItemParams {
            days: *days,
            subtype: subtype.clone().unwrap_or_default(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::WhereToApply {
            location_key,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::WhereParams(WhereToApplyParams {
            location_key: location_key.clone(),
            channel: channel.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::EligibilityRule {
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::EligibilityParams(
            EligibilityRuleParams {
                subtype: subtype.clone(),
                severity: severity.clone(),
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::AppointmentRule {
            subtype,
            advance_days,
            conditions_key,
        } => Some(citation_fact::Params::AppointmentParams(
            AppointmentRuleParams {
                subtype: subtype.clone(),
                advance_days: *advance_days,
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::FormRequired {
            form_id,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::FormParams(FormRequiredParams {
            form_id: form_id.clone(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Step {
            step_index,
            subtype,
            conditions_key,
        } => Some(citation_fact::Params::StepParams(StepParams {
            step_index: *step_index,
            subtype: subtype.clone(),
            conditions_key: conditions_key.clone(),
        })),
    }
}

#[async_trait]
impl SeoBuildInputRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_site_build_input(
        &self,
        run_id: &str,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        sqlx_seo_adapter::load_seo_site_build_input(self.pool, run_id).await
    }
}

#[async_trait]
impl SeoBuildRegistrationRepository for SqlxSeoRuntimeRepository<'_> {
    async fn register_site_build_input(
        &self,
        request: &SeoSiteBuildRegistrationRequest,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        let non_empty_queries = request
            .queries
            .iter()
            .map(|query| query.trim().to_string())
            .filter(|query| !query.is_empty())
            .collect::<Vec<_>>();
        if non_empty_queries.is_empty() {
            return Err(DomainError::ValidationFailure {
                message: "SeoSiteBuildWorkflow requires at least one --query".to_string(),
            });
        }
        Uuid::parse_str(&request.run_id).map_err(|err| DomainError::ValidationFailure {
            message: format!("SeoSiteBuild workflow_id must be a UUID: {err}"),
        })?;

        let scope = identity::derive_scope(
            &request.market,
            &request.locale,
            &request.country_code,
            &request.visa_type,
            &request.applicant_profile,
        )?;
        sqlx_seo_adapter::ensure_seo_runtime_registries(self.pool).await?;
        let normalized_profile = sqlx_seo_adapter::validate_applicant_profile_reference(
            self.pool,
            &scope.applicant_profile,
        )
        .await?;
        let scope = identity::ValidatedSeoScope {
            applicant_profile: normalized_profile,
            ..scope
        };
        let truth_identity = identity::derive_truth_identity(
            &request.country_code,
            &request.visa_type,
            request.visa_subtype.as_deref(),
            &request.citizenship_code,
        )?;
        let resolved_context_key = if request.bootstrap_context {
            sqlx_seo_adapter::bootstrap_seo_scope(
                self.pool,
                request.context_key.as_deref(),
                &truth_identity.country_code,
                &truth_identity.visa_family,
                if truth_identity.visa_subtype.is_empty() {
                    None
                } else {
                    Some(truth_identity.visa_subtype.as_str())
                },
                &truth_identity.citizenship_code,
            )
            .await?
            .context_key
        } else {
            request
                .context_key
                .clone()
                .unwrap_or_else(|| truth_identity.context_key.clone())
        };
        let input = SeoSiteBuildInputPayload {
            run_id: request.run_id.clone(),
            context_key: resolved_context_key,
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    market: scope.market.clone(),
                    locale: scope.locale.clone(),
                    country_code: scope.country_code.clone(),
                    visa_type: scope.visa_type.clone(),
                    applicant_profile: scope.applicant_profile.clone(),
                    raw_scope_tuple: scope.raw_scope_tuple.clone(),
                    scope_signature: scope.scope_signature.clone(),
                },
            ),
            query_batch_key: request.query_batch_key.clone().unwrap_or_else(|| {
                primitives::seo::seo_artifact_key(
                    "query_batch",
                    &[
                        &request.run_id,
                        &scope.scope_signature,
                        &non_empty_queries.join("|"),
                    ],
                )
            }),
            queries: non_empty_queries,
            verified_support: Vec::new(),
            required_page_types: Vec::new(),
            run_mode: request
                .run_mode
                .clone()
                .unwrap_or_else(|| "publish_with_hitl".to_string()),
        };
        sqlx_seo_adapter::upsert_seo_site_build_input(self.pool, &input).await?;
        Ok(input)
    }
}

#[async_trait]
impl VerifiedSupportRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_verified_support_bundle(
        &self,
        request: &VerifiedSupportBundleRequest,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
        sqlx_seo_adapter::load_verified_support_bundle(
            self.pool,
            &request.run_id,
            &request.context_key,
            &request.scope_signature,
            &request.applicant_profile,
        )
        .await
    }
}

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

#[async_trait]
impl SourceContextRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_source_context_chunks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SourceContextChunkState>, DomainError> {
        raw_crawl_adapter::retrieve_source_context_chunks(self.pool, query, limit as u64).await
    }
}

#[async_trait]
impl ProjectionStatusRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_projection_barrier_status(
        &self,
        run_id: &str,
    ) -> Result<ProjectionBarrierStatus, DomainError> {
        let statuses =
            sqlx_seo_adapter::read_projection_sync_status_for_run(self.pool, run_id).await?;
        Ok(ProjectionBarrierStatus {
            blocked_events: statuses
                .iter()
                .map(|status| status.blocking_event_count())
                .sum(),
            max_open_lag_ms: statuses
                .iter()
                .map(|status| status.max_open_lag_ms)
                .max()
                .unwrap_or(0),
        })
    }
}

#[async_trait]
impl RebuildRepository for SqlxSeoRuntimeRepository<'_> {
    async fn narrow_rebuild_impacts(
        &self,
        changed_truth_keys: &[String],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
        let impacted =
            sqlx_seo_adapter::resolve_rebuild_impacts(self.pool, changed_truth_keys).await?;
        Ok(impacted
            .into_iter()
            .map(
                |(page_node_key, reason_package)| RebuildDependencyEvidence {
                    page_node_key,
                    reason_package,
                },
            )
            .collect())
    }

    async fn persist_rebuild_detect_output(
        &self,
        input: &RebuildDetectInputPayload,
        output: &RebuildDetectOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_rebuild_detect_output(self.pool, input, output).await
    }
}

#[async_trait]
impl SerpSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn fetch_google_organic_live_advanced(
        &self,
        locale: Option<&str>,
        query: &str,
    ) -> Result<Option<OrganicSerpResponse>, DomainError> {
        let Some(config) = dataforseo_serp_adapter::DataForSeoConfig::from_env_with_locale(locale)
        else {
            return Ok(None);
        };
        let client =
            dataforseo_serp_adapter::DataForSeoSerpClient::from_config(config).map_err(|err| {
                DomainError::InfraUnavailable {
                    message: format!("configure DataForSEO client: {err}"),
                }
            })?;
        let response = client
            .google_organic_live_advanced(query)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("DataForSEO organic live advanced failed: {err}"),
            })?;
        Ok(Some(OrganicSerpResponse {
            raw_payload_utf8: response.raw_payload_utf8,
            organic_results: response
                .organic_results
                .into_iter()
                .map(|result| OrganicSerpResult {
                    rank: result.rank,
                    title: result.title,
                    url: result.url,
                    url_norm: result.url_norm,
                    domain_norm: result.domain_norm,
                    source_tier: result.source_tier,
                    snippet: result.snippet,
                })
                .collect(),
        }))
    }
}

#[async_trait]
impl SemanticLinkSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn search_link_targets(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
        let results = semantic_search_adapter::search_by_text(
            query,
            "seo_link_targets",
            u64::try_from(limit).map_err(|_| DomainError::ValidationFailure {
                message: format!("semantic link search limit too large: {limit}"),
            })?,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("semantic search failed: {err}"),
        })?;
        Ok(results
            .into_iter()
            .map(|candidate| SemanticLinkCandidate {
                entity_key: candidate.entity_key,
                score: candidate.score,
            })
            .collect())
    }
}

#[async_trait]
impl PlanningRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_serp_ingest_output(
        &self,
        input: &SerpIngestInputPayload,
        output: &SerpIngestOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_ingest_output(self.pool, input, output).await
    }

    async fn persist_live_serp_query_results(
        &self,
        run_id: &str,
        query_batch_key: &str,
        ordinal: usize,
        query: &str,
        response: &OrganicSerpResponse,
    ) -> Result<(), DomainError> {
        let job_id = primitives::hash::content_hash_v1(&format!(
            "{run_id}|{query_batch_key}|{ordinal}|{query}"
        ));
        sqlx_serp_adapter::save_raw_snapshot(
            self.pool,
            &sqlx_serp_adapter::RawSnapshotRecord {
                run_id: run_id.to_string(),
                job_id: job_id.clone(),
                url_norm: query.to_string(),
                query: query.to_string(),
                raw_payload_utf8: response.raw_payload_utf8.clone(),
            },
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO raw snapshot: {err}"),
        })?;
        let results = response
            .organic_results
            .iter()
            .map(|result| dataforseo_serp_adapter::DataForSeoOrganicResult {
                rank: result.rank,
                title: result.title.clone(),
                url: result.url.clone(),
                url_norm: result.url_norm.clone(),
                domain_norm: result.domain_norm.clone(),
                source_tier: result.source_tier.clone(),
                snippet: result.snippet.clone(),
                raw_json: serde_json::Value::Null,
            })
            .collect::<Vec<_>>();
        sqlx_serp_adapter::save_dataforseo_organic_results_and_enqueue(
            self.pool,
            run_id,
            &job_id,
            query_batch_key,
            &results,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO organic results: {err}"),
        })?;
        Ok(())
    }

    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_normalize_output(self.pool, input, output).await
    }

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_opportunity_build_output(self.pool, input, output).await
    }

    async fn persist_ia_build_output(
        &self,
        input: &IaBuildInputPayload,
        output: &IaBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(self.pool, input, output).await
    }

    async fn persist_link_recommend_output(
        &self,
        input: &LinkRecommendInputPayload,
        output: &LinkRecommendOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_link_recommend_output(self.pool, input, output).await
    }

    async fn persist_global_site_reconcile_output(
        &self,
        input: &GlobalSiteReconcileInputPayload,
        output: &GlobalSiteReconcileOutputPayload,
    ) -> Result<GlobalNavigationPersistReport, DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(
            self.pool,
            &IaBuildInputPayload {
                run_id: input.run_id.clone(),
                scope: input.scope.clone(),
                keyword_clusters: Vec::new(),
            },
            &IaBuildOutputPayload {
                page_nodes: output.page_nodes.clone(),
                page_blueprints: Vec::new(),
                cannibalization_conflicts: Vec::new(),
            },
        )
        .await?;
        sqlx_seo_adapter::persist_link_recommend_output(
            self.pool,
            &LinkRecommendInputPayload {
                run_id: input.run_id.clone(),
                page_nodes: output.page_nodes.clone(),
                max_links_per_page: 0,
            },
            &LinkRecommendOutputPayload {
                link_recommendations: output.link_recommendations.clone(),
            },
        )
        .await?;
        let scope = input.scope.as_ref().cloned().unwrap_or_default();
        let navigation = sqlx_seo_adapter::persist_global_navigation_from_active_pages(
            self.pool,
            &scope.market,
            &scope.locale,
            &input.reconcile_reason,
        )
        .await?;
        Ok(GlobalNavigationPersistReport {
            navigation_tree_key: navigation.navigation_tree_key,
            scope_count: navigation.scope_count,
            page_item_count: navigation.page_item_count,
            silo_group_count: navigation.silo_group_count,
            rebuild_plan_count: navigation.rebuild_plan_count,
        })
    }
}

#[async_trait]
impl SectionTemplateRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
        sqlx_seo_adapter::load_section_templates(self.pool, page_type_key, dominant_intent).await
    }
}

#[async_trait]
impl DraftRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_draft_assemble_output(
        &self,
        run_id: &str,
        output: &DraftAssembleOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_assemble_output(self.pool, run_id, output).await
    }

    async fn persist_draft_normalize_output(
        &self,
        input: &DraftNormalizeInputPayload,
        output: &DraftNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_normalize_output(self.pool, input, output).await
    }

    async fn persist_content_contract_validate_output(
        &self,
        input: &ContentContractValidateInputPayload,
        output: &ContentContractValidateOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_content_contract_validate_output(self.pool, input, output).await
    }

    async fn persist_draft_qa_output(
        &self,
        input: &DraftQaInputPayload,
        output: &DraftQaOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_qa_output(self.pool, input, output).await
    }
}

#[async_trait]
impl EditorialGenerationPort for SqlxSeoRuntimeRepository<'_> {
    async fn generate_editorial_draft(
        &self,
        input: &EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
        editorial_llm_adapter::generate_editorial_draft(input).await
    }
}

#[async_trait]
impl CmsReviewPort for SqlxSeoRuntimeRepository<'_> {
    async fn persist_cms_publish_output(
        &self,
        input: &CmsPublishInputPayload,
        output: &CmsPublishOutputPayload,
    ) -> Result<CmsPublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_cms_publish_output(self.pool, input, output).await
    }

    async fn load_latest_approval_decision(
        &self,
        page_node_key: &str,
        revision_id: &str,
    ) -> Result<Option<CmsApprovalDecision>, DomainError> {
        sqlx_seo_cms_adapter::load_latest_approval_decision(self.pool, page_node_key, revision_id)
            .await
    }
}

#[async_trait]
impl CmsReviewDecisionPort for SqlxSeoRuntimeRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError> {
        sqlx_seo_cms_adapter::apply_human_review_decision(self.pool, request).await
    }
}

#[async_trait]
impl PublishArtifactRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_publish_materialize_output(
        &self,
        input: &PublishMaterializeInputPayload,
        output: &PublishMaterializeOutputPayload,
    ) -> Result<PublishMaterializeOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_publish_materialize_output(self.pool, input, output).await
    }

    async fn persist_finalize_publish_output(
        &self,
        input: &FinalizePublishInputPayload,
        output: &FinalizePublishOutputPayload,
    ) -> Result<FinalizePublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_finalize_publish_output(self.pool, input, output).await
    }
}

#[async_trait]
impl ContextBundleRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_context_bundle(&self, context_key: &str) -> Result<ContextBundle, DomainError> {
        let Some(ctx) =
            sqlx_context_bundle_adapter::load_context_bundle_base(self.pool, context_key).await?
        else {
            return Ok(ContextBundle {
                context_key: context_key.to_string(),
                country_code: String::new(),
                visa_family: String::new(),
                visa_subtype: String::new(),
                citizenship_code: String::new(),
                ontology_rules: vec![],
                citation_facts: vec![],
                related_links: vec![],
            });
        };

        let rules_rows =
            sqlx_context_bundle_adapter::load_context_bundle_rules(self.pool, context_key).await?;
        let mut facts = Vec::new();
        for r in rules_rows {
            let citation = if let Some(sk) = r.source_key.clone() {
                Some(SourceCitation {
                    source_key: sk,
                    source_label: r.source_label.unwrap_or_default(),
                    base_url: r.base_url.unwrap_or_default(),
                })
            } else {
                None
            };

            facts.push(CitationFact {
                rule_instance_id: r.rule_instance_id,
                rule_type_key: r.rule_type_key,
                status: r.status,
                effective_from: r.effective_from,
                effective_to: r.effective_to,
                citation,
                role_type: read_role_type(&r.role_type),
                params: map_params(&r.params),
            });
        }

        Ok(ContextBundle {
            context_key: context_key.to_string(),
            country_code: ctx.country_code,
            visa_family: ctx.visa_family,
            visa_subtype: ctx.visa_subtype,
            citizenship_code: ctx.citizenship_code,
            ontology_rules: vec![],
            citation_facts: facts,
            related_links: vec![],
        })
    }
}

#[async_trait]
impl HitlQueuePort for SqlxSeoRuntimeRepository<'_> {
    async fn enqueue_hitl_task(
        &self,
        task_type: &str,
        diagnostics: &HitlTaskContext,
        priority: i32,
    ) -> Result<i64, DomainError> {
        sqlx_hitl_adapter::enqueue_hitl_task(self.pool, task_type, diagnostics, priority).await
    }

    async fn resolve_hitl_task(
        &self,
        task_id: i64,
        resolution: &HitlDecision,
    ) -> Result<(), DomainError> {
        sqlx_hitl_adapter::resolve_hitl_task(self.pool, task_id, resolution).await
    }
}
