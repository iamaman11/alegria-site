    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn synthetic_full_scenario_persists_page_draft_on_full_harness() {
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
                "tasks": [{
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

        let draft_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_drafts")
            .fetch_one(&infra.postgres.pool)
            .await
            .unwrap();
        assert!(
            draft_count >= 1,
            "expected at least one persisted page draft, status={} page_total={} phases={:?}",
            result.status,
            result.page_total,
            result.phase_reports
        );
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn cli_vs_temporal_semantic_parity_on_full_harness() {
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
                "tasks": [{
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

        let (cli_result, cli_snapshot, cli_phase_multiset) = {
            let cli_postgres = PostgresHarness::start().await.unwrap();
            let _cli_provider_env = ctx.apply_provider_env();
            let cli_repo = SqlxSeoRuntimeRepository::new(&cli_postgres.pool);
            let cli_run_id = Uuid::new_v4().to_string();
            let cli_input = register_synthetic_site_input(
                &cli_repo,
                cli_run_id.clone(),
                format!("synthetic-cli-batch-{cli_run_id}"),
            )
            .await;
            seed_synthetic_truth(&cli_postgres.pool, &cli_input.context_key, &source.base_url)
                .await;
            let cli_output_dir =
                std::env::temp_dir().join(format!("integration-harness-step4-cli-{cli_run_id}"));
            fs::create_dir_all(&cli_output_dir).unwrap();
            let cli_result = execute_site_build_scenario(
                &cli_repo,
                &SeoScenarioRequest {
                    scenario: SeoScenarioKind::Full,
                    mode: SeoExecutionMode::SemiAutoOperator,
                    policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                    output_dir: cli_output_dir.display().to_string(),
                    base_url: "https://example.com".to_string(),
                    site_input: cli_input.clone(),
                },
            )
            .await
            .unwrap();
            let cli_scope_signature = cli_input.scope.as_ref().unwrap().scope_signature.clone();
            let cli_snapshot =
                capture_semantic_snapshot(&cli_postgres.pool, &cli_scope_signature).await;
            let cli_phase_multiset = normalized_cli_phase_multiset(&cli_result);
            (cli_result, cli_snapshot, cli_phase_multiset)
        };

        let temporal_postgres = PostgresHarness::start().await.unwrap();
        let temporal = TemporalHarness::start().await.unwrap();
        let temporal_repo = SqlxSeoRuntimeRepository::new(&temporal_postgres.pool);
        let temporal_run_id = Uuid::new_v4().to_string();
        let temporal_input = register_synthetic_site_input(
            &temporal_repo,
            temporal_run_id.clone(),
            format!("synthetic-temporal-batch-{temporal_run_id}"),
        )
        .await;
        seed_synthetic_truth(
            &temporal_postgres.pool,
            &temporal_input.context_key,
            &source.base_url,
        )
        .await;

        let mut worker_env = BTreeMap::new();
        worker_env.insert(
            "DATABASE_URL".to_string(),
            temporal_postgres.database_url.clone(),
        );
        worker_env.insert("TEMPORAL_URL".to_string(), temporal.temporal_url.clone());
        worker_env.insert("TEMPORAL_NAMESPACE".to_string(), temporal.namespace.clone());
        worker_env.extend(ctx.provider_env());
        let mut worker = spawn_temporal_worker(&worker_env);
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            worker.child.try_wait().unwrap().is_none(),
            "temporal_worker exited before workflow start"
        );

        let client = connect_client(
            &temporal.temporal_url,
            format!("integration-parity-client-{}", Uuid::new_v4()),
            &temporal.namespace,
        )
        .await
        .unwrap();
        let handle = client
            .start_workflow(
                UntypedWorkflow::new("SeoSiteBuildWorkflow"),
                RawValue::default(),
                WorkflowStartOptions::new("alegria-pipeline", temporal_run_id.clone()).build(),
            )
            .await
            .unwrap();
        timeout(
            Duration::from_secs(120),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("temporal parity workflow timed out")
        .expect("temporal parity workflow failed");

        let temporal_scope_signature = temporal_input
            .scope
            .as_ref()
            .unwrap()
            .scope_signature
            .clone();
        let temporal_snapshot =
            capture_semantic_snapshot(&temporal_postgres.pool, &temporal_scope_signature).await;
        let temporal_step_multiset =
            temporal_step_multiset(&temporal_postgres.pool, &temporal_run_id).await;

        assert_eq!(cli_result.status, "done");
        assert_eq!(
            cli_result.page_total as usize,
            cli_snapshot.page_nodes.len()
        );
        assert!(
            !cli_snapshot.page_drafts.is_empty(),
            "cli path produced no drafts"
        );
        assert_eq!(cli_snapshot, temporal_snapshot);
        assert_eq!(cli_phase_multiset, temporal_step_multiset);
    }
