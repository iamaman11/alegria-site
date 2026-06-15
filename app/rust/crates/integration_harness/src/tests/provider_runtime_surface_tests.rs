    async fn temporal_step_multiset(pool: &sqlx::PgPool, run_id: &str) -> BTreeMap<String, usize> {
        let allowed = [
            "serp_ingest",
            "crawl_sources",
            "raw_knowledge_ingestion",
            "serp_normalize",
            "opportunity_build",
            "ia_build",
            "link_recommend",
            "global_site_reconcile",
            "draft_assemble",
            "editorial_draft_generate",
            "draft_normalize",
            "content_contract_validate",
            "draft_qa",
            "rebuild_detect",
        ];
        let rows = sqlx::query(
            r#"
            SELECT step_name
            FROM pipeline.step_executions
            WHERE run_id = $1
              AND status = 'done'
            ORDER BY step_name
            "#,
        )
        .bind(Uuid::parse_str(run_id).unwrap())
        .fetch_all(pool)
        .await
        .unwrap();
        let mut counts = BTreeMap::new();
        for row in rows {
            let step_name: String = row.get("step_name");
            if allowed.contains(&step_name.as_str()) {
                *counts.entry(step_name).or_insert(0) += 1;
            }
        }
        counts
    }

    #[tokio::test]
    async fn integration_ctx_exposes_stub_servers() {
        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": "https://example.gov/visa",
                            "description": "Visa rules"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":[]}"
                    }
                }]
            }),
        )
        .await
        .unwrap();

        let serp = reqwest::Client::new()
            .post(format!(
                "{}/v3/serp/google/organic/live/advanced",
                ctx.dataforseo.base_url
            ))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            serp["tasks"][0]["result"][0]["items"][0]["rank_absolute"],
            1
        );

        let completion = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", ctx.openai.base_url))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(completion["choices"][0]["message"]["role"], "assistant");
    }

    fn sample_editorial_input() -> EditorialDraftGenerateInputPayload {
        EditorialDraftGenerateInputPayload {
            run_id: "run-1".to_string(),
            request: Some(LlmDraftRequest {
                request_key: "req-1".to_string(),
                run_id: "run-1".to_string(),
                editorial_brief: Some(EditorialBrief {
                    editorial_brief_key: "brief-1".to_string(),
                    page_node_key: "page-1".to_string(),
                    page_brief_key: "page-brief-1".to_string(),
                    target_audience: "tourist".to_string(),
                    expertise_goal: "help applicant understand the process".to_string(),
                    section_templates: vec![SectionTemplateBinding {
                        template_key: "overview".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "informational".to_string(),
                        section_role: "overview".to_string(),
                        heading: "Overview".to_string(),
                        template_body: "overview".to_string(),
                        template_version: 1,
                        required: true,
                    }],
                    verified_support: vec![SeoVerifiedFactSupportState {
                        support_ref: "rule-1".to_string(),
                        role_type: "document_required".to_string(),
                        fragment_text: "Passport is required.".to_string(),
                        source_label: "Official source".to_string(),
                        source_tier: "official".to_string(),
                        freshness_class: "fresh".to_string(),
                        observed_at: "2026-05-08T00:00:00Z".to_string(),
                        valid_until: "2027-05-08T00:00:00Z".to_string(),
                    }],
                    required_links: vec![LinkRecommendationState::default()],
                    truth_snapshot_ref: "truth-1".to_string(),
                    source_context_chunks: Vec::new(),
                }),
                provider_policy: "multi_provider:first_available".to_string(),
                prompt_version: "editorial_prompt@1".to_string(),
                output_contract: "headless".to_string(),
            }),
            provider_policy: String::new(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn provider_env_drives_real_adapter_contracts_against_stubs() {
        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "type": "organic",
                            "rank_group": 1,
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": "https://example.gov/visa",
                            "description": "Visa rules"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":{\"overview\":\"Passport is required.\"}}"
                    }
                }]
            }),
        )
        .await
        .unwrap();
        let _env = ctx.apply_provider_env();

        let config = DataForSeoConfig::from_env_with_locale(Some("en-US"))
            .expect("dataforseo config from integration env");
        let serp = DataForSeoSerpClient::from_config(config)
            .unwrap()
            .google_organic_live_advanced("visa rules")
            .await
            .unwrap();
        assert_eq!(serp.organic_results.len(), 1);
        assert_eq!(serp.organic_results[0].title, "Official source");

        let editorial = editorial_llm_adapter::generate_editorial_draft(&sample_editorial_input())
            .await
            .expect("editorial draft via local-compatible stub");
        assert_eq!(editorial.provider_key, "local_compatible");
        let candidate = editorial.candidate.expect("editorial candidate");
        assert!(candidate
            .sections
            .iter()
            .any(|section| section.section_role == "overview"
                && section.body_markdown.contains("Passport is required.")));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn postgres_harness_bootstraps_schema() {
        let harness = PostgresHarness::start().await.unwrap();
        let schemas: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM information_schema.schemata WHERE schema_name IN ('pipeline', 'system', 'site')")
                .fetch_one(&harness.pool)
                .await
                .unwrap();
        assert_eq!(schemas, 3);
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn qdrant_harness_accepts_client_probe() {
        let harness = QdrantHarness::start().await.unwrap();
        let exists = harness
            .client
            .collection_exists("raw_chunks_4")
            .await
            .unwrap();
        assert!(!exists);
        assert!(harness.qdrant_url.starts_with("http://127.0.0.1:"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn neo4j_harness_exposes_bolt_connection() {
        let harness = Neo4jHarness::start().await.unwrap();
        let graph = neo4rs::Graph::new(&harness.uri, &harness.user, &harness.password).unwrap();
        graph.run(neo4rs::query("RETURN 1 AS ok")).await.unwrap();
        assert!(harness.uri.starts_with("127.0.0.1:"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn temporal_harness_exposes_grpc_endpoint() {
        let harness = TemporalHarness::start().await.unwrap();
        assert!(harness.temporal_url.starts_with("http://127.0.0.1:"));
        assert_eq!(harness.namespace, "default");
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn infra_harness_exposes_runtime_env_surface() {
        let harness = InfraHarness::start().await.unwrap();
        let env = harness.runtime_env();
        assert!(env.contains_key("DATABASE_URL"));
        assert!(env.contains_key("QDRANT_URL"));
        assert!(env.contains_key("NEO4J_URI"));
        assert!(env.contains_key("NEO4J_USER"));
        assert!(env.contains_key("NEO4J_PASSWORD"));
        assert!(env.contains_key("TEMPORAL_URL"));
        assert!(env.contains_key("TEMPORAL_NAMESPACE"));
    }
