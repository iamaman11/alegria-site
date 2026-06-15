#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        CmsApprovalDecision, EditorialDraftGenerateOutputPayload, GlobalSiteReconcileOutputPayload,
        IaBuildOutputPayload, LinkRecommendOutputPayload, LlmDraftCandidate,
        OpportunityBuildOutputPayload, RawKnowledgeIngestionOutputPayload, SectionTemplateBinding,
        SeoScopePayload, SeoVerifiedFactSupportState, SerpIngestOutputPayload,
        SerpNormalizeOutputPayload,
    };
    use seo_ports::{
        CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
        CrawlIngestRepository, GraphCoverageEvaluation, GraphNeighborhoodHit, GraphReasoningPort,
        OrganicSerpResponse, PlanningRepository, ProjectionBarrierStatus,
        ProjectionStatusRepository, PublishArtifactRepository, RebuildDependencyEvidence,
        RebuildRepository, SemanticLinkCandidate, SemanticLinkSearchPort, SeoBuildInputRepository,
        SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest, SerpSearchPort,
        SourceContextRepository, VerifiedSupportBundleRequest, VerifiedSupportRepository,
        GraphCapabilityPort,
    };

    #[derive(Default)]
    struct FakeRepo;

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
            Ok(vec![SeoVerifiedFactSupportState {
                fragment_text: "Passport required".to_string(),
                support_ref: "rule:passport".to_string(),
                role_type: "document_required".to_string(),
                source_label: "Consulate".to_string(),
                source_tier: "official".to_string(),
                freshness_class: "watch".to_string(),
                observed_at: String::new(),
                valid_until: String::new(),
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
                changed_truth_keys: vec!["verified.rule_instance:passport".to_string()],
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
        async fn fetch_google_organic_live_advanced(
            &self,
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
    impl PlanningRepository for FakeRepo {
        async fn load_graph_planning_context(
            &self,
            _run_id: &str,
            _scope_signature: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }

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
            _output: &OpportunityBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
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
        ) -> Result<seo_ports::GlobalNavigationPersistReport, DomainError> {
            Ok(seo_ports::GlobalNavigationPersistReport {
                navigation_tree_key: "synthetic-navigation".to_string(),
                scope_count: 1,
                page_item_count: output.page_nodes.len() as u64,
                silo_group_count: 0,
                rebuild_plan_count: 0,
            })
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

        async fn find_conflict_neighborhood(
            &self,
            _scope_signature: &str,
            _page_node_key: &str,
        ) -> Result<Vec<GraphNeighborhoodHit>, DomainError> {
            Ok(Vec::new())
        }

        async fn find_rebuild_impact_neighborhood(
            &self,
            _changed_truth_keys: &[String],
            _page_nodes: &[contracts::generated::alegria::temporal::v1::PageNodeState],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }

        async fn evaluate_draft_coverage_neighborhood(
            &self,
            page_node_key: &str,
            _draft_markdown: &str,
        ) -> Result<GraphCoverageEvaluation, DomainError> {
            Ok(GraphCoverageEvaluation {
                page_node_key: page_node_key.to_string(),
                coverage_score: 0.0,
                missing_topics: Vec::new(),
                reason_codes: vec!["graph_draft_coverage".to_string()],
                support_refs: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl SectionTemplateRepository for FakeRepo {
        async fn load_section_templates(
            &self,
            page_type_key: &str,
            dominant_intent: &str,
        ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
            Ok(vec![SectionTemplateBinding {
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
    impl SourceContextRepository for FakeRepo {
        async fn load_source_context_chunks(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<
            Vec<contracts::generated::alegria::temporal::v1::SourceContextChunkState>,
            DomainError,
        > {
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
        ) -> Result<Option<CmsApprovalDecision>, DomainError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl PublishArtifactRepository for FakeRepo {
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
            _input: &RebuildDetectInputPayload,
            _output: &contracts::generated::alegria::temporal::v1::RebuildDetectOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn semantic_neighbor_impacts(
            &self,
            _changed_truth_keys: &[String],
            _page_nodes: &[contracts::generated::alegria::temporal::v1::PageNodeState],
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
        SeoSiteBuildInputPayload {
            run_id: "run-1".to_string(),
            context_key: "ES|tourist||BY".to_string(),
            scope: Some(SeoScopePayload {
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                applicant_profile: "standard".to_string(),
                raw_scope_tuple: String::new(),
                scope_signature: "sig-1".to_string(),
            }),
            query_batch_key: "batch-1".to_string(),
            queries: vec!["spain tourist visa".to_string()],
            verified_support: Vec::new(),
            required_page_types: Vec::new(),
            run_mode: "publish_with_hitl".to_string(),
        }
    }

    #[tokio::test]
    async fn scenario_planning_only_uses_shared_plan() {
        let repo = FakeRepo;
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
    async fn scenario_publish_blocks_without_human_approval() {
        let repo = FakeRepo;
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
