#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        EditorialDraftGenerateOutputPayload, GlobalSiteReconcileOutputPayload,
        IaBuildOutputPayload, LinkRecommendOutputPayload, LlmDraftCandidate,
        OpportunityBuildOutputPayload, RawKnowledgeIngestionOutputPayload, SeoScopePayload,
        SeoVerifiedFactSupportState, SerpIngestOutputPayload, SerpNormalizeOutputPayload,
    };
    use seo_ports::{
        CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
        CrawlIngestRepository, GraphCapabilityPort, GraphReasoningPort, OrganicSerpResponse,
        PlanningRepository, ProjectionBarrierStatus, ProjectionStatusRepository,
        PublishArtifactRepository, RebuildDependencyEvidence, RebuildRepository,
        SemanticLinkCandidate, SemanticLinkSearchPort, SeoBuildInputRepository,
        SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest, SerpIngestRepository,
        SerpSearchPort,
        SourceContextSearchPort, VerifiedSupportBundleRequest, VerifiedSupportRepository,
    };

    #[derive(Default)]
    enum EvidenceFixture {
        #[default]
        Passport,
        EligibilityOnly,
    }

    #[derive(Default)]
    struct FakeRepo {
        evidence: EvidenceFixture,
        empty_truth_changes: bool,
        reconciled: std::sync::Mutex<Option<GlobalSiteReconcileOutputPayload>>,
        editorial_calls: std::sync::Mutex<Vec<String>>,
        support_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl GraphCapabilityPort for FakeRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl SeoBuildInputRepository for FakeRepo {
        async fn load_site_build_input(
            &self,
            _run_id: &str,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl SeoBuildRegistrationRepository for FakeRepo {
        async fn register_site_build_input(
            &self,
            _request: &SeoSiteBuildRegistrationRequest,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl VerifiedSupportRepository for FakeRepo {
        async fn load_verified_support_bundle(
            &self,
            _request: &VerifiedSupportBundleRequest,
        ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
            self.support_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (fragment, support_ref, role_type) = match self.evidence {
                EvidenceFixture::Passport => {
                    ("Passport required", "rule:passport", "document_required")
                }
                EvidenceFixture::EligibilityOnly => (
                    "Tourists are eligible",
                    "rule:eligibility",
                    "eligibility_rule",
                ),
            };
            Ok(vec![SeoVerifiedFactSupportState {
                fragment_text: fragment.to_string(),
                support_ref: support_ref.to_string(),
                role_type: role_type.to_string(),
                source_label: "Consulate".to_string(),
                source_tier: "official".to_string(),
                freshness_class: "fresh".to_string(),
                observed_at: String::new(),
                valid_until: String::new(),
                source_url: String::new(),
            }])
        }
    }

    #[async_trait]
    impl CrawlIngestRepository for FakeRepo {
        async fn crawl_sources(
            &self,
            _input: &CrawlSourcesInputPayload,
        ) -> Result<CrawlSourcesOutputPayload, DomainError> {
            Ok(CrawlSourcesOutputPayload {
                claimed_count: 1,
                crawled_count: 1,
                failed_count: 0,
                raw_page_count: 1,
                raw_section_count: 1,
                qdrant_event_count: 0,
                status: "done".to_string(),
                raw_page_ids: vec![7],
                failed_urls: Vec::new(),
            })
        }

        async fn ingest_raw_knowledge(
            &self,
            _input: &RawKnowledgeIngestionInputPayload,
        ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
            Ok(RawKnowledgeIngestionOutputPayload {
                raw_page_count: 1,
                raw_section_count: 1,
                extracted_rule_count: 1,
                verified_rule_count: 1,
                outbox_event_count: 1,
                changed_truth_keys: if self.empty_truth_changes {
                    Vec::new()
                } else {
                    vec!["verified.rule_instance:passport".to_string()]
                },
                status: "done".to_string(),
            })
        }
    }

    #[async_trait]
    impl ProjectionStatusRepository for FakeRepo {
        async fn load_projection_barrier_status(
            &self,
            _run_id: &str,
        ) -> Result<ProjectionBarrierStatus, DomainError> {
            Ok(ProjectionBarrierStatus::default())
        }
    }

    #[async_trait]
    impl SerpSearchPort for FakeRepo {
        async fn fetch_google_organic_standard(
            &self,
            _run_id: &str,
            _locale: Option<&str>,
            _query: &str,
        ) -> Result<Option<OrganicSerpResponse>, DomainError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl SemanticLinkSearchPort for FakeRepo {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(Vec::new())
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(Vec::new())
        }

        async fn cluster_demand_queries(
            &self,
            _queries: &[String],
        ) -> Result<Vec<seo_ports::SemanticDemandCluster>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl SerpIngestRepository for FakeRepo {
        async fn persist_serp_ingest_output(
            &self,
            _input: &SerpIngestInputPayload,
            _output: &SerpIngestOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_live_serp_query_results(
            &self,
            _run_id: &str,
            _query_batch_key: &str,
            _ordinal: usize,
            _query: &str,
            _response: &OrganicSerpResponse,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl PlanningRepository for FakeRepo {
        async fn existing_page_owners_by_cluster(
            &self,
            _scope_signature: &str,
            _cluster_keys: &[String],
        ) -> Result<Vec<(String, String)>, DomainError> {
            Ok(Vec::new())
        }

        async fn persist_serp_normalize_output(
            &self,
            _input: &SerpNormalizeInputPayload,
            _output: &SerpNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_opportunity_build_output(
            &self,
            _input: &OpportunityBuildInputPayload,
            output: &OpportunityBuildOutputPayload,
        ) -> Result<OpportunityBuildOutputPayload, DomainError> {
            Ok(output.clone())
        }
        async fn persist_ia_build_output(
            &self,
            _input: &IaBuildInputPayload,
            _output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_link_recommend_output(
            &self,
            _input: &LinkRecommendInputPayload,
            _output: &LinkRecommendOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_global_site_reconcile_output(
            &self,
            _input: &GlobalSiteReconcileInputPayload,
            output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<(), DomainError> {
            *self.reconciled.lock().unwrap() = Some(output.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl GraphReasoningPort for FakeRepo {
        async fn load_planning_graph_context(
            &self,
            _scope_signature: &str,
            _run_id: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }
    }

    #[async_trait]
    impl SectionTemplateRepository for FakeRepo {
        async fn load_section_templates(
            &self,
            page_type_key: &str,
            dominant_intent: &str,
        ) -> Result<Vec<seo_ports::SectionTemplate>, DomainError> {
            Ok(vec![seo_ports::SectionTemplate {
                template_key: "tmpl:overview".to_string(),
                page_type_key: page_type_key.to_string(),
                dominant_intent: dominant_intent.to_string(),
                section_role: "overview".to_string(),
                heading: "Overview".to_string(),
                template_body: "overview template".to_string(),
                template_version: 1,
                required: true,
            }])
        }
    }

    #[async_trait]
    impl SourceContextSearchPort for FakeRepo {
        fn provider_configured(&self) -> bool {
            false
        }
        fn retrieval_required(&self) -> bool {
            false
        }
        async fn search_source_context_collection(
            &self,
            _collection: seo_ports::knowledge::SourceContextCollection,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<seo_ports::knowledge::SourceContextChunk>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl DraftRepository for FakeRepo {
        async fn persist_draft_assemble_output(
            &self,
            _run_id: &str,
            _output: &DraftAssembleOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_draft_normalize_output(
            &self,
            _input: &DraftNormalizeInputPayload,
            _output: &DraftNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_content_contract_validate_output(
            &self,
            _input: &ContentContractValidateInputPayload,
            _output: &contracts::generated::alegria::temporal::v1::ContentContractValidateOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_draft_qa_output(
            &self,
            _input: &DraftQaInputPayload,
            _output: &DraftQaOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl EditorialGenerationPort for FakeRepo {
        async fn generate_editorial_draft(
            &self,
            input: &EditorialDraftGenerateInputPayload,
        ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
            let brief = input
                .request
                .as_ref()
                .unwrap()
                .editorial_brief
                .as_ref()
                .unwrap();
            self.editorial_calls
                .lock()
                .unwrap()
                .push(brief.page_node_key.clone());
            Ok(EditorialDraftGenerateOutputPayload {
                provider_key: "fake".to_string(),
                model_key: "fake-model".to_string(),
                status: "selected".to_string(),
                candidate: Some(LlmDraftCandidate {
                    candidate_key: "cand-1".to_string(),
                    request_key: input
                        .request
                        .as_ref()
                        .map(|x| x.request_key.clone())
                        .unwrap_or_default(),
                    provider_key: "fake".to_string(),
                    model_key: "fake-model".to_string(),
                    prompt_version: "prompt@1".to_string(),
                    body_markdown: "draft".to_string(),
                    sections: Vec::new(),
                    claim_ledger: Vec::new(),
                    content_blocks: Vec::new(),
                    faq_json: "[]".to_string(),
                    schema_markup_json: "{}".to_string(),
                    status: "selected".to_string(),
                    seo_title: String::new(),
                    seo_meta_description: String::new(),
                }),
            })
        }
    }

    #[async_trait]
    impl CmsReviewPort for FakeRepo {
        async fn persist_cms_publish_output(
            &self,
            _input: &CmsPublishInputPayload,
            output: &CmsPublishOutputPayload,
        ) -> Result<CmsPublishOutputPayload, DomainError> {
            Ok(output.clone())
        }
        async fn load_latest_approval_decision(
            &self,
            _page_node_key: &str,
            _revision_id: &str,
        ) -> Result<Option<seo_ports::CmsApproval>, DomainError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl PublishArtifactRepository for FakeRepo {
        fn resolve_publish_materialize_input(
            &self,
            input: &PublishMaterializeInputPayload,
        ) -> PublishMaterializeInputPayload {
            input.clone()
        }

        async fn persist_publish_materialize_output(
            &self,
            _input: &PublishMaterializeInputPayload,
            output: &PublishMaterializeOutputPayload,
        ) -> Result<PublishMaterializeOutputPayload, DomainError> {
            Ok(output.clone())
        }
        async fn persist_finalize_publish_output(
            &self,
            _input: &FinalizePublishInputPayload,
            output: &contracts::generated::alegria::temporal::v1::FinalizePublishOutputPayload,
        ) -> Result<
            contracts::generated::alegria::temporal::v1::FinalizePublishOutputPayload,
            DomainError,
        > {
            Ok(output.clone())
        }
    }

    #[async_trait]
    impl RebuildRepository for FakeRepo {
        async fn narrow_rebuild_impacts(
            &self,
            _changed_truth_keys: &[String],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }
        async fn persist_rebuild_detect_output(
            &self,
            _input: &RebuildDetectRequest,
            _output: &seo_domain::rebuild::RebuildDetectResult,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn semantic_neighbor_impacts(
            &self,
            _changed_truth_keys: &[String],
            _page_node_keys: &[String],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl CmsReviewDecisionPort for FakeRepo {
        async fn apply_human_review_decision(
            &self,
            _request: &CmsReviewDecisionRequest,
        ) -> Result<CmsReviewDecisionOutcome, DomainError> {
            unreachable!()
        }
    }

    fn sample_site_input() -> SeoSiteBuildInputPayload {
        let validated = seo_domain::identity::derive_scope(
            "alegria-site",
            "ru-RU",
            "ES",
            "tourist",
            "standard",
        )
        .unwrap();
        SeoSiteBuildInputPayload {
            run_id: "run-1".to_string(),
            context_key: "ES|tourist||BY".to_string(),
            scope: Some(SeoScopePayload {
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                applicant_profile: "standard".to_string(),
                raw_scope_tuple: validated.raw_scope_tuple,
                scope_signature: validated.scope_signature,
            }),
            query_batch_key: "batch-1".to_string(),
            queries: vec!["spain tourist visa".to_string()],
            verified_support: Vec::new(),
            required_page_types: Vec::new(),
            retained_source_run_id: String::new(),
            run_mode: "publish_with_hitl".to_string(),
        }
    }

    // These two test types intentionally implement *only* the ports named by
    // their functional scenario, even though the shared fixture they delegate
    // to implements the complete runtime surface. The calls below therefore
    // compile only if the focused public executors have genuinely narrow
    // bounds (not a hidden aggregate runtime repository constraint).
    #[derive(Default)]
    struct CrawlPortsOnly {
        fixture: FakeRepo,
    }

    #[derive(Default)]
    struct PlanningPortsOnly {
        fixture: FakeRepo,
    }

    macro_rules! impl_ingestion_ports_only {
        ($owner:ty) => {
            #[async_trait]
            impl VerifiedSupportRepository for $owner {
                async fn load_verified_support_bundle(
                    &self,
                    request: &VerifiedSupportBundleRequest,
                ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
                    self.fixture.load_verified_support_bundle(request).await
                }
            }

            #[async_trait]
            impl SerpIngestRepository for $owner {
                async fn persist_serp_ingest_output(
                    &self,
                    input: &SerpIngestInputPayload,
                    output: &SerpIngestOutputPayload,
                ) -> Result<(), DomainError> {
                    self.fixture.persist_serp_ingest_output(input, output).await
                }

                async fn persist_live_serp_query_results(
                    &self,
                    run_id: &str,
                    query_batch_key: &str,
                    ordinal: usize,
                    query: &str,
                    response: &OrganicSerpResponse,
                ) -> Result<(), DomainError> {
                    self.fixture
                        .persist_live_serp_query_results(
                            run_id,
                            query_batch_key,
                            ordinal,
                            query,
                            response,
                        )
                        .await
                }
            }

            #[async_trait]
            impl CrawlIngestRepository for $owner {
                async fn crawl_sources(
                    &self,
                    input: &CrawlSourcesInputPayload,
                ) -> Result<CrawlSourcesOutputPayload, DomainError> {
                    self.fixture.crawl_sources(input).await
                }

                async fn ingest_raw_knowledge(
                    &self,
                    input: &RawKnowledgeIngestionInputPayload,
                ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
                    self.fixture.ingest_raw_knowledge(input).await
                }
            }

            #[async_trait]
            impl SerpSearchPort for $owner {
                async fn fetch_google_organic_standard(
                    &self,
                    run_id: &str,
                    locale: Option<&str>,
                    query: &str,
                ) -> Result<Option<OrganicSerpResponse>, DomainError> {
                    self.fixture
                        .fetch_google_organic_standard(run_id, locale, query)
                        .await
                }
            }

            #[async_trait]
            impl ProjectionStatusRepository for $owner {
                async fn load_projection_barrier_status(
                    &self,
                    run_id: &str,
                ) -> Result<ProjectionBarrierStatus, DomainError> {
                    self.fixture.load_projection_barrier_status(run_id).await
                }
            }
        };
    }

    impl_ingestion_ports_only!(CrawlPortsOnly);
    impl_ingestion_ports_only!(PlanningPortsOnly);

    #[async_trait]
    impl PlanningRepository for PlanningPortsOnly {
        async fn existing_page_owners_by_cluster(
            &self,
            scope_signature: &str,
            cluster_keys: &[String],
        ) -> Result<Vec<(String, String)>, DomainError> {
            self.fixture
                .existing_page_owners_by_cluster(scope_signature, cluster_keys)
                .await
        }

        async fn persist_serp_normalize_output(
            &self,
            input: &SerpNormalizeInputPayload,
            output: &SerpNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            self.fixture.persist_serp_normalize_output(input, output).await
        }

        async fn persist_opportunity_build_output(
            &self,
            input: &OpportunityBuildInputPayload,
            output: &OpportunityBuildOutputPayload,
        ) -> Result<OpportunityBuildOutputPayload, DomainError> {
            self.fixture
                .persist_opportunity_build_output(input, output)
                .await
        }

        async fn persist_ia_build_output(
            &self,
            input: &IaBuildInputPayload,
            output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            self.fixture.persist_ia_build_output(input, output).await
        }

        async fn persist_link_recommend_output(
            &self,
            input: &LinkRecommendInputPayload,
            output: &LinkRecommendOutputPayload,
        ) -> Result<(), DomainError> {
            self.fixture.persist_link_recommend_output(input, output).await
        }

        async fn persist_global_site_reconcile_output(
            &self,
            input: &GlobalSiteReconcileInputPayload,
            output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<(), DomainError> {
            self.fixture
                .persist_global_site_reconcile_output(input, output)
                .await
        }
    }

    #[async_trait]
    impl GraphReasoningPort for PlanningPortsOnly {
        async fn load_planning_graph_context(
            &self,
            scope_signature: &str,
            run_id: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            self.fixture
                .load_planning_graph_context(scope_signature, run_id)
                .await
        }
    }

    #[async_trait]
    impl GraphCapabilityPort for PlanningPortsOnly {
        async fn ensure_graph_contract(&self, context_key: &str) -> Result<(), DomainError> {
            self.fixture.ensure_graph_contract(context_key).await
        }
    }

    #[async_trait]
    impl SemanticLinkSearchPort for PlanningPortsOnly {
        async fn search_link_targets(
            &self,
            query: &str,
            limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            self.fixture.search_link_targets(query, limit).await
        }

        async fn search_keyword_clusters(
            &self,
            query: &str,
            limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            self.fixture.search_keyword_clusters(query, limit).await
        }

        async fn cluster_demand_queries(
            &self,
            queries: &[String],
        ) -> Result<Vec<seo_ports::SemanticDemandCluster>, DomainError> {
            self.fixture.cluster_demand_queries(queries).await
        }
    }

    fn narrow_scenario_request(scenario: SeoScenarioKind) -> SeoScenarioRequest {
        SeoScenarioRequest {
            scenario,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        }
    }

    #[tokio::test]
    async fn crawl_only_executes_with_five_ports_and_alternate_order() {
        use SeoPhaseKey::*;
        let repo = CrawlPortsOnly::default();
        let request = narrow_scenario_request(SeoScenarioKind::CrawlIngestOnly);
        let result = execute_crawl_ingest_only(&repo, &request).await.unwrap();
        assert_eq!(result.published_pages, 0);
        assert_eq!(result.page_total, 0);
        assert_eq!(result.changed_truth_keys.len(), 1);

        let alternate = SeoPhaseRecipe::compose(
            vec![SerpIngest, LoadVerifiedSupportBundle, CrawlSources, RawKnowledgeIngestion],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let alternative_result = execute_crawl_ingest_only_with_recipe(&repo, &request, &alternate)
            .await
            .unwrap();
        assert_eq!(alternative_result.phase_reports[0].phase, "serp_ingest");
        assert_eq!(alternative_result.phase_reports[1].phase, "load_verified_support_bundle");
    }

    #[tokio::test]
    async fn crawl_only_rejects_foreign_steps_and_scenarios_without_io() {
        let repo = CrawlPortsOnly::default();
        let request = narrow_scenario_request(SeoScenarioKind::CrawlIngestOnly);
        let planning_recipe = SeoPhaseRecipe::for_plan(
            &build_execution_plan(&narrow_scenario_request(SeoScenarioKind::PlanningOnly)),
            true,
        )
        .unwrap();
        let result = execute_crawl_ingest_only_with_recipe(&repo, &request, &planning_recipe).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));

        let wrong_scenario = narrow_scenario_request(SeoScenarioKind::Full);
        let result = execute_crawl_ingest_only(&repo, &wrong_scenario).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));
        assert_eq!(
            repo.fixture
                .support_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn planning_only_compiles_without_editorial_cms_publish_or_rebuild_ports() {
        use SeoPhaseKey::*;
        let repo = PlanningPortsOnly::default();
        let request = narrow_scenario_request(SeoScenarioKind::PlanningOnly);
        let result = execute_planning_only(&repo, &request).await.unwrap();
        assert_eq!(result.published_pages, 0);
        assert!(result.phase_reports.iter().any(|report| report.phase == "global_site_reconcile"));

        let interleaved = SeoPhaseRecipe::compose_pre_page(
            vec![
                SerpIngest, SerpNormalize, OpportunityBuild, LoadVerifiedSupportBundle,
                CrawlSources, RawKnowledgeIngestion, RefreshVerifiedSupportBundle,
                IaBuild, LinkRecommend, GlobalSiteReconcile,
            ],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let result = execute_planning_only_with_recipe(&repo, &request, &interleaved)
            .await
            .unwrap();
        let phases = result
            .phase_reports
            .iter()
            .map(|report| report.phase.as_str())
            .collect::<Vec<_>>();
        assert!(phases.iter().position(|p| *p == "serp_normalize").unwrap()
            < phases.iter().position(|p| *p == "crawl_sources").unwrap());
        assert!(phases.contains(&"global_site_reconcile"));
    }

    #[tokio::test]
    async fn planning_only_rejects_foreign_recipe_and_request_without_io() {
        let repo = PlanningPortsOnly::default();
        let request = narrow_scenario_request(SeoScenarioKind::PlanningOnly);
        let full_recipe = SeoPhaseRecipe::for_plan(
            &build_execution_plan(&narrow_scenario_request(SeoScenarioKind::Full)),
            true,
        )
        .unwrap();
        assert!(matches!(
            execute_planning_only_with_recipe(&repo, &request, &full_recipe).await,
            Err(DomainError::ContractViolation { .. })
        ));
        assert!(matches!(
            execute_planning_only(
                &repo,
                &narrow_scenario_request(SeoScenarioKind::CrawlIngestOnly)
            )
            .await,
            Err(DomainError::ContractViolation { .. })
        ));
        assert_eq!(
            repo.fixture.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn scenario_planning_only_uses_shared_plan() {
        let repo = FakeRepo::default();
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::PlanningOnly,
                mode: SeoExecutionMode::ManualTest,
                policy: SeoRunPolicy::for_manual_test(),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.scenario, "site_build_planning_only");
        assert!(result
            .phase_reports
            .iter()
            .any(|r| r.phase == "global_site_reconcile"));
    }

    #[tokio::test]
    async fn scenario_executes_an_alternate_valid_step_order() {
        use SeoPhaseKey::*;
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let standard = SeoPhaseRecipe::for_plan(&build_execution_plan(&request), true).unwrap();
        let alternate = SeoPhaseRecipe::compose(
            vec![
                SerpIngest,
                LoadVerifiedSupportBundle,
                CrawlSources,
                RawKnowledgeIngestion,
            ],
            standard.steps(SeoExecutionSegment::Planning).to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &alternate)
            .await
            .unwrap();
        assert_eq!(result.phase_reports[0].phase, "serp_ingest");
        assert_eq!(
            result.phase_reports[1].phase,
            "load_verified_support_bundle"
        );
        assert!(result
            .phase_reports
            .iter()
            .any(|phase| phase.phase == "global_site_reconcile"));
    }

    #[tokio::test]
    async fn scenario_executes_planning_between_source_acquisition_steps() {
        use SeoPhaseKey::*;
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let interleaved = SeoPhaseRecipe::compose_pre_page(
            vec![
                SerpIngest,
                SerpNormalize,
                OpportunityBuild,
                LoadVerifiedSupportBundle,
                CrawlSources,
                RawKnowledgeIngestion,
                RefreshVerifiedSupportBundle,
                IaBuild,
                LinkRecommend,
                GlobalSiteReconcile,
            ],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &interleaved)
            .await
            .unwrap();
        let phases = result
            .phase_reports
            .iter()
            .map(|report| report.phase.as_str())
            .collect::<Vec<_>>();
        let normalize = phases
            .iter()
            .position(|phase| *phase == "serp_normalize")
            .unwrap();
        let crawl = phases
            .iter()
            .position(|phase| *phase == "crawl_sources")
            .unwrap();
        assert!(normalize < crawl, "interleaved order must be actually executed");
        assert!(phases.contains(&"global_site_reconcile"));
    }

    #[tokio::test]
    async fn scenario_executes_a_small_draft_only_recipe_without_qa_side_effects() {
        use SeoPhaseKey::*;
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let standard = SeoPhaseRecipe::for_plan(&build_execution_plan(&request), true).unwrap();
        let without_qa = SeoPhaseRecipe::compose_pre_page(
            standard.pre_page_steps().to_vec(),
            vec![DraftAssemble, EditorialDraftGenerate, DraftNormalize],
            Vec::new(),
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &without_qa)
            .await
            .unwrap();
        assert!(result
            .phase_reports
            .iter()
            .any(|report| report.phase == "draft_normalize"));
        assert!(!result
            .phase_reports
            .iter()
            .any(|report| report.phase == "draft_qa"));
        assert_eq!(result.published_pages, 0);
    }

    #[tokio::test]
    async fn late_support_producer_runs_even_without_truth_changes() {
        use SeoPhaseKey::*;
        let repo = FakeRepo {
            empty_truth_changes: true,
            ..Default::default()
        };
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let standard = SeoPhaseRecipe::for_plan(&build_execution_plan(&request), true).unwrap();
        let late_support = SeoPhaseRecipe::compose(
            vec![SerpIngest, CrawlSources, RawKnowledgeIngestion],
            standard.steps(SeoExecutionSegment::Planning).to_vec(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &late_support)
            .await
            .unwrap();
        assert_eq!(
            repo.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(result
            .phase_reports
            .iter()
            .any(|phase| phase.phase == "load_verified_support_bundle.refresh"));
    }

    #[tokio::test]
    async fn invalid_recipe_policy_fails_before_external_effects() {
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let full_publish = SeoPhaseRecipe::for_plan(
            &crate::execution::SeoExecutionPlan {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::TemporalDurable,
                policy: SeoRunPolicy::for_temporal_durable(),
            },
            true,
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &full_publish).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));
        assert_eq!(
            repo.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn publish_enabled_recipe_cannot_silently_omit_publishing_before_io() {
        use SeoPhaseKey::*;
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::TemporalDurable,
            policy: SeoRunPolicy::for_temporal_durable(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let standard = SeoPhaseRecipe::for_plan(&build_execution_plan(&request), true).unwrap();
        let missing_publish = SeoPhaseRecipe::compose_pre_page(
            standard.pre_page_steps().to_vec(),
            vec![DraftAssemble, EditorialDraftGenerate, DraftNormalize],
            Vec::new(),
        )
        .unwrap();
        let result =
            execute_site_build_scenario_with_recipe(&repo, &request, &missing_publish).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));
        assert_eq!(
            repo.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn crawl_only_recipe_rejects_unreachable_planning_before_io() {
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::CrawlIngestOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let recipe = SeoPhaseRecipe::for_plan(
            &crate::execution::SeoExecutionPlan {
                scenario: SeoScenarioKind::PlanningOnly,
                mode: SeoExecutionMode::ManualTest,
                policy: SeoRunPolicy::for_manual_test(),
            },
            false,
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &recipe).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));
        assert_eq!(
            repo.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn planning_only_recipe_rejects_unreachable_page_steps_before_io() {
        let repo = FakeRepo::default();
        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
            output_dir: "/tmp".to_string(),
            base_url: "https://example.com".to_string(),
            site_input: sample_site_input(),
        };
        let recipe = SeoPhaseRecipe::for_plan(
            &crate::execution::SeoExecutionPlan {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::ManualTest,
                policy: SeoRunPolicy::for_manual_test(),
            },
            false,
        )
        .unwrap();
        let result = execute_site_build_scenario_with_recipe(&repo, &request, &recipe).await;
        assert!(matches!(result, Err(DomainError::ContractViolation { .. })));
        assert_eq!(
            repo.support_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn scenario_publish_blocks_without_human_approval() {
        let repo = FakeRepo::default();
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::PublishOnly,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(true, false, true),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.status, "blocked_publish_gate");
    }

    #[tokio::test]
    async fn scenario_incomplete_evidence_blocks_before_editorial_generation() {
        // This fixture has applicable eligibility evidence, but none of the
        // documents/fees/timing evidence that creates a focused leaf page.
        let repo = FakeRepo {
            evidence: EvidenceFixture::EligibilityOnly,
            ..Default::default()
        };
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.status, "blocked_publish_gate");
        assert_eq!(result.published_pages, 0);
        assert!(result.phase_reports.iter().any(|report| {
            report.phase == "page_admissibility"
                && report
                    .detail
                    .starts_with("page_missing_verified_support:roles=")
                && report.detail.contains("fees")
        }));
        assert!(!result.phase_reports.iter().any(|report| {
            matches!(
                report.phase.as_str(),
                "draft_assemble" | "editorial_draft_generate"
            )
        }));
        assert!(repo.editorial_calls.lock().unwrap().is_empty());
        let reconciled = repo.reconciled.lock().unwrap();
        let pages = &reconciled.as_ref().unwrap().page_nodes;
        assert!(!pages.is_empty());
        assert!(pages.iter().all(|page| page.lifecycle_state == "blocked"));
        for page in pages {
            assert!(result.phase_reports.iter().any(|report| {
                report.phase == "page_admissibility"
                    && report.page_node_key == page.page_node_key
                    && report.status == "blocked_publish_gate"
            }));
        }
    }

    #[tokio::test]
    async fn scenario_mixed_evidence_only_generates_for_admitted_page() {
        let repo = FakeRepo::default();
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.status, "blocked_publish_gate");
        assert_eq!(result.published_pages, 0);
        let reconciled = repo.reconciled.lock().unwrap();
        let pages = &reconciled.as_ref().unwrap().page_nodes;
        let eligible: Vec<_> = pages
            .iter()
            .filter(|page| page.lifecycle_state == "planned")
            .collect();
        assert_eq!(
            eligible.len(),
            1,
            "only the evidenced document leaf should be admitted"
        );
        assert_eq!(eligible[0].page_type_key, "requirement_page");
        assert_eq!(
            eligible[0].canonical_url_path,
            "/ru/visa/spain/tourist/documents/"
        );
        let blocked: Vec<_> = pages
            .iter()
            .filter(|page| page.lifecycle_state == "blocked")
            .collect();
        assert!(!blocked.is_empty());
        let calls = repo.editorial_calls.lock().unwrap();
        assert_eq!(calls.as_slice(), &[eligible[0].page_node_key.clone()]);
        for page in blocked {
            assert!(result.phase_reports.iter().any(|report| {
                report.phase == "page_admissibility"
                    && report.page_node_key == page.page_node_key
                    && report.status == "blocked_publish_gate"
            }));
            assert!(!result.phase_reports.iter().any(|report| {
                report.page_node_key == page.page_node_key
                    && matches!(
                        report.phase.as_str(),
                        "draft_assemble" | "editorial_draft_generate"
                    )
            }));
        }
    }

    #[test]
    fn scenario_report_surface_serializes_phase_reports() {
        let result = SeoScenarioResult {
            scenario: "site_build_full".to_string(),
            mode: "temporal_durable".to_string(),
            status: "ok".to_string(),
            page_total: 1,
            published_pages: 0,
            changed_truth_keys: vec!["truth_key".to_string()],
            phase_reports: vec![SeoPhaseReport {
                phase: "draft_assemble".to_string(),
                status: "done".to_string(),
                page_node_key: "page_1".to_string(),
                detail: "assembled".to_string(),
            }],
        };

        assert_eq!(result.phase_reports[0].phase, "draft_assemble");
        assert_eq!(result.phase_reports[0].status, "done");
        assert_eq!(result.changed_truth_keys[0], "truth_key");
    }
}
