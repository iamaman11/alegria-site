#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use seo_ports::{
        GraphCapabilityPort, GraphReasoningPort,
        OrganicSerpResponse, OrganicSerpResult,
        PlanningRepository, SemanticLinkCandidate,
        SemanticLinkSearchPort, SerpSearchPort,
    };
    use std::sync::Mutex;

    fn valid_scope() -> SeoScopePayload {
        let scope = SeoScopePayload {
            market: "alegria-site".into(),
            locale: "ru-RU".into(),
            country_code: "ES".into(),
            visa_type: "tourist".into(),
            applicant_profile: "standard".into(),
            ..Default::default()
        };
        let validated = seo_domain::identity::derive_scope(
            &scope.market,
            &scope.locale,
            &scope.country_code,
            &scope.visa_type,
            &scope.applicant_profile,
        )
        .unwrap();
        SeoScopePayload {
            raw_scope_tuple: validated.raw_scope_tuple,
            scope_signature: validated.scope_signature,
            ..scope
        }
    }

    #[test]
    fn malformed_explicit_scope_fails_before_graph_or_provider_io() {
        let invalid = SeoScopePayload {
            market: "alegria-site".into(),
            locale: "ru-RU".into(),
            country_code: "ES".into(),
            visa_type: "tourist".into(),
            applicant_profile: "standard".into(),
            scope_signature: "forged".into(),
            ..Default::default()
        };
        assert!(validated_planning_graph_scope(Some(&invalid)).is_err());
        assert!(validated_planning_graph_scope(None).unwrap().is_empty());
        let valid_without_wire_signature = SeoScopePayload {
            scope_signature: String::new(),
            ..invalid
        };
        let canonical = seo_steps::seo_identity::derive_scope_from_payload(
            &valid_without_wire_signature,
        )
        .unwrap();
        assert_eq!(
            validated_planning_graph_scope(Some(&valid_without_wire_signature)).unwrap(),
            canonical.scope_signature
        );
    }

    #[derive(Default)]
    struct FakePlanningRepo;

    struct UnavailableGraph;

    #[async_trait]
    impl GraphCapabilityPort for UnavailableGraph {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Err(DomainError::ValidationFailure {
                message: "graph provider refused connection".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn mandatory_graph_check_maps_failures_consistently_across_phases() {
        for (family, phase) in [
            ("planning", "ia_build"),
            ("drafting", "draft_qa"),
            ("rebuild", "rebuild_detect"),
        ] {
            let err = UnavailableGraph
                .require_graph_contract(family, phase)
                .await
                .unwrap_err();
            assert_eq!(err.class_str(), "infra_unavailable");
            assert!(err.to_string().contains(&format!(
                "graph capability contract failed for {family} phase `{phase}`"
            )));
            assert!(err.to_string().contains("graph provider refused connection"));
        }
    }

    #[async_trait]
    impl GraphCapabilityPort for FakePlanningRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl PlanningRepository for FakePlanningRepo {
        async fn existing_page_owners_by_cluster(
            &self,
            _scope_signature: &str,
            _cluster_keys: &[String],
        ) -> Result<Vec<(String, String)>, DomainError> {
            Ok(Vec::new())
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
            _output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl GraphReasoningPort for FakePlanningRepo {
        async fn load_planning_graph_context(
            &self,
            _scope_signature: &str,
            _run_id: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }

    }

    struct FakeSerpSearchPort;

    #[async_trait]
    impl SerpSearchPort for FakeSerpSearchPort {
        async fn fetch_google_organic_standard(
            &self,
            _run_id: &str,
            _locale: Option<&str>,
            _query: &str,
        ) -> Result<Option<OrganicSerpResponse>, DomainError> {
            Ok(Some(OrganicSerpResponse {
                raw_payload_utf8: "{}".to_string(),
                organic_results: vec![OrganicSerpResult {
                    rank: 1,
                    title: "Example".to_string(),
                    url: "https://example.com/visa".to_string(),
                    url_norm: "example.com/visa".to_string(),
                    domain_norm: "example.com".to_string(),
                    source_tier: "official".to_string(),
                    snippet: "snippet".to_string(),
                }],
            }))
        }
    }

    struct FakeSemanticSearchPort;

    #[async_trait]
    impl SemanticLinkSearchPort for FakeSemanticSearchPort {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "target".to_string(),
                score: 0.92,
            }])
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "cluster:spain-tourist".to_string(),
                score: 0.89,
            }])
        }

        async fn cluster_demand_queries(
            &self,
            queries: &[String],
        ) -> Result<Vec<seo_ports::SemanticDemandCluster>, DomainError> {
            Ok(vec![seo_ports::SemanticDemandCluster {
                cluster_key: "step0:cluster-spain".to_string(),
                member_queries: queries.to_vec(),
                confidence: 0.9,
            }])
        }
    }

    struct RecordingPlanningRepo {
        graph_context: runtime_models::GraphPlanningContext,
        opportunity_input: Mutex<Option<OpportunityBuildInputPayload>>,
        opportunity_output: Mutex<Option<OpportunityBuildOutputPayload>>,
        reconcile_output: Mutex<Option<GlobalSiteReconcileOutputPayload>>,
    }

    impl RecordingPlanningRepo {
        fn new(graph_context: runtime_models::GraphPlanningContext) -> Self {
            Self {
                graph_context,
                opportunity_input: Mutex::new(None),
                opportunity_output: Mutex::new(None),
                reconcile_output: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl GraphCapabilityPort for RecordingPlanningRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl PlanningRepository for RecordingPlanningRepo {
        async fn existing_page_owners_by_cluster(
            &self,
            _scope_signature: &str,
            _cluster_keys: &[String],
        ) -> Result<Vec<(String, String)>, DomainError> {
            Ok(Vec::new())
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
            input: &OpportunityBuildInputPayload,
            output: &OpportunityBuildOutputPayload,
        ) -> Result<OpportunityBuildOutputPayload, DomainError> {
            *self.opportunity_input.lock().unwrap() = Some(input.clone());
            *self.opportunity_output.lock().unwrap() = Some(output.clone());
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
            *self.reconcile_output.lock().unwrap() = Some(output.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl GraphReasoningPort for RecordingPlanningRepo {
        async fn load_planning_graph_context(
            &self,
            _scope_signature: &str,
            _run_id: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(self.graph_context.clone())
        }

    }

    #[tokio::test]
    async fn planning_serp_ingest_runs_without_sql_adapter() {
        let repo = FakePlanningRepo;
        let search = FakeSerpSearchPort;
        let output = run_serp_ingest(
            &repo,
            &search,
            &SerpIngestInputPayload {
                run_id: "run-1".to_string(),
                query_batch_key: "batch-1".to_string(),
                queries: vec!["spain tourist visa".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(output.persisted_snapshot_count, 1);
    }

    #[tokio::test]
    async fn planning_link_recommend_can_enrich_links() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let repo = FakePlanningRepo;
        let search = FakeSemanticSearchPort;
        let output = run_link_recommend(
            &repo,
            &search,
            &LinkRecommendInputPayload {
                max_links_per_page: 3,
                page_nodes: vec![
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "source".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "hub".to_string(),
                        dominant_intent: "requirements".to_string(),
                        canonical_url_path: "/visa/spain".to_string(),
                        menu_group: "visa".to_string(),
                        canonical_url_family: "/visa".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "target".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "documents".to_string(),
                        canonical_url_path: "/visa/spain/documents".to_string(),
                        menu_group: "visa".to_string(),
                        canonical_url_family: "/visa".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(output.link_recommendations.len() >= 1);
        assert!(output
            .link_recommendations
            .iter()
            .any(|link| { link.source_page_key == "source" && link.target_page_key == "target" }));
    }

    #[tokio::test]
    async fn run_opportunity_build_loads_graph_context_and_persists_reason_payload() {
        let scope = valid_scope();
        let repo = RecordingPlanningRepo::new(runtime_models::GraphPlanningContext {
            scope_signature: scope.scope_signature.clone(),
            topic_signals: vec![runtime_models::GraphPlanningTopicSignal {
                topic_key: "spain-tourist-fee".to_string(),
                topic_type: "fee".to_string(),
                support_refs: vec!["editorial://topic/fee".to_string()],
                graph_confidence: 0.84,
            }],
            coverage_signals: vec![runtime_models::GraphPlanningCoverageSignal {
                page_node_key: String::new(),
                keyword_cluster_key: String::new(),
                covered_topic_keys: Vec::new(),
                missing_topic_keys: vec!["processing-time".to_string()],
                graph_confidence: 0.8,
            }],
            ..Default::default()
        });
        let output = run_opportunity_build(
            &repo,
            &FakeSemanticSearchPort,
            &OpportunityBuildInputPayload {
                run_id: "run-1".to_string(),
                scope: Some(scope.clone()),
                serp_patterns: vec![
                    contracts::generated::alegria::temporal::v1::SerpPatternState {
                        serp_pattern_key: "p1".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        query: "Spain tourist visa fee".to_string(),
                        dominant_intent: "fee".to_string(),
                        reliability_score: 0.6,
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::SerpPatternState {
                        serp_pattern_key: "p2".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        query: "Spain tourist visa costs".to_string(),
                        dominant_intent: "fee".to_string(),
                        reliability_score: 0.58,
                        ..Default::default()
                    },
                ],
                graph_context: None,
            },
        )
        .await
        .unwrap();

        let persisted_input = repo.opportunity_input.lock().unwrap().clone().unwrap();
        assert!(persisted_input.graph_context.is_some());
        let persisted_output = repo.opportunity_output.lock().unwrap().clone().unwrap();
        assert_eq!(persisted_output.keyword_clusters.len(), 1);
        assert!(persisted_output
            .keyword_clusters
            .iter()
            .all(|cluster| cluster.reason_code == "graph_topic_family_merge"));
        assert!(persisted_output.content_gaps.iter().any(|gap| {
            gap.reason_code == "graph_missing_topic_coverage"
                && gap
                    .topic_keys
                    .iter()
                    .any(|topic| topic == "processing-time")
        }));
        assert_eq!(
            output.keyword_clusters.len(),
            persisted_output.keyword_clusters.len()
        );
    }

    #[tokio::test]
    async fn run_global_site_reconcile_emits_graph_reasoned_gap_without_truth_side_effects() {
        let scope = valid_scope();
        let repo = RecordingPlanningRepo::new(runtime_models::GraphPlanningContext {
            scope_signature: scope.scope_signature.clone(),
            coverage_signals: vec![runtime_models::GraphPlanningCoverageSignal {
                page_node_key: "page-spain".to_string(),
                keyword_cluster_key: String::new(),
                covered_topic_keys: Vec::new(),
                missing_topic_keys: vec!["insurance".to_string()],
                graph_confidence: 0.72,
            }],
            ..Default::default()
        });
        let output = run_global_site_reconcile(
            &repo,
            &FakeSemanticSearchPort,
            &GlobalSiteReconcileInputPayload {
                page_blueprints: Vec::new(),
                verified_support: Vec::new(),
                run_id: "run-2".to_string(),
                scope: Some(scope.clone()),
                page_nodes: vec![contracts::generated::alegria::temporal::v1::PageNodeState {
                    page_node_key: "page-spain".to_string(),
                    scope_signature: scope.scope_signature.clone(),
                    page_type_key: "hub_page".to_string(),
                    dominant_intent: "requirements".to_string(),
                    canonical_url_path: "/visa/spain".to_string(),
                    canonical_url_family: "/visa".to_string(),
                    lifecycle_state: "active".to_string(),
                    ..Default::default()
                }],
                link_recommendations: Vec::new(),
                reconcile_reason: "test@1".to_string(),
                graph_context: None,
            },
        )
        .await
        .unwrap();

        let persisted_output = repo.reconcile_output.lock().unwrap().clone().unwrap();
        assert!(persisted_output.content_gaps.iter().any(|gap| {
            gap.reason_code == "global_reconcile_missing_topic"
                && gap.topic_keys.iter().any(|topic| topic == "insurance")
        }));
        assert_eq!(output.page_nodes[0].lifecycle_state, "active");
    }

    struct OverlapSemanticSearchPort;

    #[async_trait]
    impl SemanticLinkSearchPort for OverlapSemanticSearchPort {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "target".to_string(),
                score: 0.91,
            }])
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "cluster-b".to_string(),
                score: 0.9,
            }])
        }

        async fn cluster_demand_queries(
            &self,
            queries: &[String],
        ) -> Result<Vec<seo_ports::SemanticDemandCluster>, DomainError> {
            Ok(vec![seo_ports::SemanticDemandCluster {
                cluster_key: "step0:overlap".to_string(),
                member_queries: queries.to_vec(),
                confidence: 0.9,
            }])
        }
    }

    #[tokio::test]
    async fn run_ia_build_adds_semantic_overlap_conflict_when_cluster_owner_diverges() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let scope = valid_scope();
        let repo = FakePlanningRepo;
        let output = run_ia_build(
            &repo,
            &OverlapSemanticSearchPort,
            &IaBuildInputPayload {
                verified_support: Vec::new(),
                run_id: "run-ia".to_string(),
                scope: Some(scope.clone()),
                keyword_clusters: vec![
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-a".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        seed_keyword: "spain visa requirements".to_string(),
                        dominant_intent: "informational".to_string(),
                        status: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-b".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        seed_keyword: "spain visa cost".to_string(),
                        dominant_intent: "informational".to_string(),
                        status: "active".to_string(),
                        ..Default::default()
                    },
                ],
                graph_context: None,
            },
        )
        .await
        .unwrap();
        assert!(
            output
                .cannibalization_conflicts
                .iter()
                .any(|conflict| conflict.conflict_reason == "semantic_cluster_owner_overlap"),
            "conflicts={:?}",
            output
                .cannibalization_conflicts
                .iter()
                .map(|conflict| (&conflict.conflict_key, &conflict.conflict_reason))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn run_global_site_reconcile_adds_semantic_overlap_conflict_key() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let scope = valid_scope();
        let repo = FakePlanningRepo;
        let output = run_global_site_reconcile(
            &repo,
            &OverlapSemanticSearchPort,
            &GlobalSiteReconcileInputPayload {
                page_blueprints: Vec::new(),
                verified_support: Vec::new(),
                run_id: "run-global".to_string(),
                scope: Some(scope.clone()),
                page_nodes: vec![
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "source".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        page_type_key: "detail_page".to_string(),
                        dominant_intent: "informational".to_string(),
                        canonical_url_path: "/ru/visa/spain/source/".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "target".to_string(),
                        scope_signature: scope.scope_signature.clone(),
                        page_type_key: "detail_page".to_string(),
                        dominant_intent: "informational".to_string(),
                        canonical_url_path: "/ru/visa/spain/target/".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!output.cannibalization_conflict_keys.is_empty());
    }
}
