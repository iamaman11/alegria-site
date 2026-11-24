    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness() {
        let source = stub_servers::spawn_text_stub(
            "/official/spain-tourist-visa",
            r#"
            <html>
              <head>
                <title>Spain Tourist Visa</title>
                <meta name="description" content="Official visa requirements" />
              </head>
              <body>
                <main>
                  <h1>Spain tourist visa for Belarusians</h1>
                  <p>Passport is required for the visa application.</p>
                  <p>The visa fee is 80 EUR.</p>
                  <p>Applications are submitted at the visa centre.</p>
                  <p>Standard tourist applicants are eligible to apply.</p>
                  <p>Application step 1: submit the application.</p>
                  <p>Processing takes 15 days.</p>
                </main>
              </body>
            </html>
            "#,
            "text/html; charset=utf-8",
        )
        .await
        .unwrap();

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
                            "url": format!("{}/official/spain-tourist-visa", source.base_url),
                            "description": "Official visa requirements"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":{\"overview\":\"Passport is required for the visa application.\"}}"
                    }
                }]
            }),
        )
        .await
        .unwrap();
        let infra = InfraHarness::start().await.unwrap();
        let _runtime_env = infra.apply_runtime_env();
        let _provider_env = ctx.apply_provider_env();

        let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
        let run_id = Uuid::new_v4().to_string();
        let query_batch_key = format!("synthetic-batch-{run_id}");
        let site_input = register_site_build_input(
            &repo,
            &SeoSiteBuildRegistrationRequest {
                run_id: run_id.clone(),
                context_key: None,
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec!["spain tourist visa belarus".to_string()],
                query_batch_key: Some(query_batch_key),
                retained_source_run_id: None,
                run_mode: Some(
                    run_mode_for_scenario(SeoScenarioKind::Full, false, false, true).to_string(),
                ),
            },
        )
        .await
        .unwrap();

        seed_synthetic_truth(
            &infra.postgres.pool,
            &site_input.context_key,
            &source.base_url,
        )
        .await;

        let output_dir = std::env::temp_dir().join(format!("integration-harness-step3-{run_id}"));
        fs::create_dir_all(&output_dir).unwrap();

        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                output_dir: output_dir.display().to_string(),
                base_url: "https://example.com".to_string(),
                site_input,
            },
        )
        .await
        .unwrap();

        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "serp_ingest"),
            "missing serp_ingest phase: {:?}",
            result.phase_reports
        );
        assert!(result
            .phase_reports
            .iter()
            .any(|r| r.phase == "serp_ingest"));
        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "crawl_sources"),
            "missing crawl_sources phase: {:?}",
            result.phase_reports
        );
        assert!(result
            .phase_reports
            .iter()
            .any(|r| r.phase == "crawl_sources"));
        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "draft_assemble"),
            "missing draft_assemble phase: {:?}",
            result.phase_reports
        );
        assert!(result
            .phase_reports
            .iter()
            .any(|r| r.phase == "draft_assemble"));

        assert_eq!(result.status, "done", "{:?}", result.phase_reports);
        let ready_pages: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT page_node_key)::bigint FROM site.page_drafts WHERE qa_verdict = 'publish_ready'",
        )
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
        assert!(result.page_total > 0);
        assert_eq!(
            ready_pages, i64::from(result.page_total),
            "every planned page must have a ready draft: {:?}", result.phase_reports
        );
    }
