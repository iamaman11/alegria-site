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
                "status_code": 20000,
                "tasks": [{
                    "status_code": 20000,
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

        // The two LLM operations must use distinct stubs with distinct JSON
        // contracts: sections for editorial, candidates for truth extraction.
        let truth_completion = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", ctx.truth.base_url))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        let truth_body = truth_completion["choices"][0]["message"]["content"]
            .as_str()
            .expect("truth stub text response");
        assert_eq!(
            serde_json::from_str::<Value>(truth_body).unwrap(),
            serde_json::json!({"candidates": []})
        );
        let provider_env = ctx.provider_env();
        assert_ne!(
            provider_env["SEO_TRUTH_LLM_LOCAL_ENDPOINT"],
            provider_env["SEO_LLM_LOCAL_ENDPOINT"]
        );
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
                "status_code": 20000,
                "tasks": [{
                    "status_code": 20000,
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


    // One real PostgreSQL inbox, no second workflow or mock approval path.
    // The ChatGPT output is typed draft text, never a truth/publication decision.
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn manual_llm_queue_persists_validates_replays_and_recovers() {
        use infrastructure::adapters::manual_llm_queue::{
            self, ManualLlmSubmission,
        };

        let pg = PostgresHarness::start().await.unwrap();
        let mut input = sample_editorial_input();
        input.run_id = Uuid::new_v4().to_string();
        input.request.as_mut().unwrap().run_id = input.run_id.clone();
        input.provider_policy = "chat_manual".to_string();

        let pending = manual_llm_queue::enqueue_editorial(&pg.pool, &input)
            .await
            .unwrap();
        assert_eq!(pending.status, "awaiting_manual");
        assert!(pending.candidate.is_none());
        let job = manual_llm_queue::next_editorial(&pg.pool)
            .await
            .unwrap()
            .expect("pending manual job");
        assert_eq!(job.operation, "editorial_draft");
        assert_eq!(job.payload.run_id, input.run_id);
        assert_eq!(job.input_sha256.len(), 64);

        let payload = serde_json::json!({
            "task_id": job.task_id,
            "schema_version": job.schema_version,
            "input_sha256": job.input_sha256,
            "response": {
                "sections": {
                    "overview": "A passport is required according to verified source material."
                }
            }
        });
        let mut bad_hash = payload.clone();
        bad_hash["input_sha256"] = serde_json::json!("wrong-hash");
        let reject: ManualLlmSubmission = serde_json::from_value(bad_hash).unwrap();
        assert!(manual_llm_queue::submit_editorial(&pg.pool, &reject, "gpt-6")
            .await
            .is_err());

        let mut bad_role = payload.clone();
        bad_role["response"]["sections"] = serde_json::json!({
            "human_approval": "approved"
        });
        let reject: ManualLlmSubmission = serde_json::from_value(bad_role).unwrap();
        assert!(manual_llm_queue::submit_editorial(&pg.pool, &reject, "gpt-6")
            .await
            .is_err());

        let mut bad_envelope = payload.clone();
        bad_envelope["response"]["publish_approved"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<ManualLlmSubmission>(bad_envelope).is_err(),
            "LLM must not be able to approve publication"
        );
        assert_eq!(
            manual_llm_queue::read_editorial_result(&pg.pool, &input)
                .await
                .unwrap()
                .status,
            "awaiting_manual"
        );

        let good: ManualLlmSubmission = serde_json::from_value(payload.clone()).unwrap();
        let run_id = manual_llm_queue::submit_editorial(&pg.pool, &good, "gpt-6")
            .await
            .unwrap();
        assert_eq!(run_id, input.run_id);

        // Simulate a CLI restart: read through a new physical DB connection.
        let reopened = sqlx::PgPool::connect(&pg.database_url).await.unwrap();
        let completed = manual_llm_queue::read_editorial_result(&reopened, &input)
            .await
            .unwrap();
        assert_eq!(completed.status, "done");
        assert_eq!(completed.provider_key, "chat_manual");
        assert_eq!(completed.model_key, "gpt-6");
        assert!(completed.candidate.unwrap().sections.iter().any(|section| {
            section.section_role == "overview"
                && section.body_markdown.contains("A passport is required")
        }));
        assert!(manual_llm_queue::next_editorial(&reopened)
            .await
            .unwrap()
            .is_none());

        // Retrying after a DB commit but before Temporal signaling is safe.
        assert_eq!(
            manual_llm_queue::submit_editorial(&reopened, &good, "gpt-6")
                .await
                .unwrap(),
            input.run_id
        );
        assert!(manual_llm_queue::submit_editorial(&reopened, &good, "other-model")
            .await
            .is_err());
        let mut altered = payload;
        altered["response"]["sections"]["overview"] = serde_json::json!("Changed answer");
        let altered: ManualLlmSubmission = serde_json::from_value(altered).unwrap();
        assert!(manual_llm_queue::submit_editorial(&reopened, &altered, "gpt-6")
            .await
            .is_err());
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn manual_truth_inbox_refuses_fabricated_evidence_and_replays_atomically() {
        use infrastructure::adapters::{
            manual_llm_queue::{self, ManualTruthSubmission},
            truth_extraction_llm_adapter::TruthExtractionInput,
        };
        let pg = PostgresHarness::start().await.unwrap();
        let run_id = Uuid::new_v4().to_string();
        let source = TruthExtractionInput {
            context_key: "test:visa:by".to_string(),
            raw_section_id: 42,
            source_url: "https://example.gov/requirements".to_string(),
            source_domain: "example.gov".to_string(),
            heading_path: "Documents".to_string(),
            raw_text: "Passport is required.".to_string(),
            source_snapshot_hash: "confirmed-source-snapshot".to_string(),
        };
        manual_llm_queue::enqueue_truth(&pg.pool, &run_id, &source)
            .await
            .unwrap();
        assert_eq!(
            manual_llm_queue::truth_run_status(&pg.pool, &run_id)
                .await
                .unwrap(),
            "awaiting_manual"
        );
        let job = manual_llm_queue::next_job(&pg.pool)
            .await
            .unwrap()
            .expect("pending truth job");
        assert_eq!(job.operation, "truth_candidates");
        assert_eq!(job.payload["raw_section_id"], 42);
        let candidate = serde_json::json!({
            "role": "DOCUMENT_REQUIRED",
            "concept_canonical_key": "passport",
            "raw_mention": "Passport",
            "params": {"subtype": "passport", "severity": "mandatory"},
            "scope": {},
            "severity": "mandatory",
            "applies_to_profiles": [],
            "alternatives": [],
            "derivation_type": "direct",
            "is_numeric": false,
            "is_range": false,
            "is_incomplete": false,
            "confidence": 0.95,
            "evidence_section_id": 42,
            "evidence_quote": "Passport is required.",
            "span_start": 0,
            "span_end": source.raw_text.len(),
            "uncertainty_flags": []
        });
        let good = serde_json::json!({
            "task_id": job.task_id,
            "schema_version": 1,
            "input_sha256": job.input_sha256,
            "response": {"candidates": [candidate]}
        });
        let mut invented = good.clone();
        invented["response"]["candidates"][0]["evidence_quote"] =
            serde_json::json!("Fabricated government rule");
        let wrong: ManualTruthSubmission = serde_json::from_value(invented).unwrap();
        assert!(manual_llm_queue::submit_truth(&pg.pool, &wrong, "gpt-6")
            .await
            .is_err());
        let mut wrong_id = good.clone();
        wrong_id["input_sha256"] = serde_json::json!("invalid");
        let wrong: ManualTruthSubmission = serde_json::from_value(wrong_id).unwrap();
        assert!(manual_llm_queue::submit_truth(&pg.pool, &wrong, "gpt-6")
            .await
            .is_err());
        let mut fake_approval = good.clone();
        fake_approval["response"]["verified"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<ManualTruthSubmission>(fake_approval).is_err()
        );
        assert_eq!(
            manual_llm_queue::truth_run_status(&pg.pool, &run_id)
                .await
                .unwrap(),
            "awaiting_manual"
        );
        let submitted: ManualTruthSubmission = serde_json::from_value(good.clone()).unwrap();
        assert_eq!(
            manual_llm_queue::submit_truth(&pg.pool, &submitted, "gpt-6")
                .await
                .unwrap(),
            run_id
        );
        let reopened = sqlx::PgPool::connect(&pg.database_url).await.unwrap();
        assert_eq!(
            manual_llm_queue::truth_run_status(&reopened, &run_id)
                .await
                .unwrap(),
            "done"
        );
        let results = manual_llm_queue::read_truth_results(&reopened, &run_id)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.candidates.len(), 1);
        assert_eq!(results[0].1.provider_key, "chat_manual");
        assert_eq!(results[0].0.raw_text, source.raw_text);
        assert_eq!(
            manual_llm_queue::submit_truth(&reopened, &submitted, "gpt-6")
                .await
                .unwrap(),
            run_id
        );
        assert!(manual_llm_queue::submit_truth(&reopened, &submitted, "other-model")
            .await
            .is_err());
        let mut changed = good;
        changed["response"]["candidates"][0]["confidence"] = serde_json::json!(0.84);
        let changed: ManualTruthSubmission = serde_json::from_value(changed).unwrap();
        assert!(manual_llm_queue::submit_truth(&reopened, &changed, "gpt-6")
            .await
            .is_err());
    }
