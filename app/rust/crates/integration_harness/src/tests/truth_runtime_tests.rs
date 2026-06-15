    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_promotes_corroborated_candidates_into_admissible_verified_truth() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-corroborated:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text = "Consular fee is 35 EUR";
        let (page_a, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-a.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;
        let (page_b, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-b.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![
                local_truth_response(local_truth_candidate_payload(
                    raw_text,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
                local_truth_response(local_truth_candidate_payload(
                    raw_text,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
            ],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_a, page_b],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 2);
        assert_eq!(report.verified_rule_count, 1);
        assert_eq!(report.needs_hitl_candidate_count, 0);

        let verified_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'verified'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(verified_candidates, 2);

        let verified_row = sqlx::query(
            "SELECT status, publish_admissibility, verification_method, adjudication_reason
             FROM verified.rule_instances
             WHERE context_key = $1 AND concept_key = 'consular_fee'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(verified_row.get::<String, _>("status"), "verified");
        assert_eq!(
            verified_row.get::<String, _>("publish_admissibility"),
            "admissible"
        );
        assert_eq!(
            verified_row.get::<String, _>("verification_method"),
            "cross_source_consensus@1"
        );
        assert!(verified_row
            .get::<String, _>("adjudication_reason")
            .contains("corroborated_structured_candidates"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_marks_incomplete_candidate_as_needs_hitl() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-needs-hitl:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text = "Consular fee is payable on submission.";
        let (page_id, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-c.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![local_truth_response(local_truth_candidate_payload(
                raw_text,
                "FEE_ITEM",
                "consular_fee",
                serde_json::json!({"currency": "EUR"}),
            ))],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_id],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 1);
        assert_eq!(report.verified_rule_count, 0);
        assert_eq!(report.needs_hitl_candidate_count, 1);

        let needs_hitl_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'needs_hitl'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(needs_hitl_candidates, 1);

        let admissible_verified_rows: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM verified.rule_instances
             WHERE context_key = $1 AND publish_admissibility = 'admissible'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(admissible_verified_rows, 0);
    }
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_keeps_conflicting_structured_candidates_out_of_verified_storage() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-conflict:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text_a = "Consular fee is 35 EUR";
        let raw_text_b = "Consular fee is 80 EUR";
        let (page_a, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-d.example.gov/fees",
            "government",
            "Fees",
            raw_text_a,
        )
        .await;
        let (page_b, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-e.example.gov/fees",
            "government",
            "Fees",
            raw_text_b,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![
                local_truth_response(local_truth_candidate_payload(
                    raw_text_a,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
                local_truth_response(local_truth_candidate_payload(
                    raw_text_b,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 80, "currency": "EUR"}),
                )),
            ],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_a, page_b],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 2);
        assert_eq!(report.verified_rule_count, 0);
        assert_eq!(report.needs_hitl_candidate_count, 2);

        let needs_hitl_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'needs_hitl'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(needs_hitl_candidates, 2);

        let admissible_verified_rows: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM verified.rule_instances
             WHERE context_key = $1 AND publish_admissibility = 'admissible'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(admissible_verified_rows, 0);
    }
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_certification_suite_matches_fixture_expectations() {
        let fixtures = load_truth_certification_fixtures();
        let fixture_ids = env::var("TRUTH_CERT_FIXTURE_IDS").ok().map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        });
        let fixtures = if let Some(fixture_ids) = fixture_ids {
            fixtures
                .into_iter()
                .filter(|fixture| fixture_ids.contains(&fixture.fixture_id))
                .collect::<Vec<_>>()
        } else {
            fixtures
        };
        let fixture_limit = env::var("TRUTH_CERT_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok());
        let fixtures = if let Some(limit) = fixture_limit {
            fixtures.into_iter().take(limit).collect::<Vec<_>>()
        } else {
            fixtures
        };
        assert!(
            !fixtures.is_empty(),
            "expected at least one certification fixture"
        );

        let (source_servers, serp_sequence, rendered_pages) =
            render_fixture_source_pages(&fixtures).await;
        assert_eq!(
            source_servers.len(),
            fixtures
                .iter()
                .map(|fixture| fixture.source_inputs.pages.len())
                .sum::<usize>()
        );
        let serp_stub = stub_servers::spawn_json_sequence_stub(
            "/v3/serp/google/organic/live/advanced",
            serp_sequence
                .into_iter()
                .map(|(_, payload)| payload)
                .collect(),
        )
        .await
        .unwrap();
        let truth_stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            truth_certification_truth_sequence(&fixtures),
        )
        .await
        .unwrap();
        let editorial_stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            truth_certification_editorial_sequence(&fixtures),
        )
        .await
        .unwrap();
        let infra = InfraHarness::start().await.unwrap();
        let temporal = &infra.temporal;

        let mut worker_env = infra.runtime_env();
        worker_env.insert("TEMPORAL_URL".to_string(), temporal.temporal_url.clone());
        worker_env.insert("TEMPORAL_NAMESPACE".to_string(), temporal.namespace.clone());
        worker_env.extend(truth_certification_env(
            &format!(
                "{}/v3/serp/google/organic/live/advanced",
                serp_stub.base_url
            ),
            &format!("{}/v1/chat/completions", truth_stub.base_url),
            &format!("{}/v1/chat/completions", editorial_stub.base_url),
        ));
        let mut worker = spawn_temporal_worker(&worker_env);
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            worker.child.try_wait().unwrap().is_none(),
            "temporal_worker exited before certification workflow start"
        );
        let client = connect_client(
            &temporal.temporal_url,
            format!("truth-cert-client-{}", Uuid::new_v4()),
            &temporal.namespace,
        )
        .await
        .unwrap();
        let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
        let mut reports = Vec::new();

        for fixture in &fixtures {
            assert_eq!(
                fixture.business_scope.truth_identity_tuple, "ES|tourist||BY",
                "fixture {} drifted from certification truth tuple",
                fixture.fixture_id
            );
            assert_eq!(
                fixture.business_scope.publishing_scope_tuple, "alegria-site|ru-RU|ES|tourist|BY",
                "fixture {} drifted from certification publish tuple",
                fixture.fixture_id
            );
            assert!(
                fixture
                    .invariants
                    .no_truth_promotion_from_retrieval_or_graph
            );
            assert!(fixture.invariants.no_source_tier_shortcut_to_verified);
            assert!(
                fixture
                    .invariants
                    .no_unsupported_claim_reaches_draft_as_truth
            );

            let run_id = Uuid::new_v4().to_string();
            let site_input = register_truth_certification_input(
                &repo,
                run_id.clone(),
                format!("truth-cert-batch-{}", fixture.fixture_id),
                fixture.source_inputs.query.clone(),
                fixture.run_mode.clone(),
            )
            .await;
            let scope_signature = site_input
                .scope
                .as_ref()
                .expect("scope payload")
                .scope_signature
                .clone();
            reset_truth_certification_state(&infra.postgres.pool, &site_input.context_key).await;
            seed_truth_certification_source_governance(
                &infra.postgres.pool,
                fixture,
                &rendered_pages,
            )
            .await;
            seed_truth_certification_support_bundle(
                &infra.postgres.pool,
                &site_input.context_key,
                fixture,
            )
            .await;
            let mut outbox_worker = spawn_outbox_worker(&worker_env);
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(
                outbox_worker.child.try_wait().unwrap().is_none(),
                "outbox_worker exited before certification workflow start"
            );

            let approval_task = if fixture.run_mode == "full_auto_after_approval" {
                Some(tokio::spawn(insert_preapproved_decisions_when_ready(
                    infra.postgres.pool.clone(),
                    scope_signature.clone(),
                    Instant::now() + Duration::from_secs(30),
                )))
            } else {
                None
            };

            let handle = client
                .start_workflow(
                    UntypedWorkflow::new("SeoSiteBuildCanonicalCutoverWorkflow"),
                    RawValue::default(),
                    WorkflowStartOptions::new("alegria-pipeline", run_id.clone()).build(),
                )
                .await
                .unwrap();
            let workflow_result = timeout(
                Duration::from_secs(180),
                handle.get_result(WorkflowGetResultOptions::default()),
            )
            .await;
            let workflow_failed = match workflow_result {
                Ok(Ok(_)) => false,
                Ok(Err(_err)) => true,
                Err(err) => {
                    let diagnostics =
                        certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                    panic!(
                        "workflow timed out for fixture {} run_id={} error={err:?}\n{}",
                        fixture.fixture_id, run_id, diagnostics
                    );
                }
            };

            if let Some(task) = approval_task {
                task.await.unwrap();
            }

            let mut report = collect_truth_certification_fixture_report(
                &infra.postgres.pool,
                fixture,
                &run_id,
                &site_input.context_key,
                &scope_signature,
            )
            .await;
            if workflow_failed {
                let diagnostics =
                    certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                if !expected_workflow_failure_allowed(fixture, &report, &diagnostics) {
                    report.pass = false;
                    report.failure_reasons.push("workflow_failed".to_string());
                }
                report.workflow_failure_diagnostics = Some(diagnostics);
            }
            reports.push(report);
            if let Ok(report_path) = env::var("TRUTH_CERT_REPORT_PATH") {
                let partial = TruthCertificationSuiteReport {
                    workflow_type: "SeoSiteBuildCanonicalCutoverWorkflow".to_string(),
                    fixture_count: reports.len(),
                    fixtures: reports.clone(),
                };
                fs::write(&report_path, serde_json::to_vec_pretty(&partial).unwrap()).unwrap();
            }
            drop(outbox_worker);
        }

        let suite_report = TruthCertificationSuiteReport {
            workflow_type: "SeoSiteBuildCanonicalCutoverWorkflow".to_string(),
            fixture_count: reports.len(),
            fixtures: reports,
        };

        if let Ok(report_path) = env::var("TRUTH_CERT_REPORT_PATH") {
            fs::write(
                &report_path,
                serde_json::to_vec_pretty(&suite_report).unwrap(),
            )
            .unwrap();
        }

        assert!(
            suite_report.fixtures.iter().all(|fixture| fixture.pass),
            "truth certification mismatches: {}",
            serde_json::to_string_pretty(&suite_report).unwrap()
        );
    }
