    // This test runs against ONLY the GitHub Actions disposable PostgreSQL
    // schema. It exercises the real SQLx writer and the public support reader.
    #[tokio::test]
    async fn scoped_verified_support_round_trip_is_profile_bound() {
        if env::var("ALEGRIA_SCOPED_TRUTH_TEST").ok().as_deref() != Some("true") {
            return;
        }
        let database_url = env::var("DATABASE_URL").expect("CI scoped truth database URL");
        assert!(
            database_url.contains("@127.0.0.1:5432/alegria_ci"),
            "scoped truth integration test refuses a non-disposable database"
        );
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("disposable PostgreSQL");
        let marker = Uuid::new_v4().simple().to_string();
        let context_key = format!("ci:scoped-truth:{marker}");
        let country_code = format!("X{}", &marker[..8]);
        let concept_key = format!("scoped_fee_{marker}");
        let rule_instance_id = format!("scoped_rule_{marker}");
        let candidate_id = format!("scoped_candidate_{marker}");
        let source_key = format!("https://ci-scoped-{marker}.example.gov/fees");
        let raw_text = "Consular fee is 35 EUR";
        let scope = serde_json::json!({"applicant_profile": "student"});
        let params = serde_json::json!({"amount": 35, "currency": "EUR"});

        sqlx::query(
            "INSERT INTO kb.visa_contexts
             (context_key, country_code, visa_family, citizenship_code)
             VALUES ($1, $2, 'tourist', 'BY')",
        )
        .bind(&context_key)
        .bind(&country_code)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb.concepts (concept_key, concept_type, label_ru)
             VALUES ($1, 'fee', 'Consular fee')",
        )
        .bind(&concept_key)
        .execute(&pool)
        .await
        .unwrap();
        for (key, kind) in [("student", "status"), ("minor", "age")] {
            sqlx::query(
                "INSERT INTO kb.applicant_profiles (profile_key, profile_type, label_ru)
                 VALUES ($1, $2, $1)
                 ON CONFLICT (profile_key) DO NOTHING",
            )
            .bind(key)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO kb.sources
             (source_key, source_type, source_label, trust_level)
             VALUES ($1, 'government', 'Official source', 5)",
        )
        .bind(&source_key)
        .execute(&pool)
        .await
        .unwrap();
        let page_id: i64 = sqlx::query_scalar(
            "INSERT INTO raw.pages (url, domain, dtype)
             VALUES ($1, $2, 'government') RETURNING id",
        )
        .bind(&source_key)
        .bind(format!("ci-scoped-{marker}.example.gov"))
        .fetch_one(&pool)
        .await
        .unwrap();
        let section_id: i64 = sqlx::query_scalar(
            "INSERT INTO raw.sections (page_id, content_md)
             VALUES ($1, $2) RETURNING id",
        )
        .bind(page_id)
        .bind(raw_text)
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut write_candidate =
            infrastructure::adapters::sqlx_seo_adapter::VerifiedTruthCandidateWrite {
                rule_candidate_id: candidate_id.clone(),
                context_key: context_key.clone(),
                section_id,
                role: "FEE_ITEM".to_string(),
                concept_canonical_key: concept_key.clone(),
                evidence_quote: raw_text.to_string(),
                params: params.clone(),
                scope: scope.clone(),
                severity: "mandatory".to_string(),
                is_numeric: true,
                is_incomplete: false,
                confidence: 0.98,
                span_start: 0,
                span_end: raw_text.len() as i32,
                source_key: source_key.clone(),
                source_snapshot_hash: "test-source-snapshot".to_string(),
                provider_key: "chat_manual".to_string(),
                model_key: "gpt-6".to_string(),
                prompt_version: "truth_extraction_prompt@1".to_string(),
                epistemic_status: "needs_hitl".to_string(),
            };
        infrastructure::adapters::sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication(
            &pool,
            &write_candidate,
        )
        .await
        .unwrap();
        // The SQL write must retain the original typed ChatGPT attribution;
        // a candidate proposal remains unverified until governance approves it.
        let provenance: (String, String, String) = sqlx::query_as(
            "SELECT llm_provider, llm_model, prompt_version FROM extracted.rule_candidates WHERE rule_candidate_id = $1",
        )
        .bind(&candidate_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(provenance, ("chat_manual".to_string(), "gpt-6".to_string(), "truth_extraction_prompt@1".to_string()));

        // A real SQL queue receives one review for a needs_hitl candidate,
        // regardless of how many times the same workflow activity replays.
        let hits = infrastructure::adapters::sqlx_hitl_adapter::reconcile_truth_candidate_reviews(
            &pool, &context_key, "ci-truth-run",
        )
        .await
        .unwrap();
        assert_eq!(hits, 1);
        let replay = infrastructure::adapters::sqlx_hitl_adapter::reconcile_truth_candidate_reviews(
            &pool, &context_key, "ci-truth-run",
        )
        .await
        .unwrap();
        assert_eq!(replay, 0);
        let reviews = infrastructure::adapters::sqlx_hitl_adapter::list_truth_candidate_reviews(
            &pool, &context_key, 10,
        )
        .await
        .unwrap();
        assert_eq!(reviews.len(), 1);
        let task = reviews[0]["task_key"].as_str().unwrap().to_string();
        assert_eq!(reviews[0]["queue_state"], "open");
        assert_eq!(reviews[0]["candidate"]["source_snapshot_hash"], "test-source-snapshot");
        assert_eq!(reviews[0]["candidate"]["llm_provider"], "chat_manual");
        assert_eq!(reviews[0]["candidate"]["evidence_quote"], raw_text);
        assert!(infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
            &pool, &task, "seo_system", "approve", "test approval",
        )
        .await
        .is_err());
        let changes_requested =
            infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
                &pool, &task, "ci_human_reviewer", "request_changes", "source is incomplete",
            )
            .await
            .unwrap();
        assert_eq!(changes_requested, "reopened");
        let unchanged_after_changes: String = sqlx::query_scalar(
            "SELECT epistemic_status FROM extracted.rule_candidates WHERE rule_candidate_id = $1",
        )
        .bind(&candidate_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unchanged_after_changes, "needs_hitl");
        let reviewed = infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
            &pool, &task, "ci_human_reviewer", "approve", "source checked",
        )
        .await
        .unwrap();
        assert_eq!(reviewed, "in_review");
        let same = infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
            &pool, &task, "ci_human_reviewer", "approve", "source checked",
        )
        .await
        .unwrap();
        assert_eq!(same, "in_review");
        assert!(infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
            &pool, &task, "other_reviewer", "reject", "different decision",
        )
        .await
        .is_err());
        let task_after =
            infrastructure::adapters::sqlx_hitl_adapter::show_truth_candidate_review(
                &pool, &task,
            )
            .await
            .unwrap();
        assert_eq!(task_after["queue_state"], "in_review");
        assert_eq!(task_after["audit_log"].as_array().unwrap().len(), 3);
        let not_verified: String = sqlx::query_scalar(
            "SELECT epistemic_status FROM extracted.rule_candidates WHERE rule_candidate_id = $1",
        )
        .bind(&candidate_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(not_verified, "needs_hitl");
        let after_review_replay =
            infrastructure::adapters::sqlx_hitl_adapter::reconcile_truth_candidate_reviews(
                &pool, &context_key, "ci-truth-run",
            )
            .await
            .unwrap();
        assert_eq!(after_review_replay, 0);

        // This separate synthetic writer fixture models an independent
        // verification result; the human queue above did NOT promote it.
        write_candidate.epistemic_status = "verified".to_string();
        infrastructure::adapters::sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication(
            &pool, &write_candidate,
        )
        .await
        .unwrap();

        // A review of an earlier needs_hitl candidate is not a reusable
        // approval for a later epistemic status or changed source snapshot.
        assert!(
            infrastructure::adapters::sqlx_hitl_adapter::decide_truth_candidate_review(
                &pool, &task, "ci_human_reviewer", "approve", "source checked",
            )
            .await
            .is_err()
        );

        let reconciled_after_independent_verification =
            infrastructure::adapters::sqlx_hitl_adapter::reconcile_truth_candidate_reviews(
                &pool, &context_key, "ci-truth-rechecked",
            )
            .await
            .unwrap();
        assert_eq!(reconciled_after_independent_verification, 0);
        let superseded =
            infrastructure::adapters::sqlx_hitl_adapter::show_truth_candidate_review(
                &pool, &task,
            )
            .await
            .unwrap();
        assert_eq!(superseded["queue_state"], "superseded");
        assert_eq!(superseded["audit_log"].as_array().unwrap().len(), 4);

        let write_rule = infrastructure::adapters::sqlx_seo_adapter::VerifiedRuleInstanceWrite {
            scope,
            rule_instance_id: rule_instance_id.clone(),
            context_key: context_key.clone(),
            rule_type_key: "fee_item".to_string(),
            concept_key: concept_key.clone(),
            role_type: "fee_item".to_string(),
            params,
            source_key,
            confidence: 0.98,
            rule_candidate_id: candidate_id,
            evidence_section_id: section_id,
            evidence_quote: raw_text.to_string(),
            span_start: 0,
            span_end: raw_text.len() as i32,
            source_snapshot_hash: "test-source-snapshot".to_string(),
            verification_method: "ci-profile-scope".to_string(),
            adjudication_reason: "independent evidence fixture".to_string(),
            publish_admissibility: "admissible".to_string(),
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
        };
        infrastructure::adapters::sqlx_seo_adapter::upsert_verified_rule_instance(
            &pool,
            &write_rule,
        )
        .await
        .unwrap();

        for profile in ["student", "minor"] {
            let run_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO pipeline.execution_runs
                 (run_id, workflow_run_id, workflow_type, context_key)
                 VALUES ($1, $2, 'seo_site_build', $3)",
            )
            .bind(run_id)
            .bind(format!("ci-scoped-{run_id}"))
            .bind(&context_key)
            .execute(&pool)
            .await
            .unwrap();
            let supports =
                infrastructure::adapters::sqlx_seo_adapter::load_verified_support_bundle(
                    &pool,
                    &run_id.to_string(),
                    &context_key,
                    "ci-profile-scope",
                    profile,
                )
                .await
                .unwrap();
            if profile == "student" {
                assert_eq!(supports.len(), 1);
                assert_eq!(supports[0].support_ref, rule_instance_id);
            } else {
                assert!(supports.is_empty(), "minor may not see student-only fee");
            }
        }

        // Context-only/global readers lack applicant identity, so they must
        // never expose this otherwise admissible student fact.
        let public_rules =
            infrastructure::adapters::sqlx_verified_rule_instances_adapter::read_verified_rules(
                &pool,
                &context_key,
            )
            .await
            .unwrap();
        assert!(public_rules.is_empty());
        let context_rules =
            infrastructure::adapters::sqlx_context_bundle_adapter::load_context_bundle_rules(
                &pool,
                &context_key,
            )
            .await
            .unwrap();
        assert!(context_rules.is_empty());

        // Re-verification cannot overwrite a manually changed restrictive
        // profile policy, even though the source fact remains verified.
        sqlx::query(
            "UPDATE verified.rule_instance_profiles
             SET applicability = 'excludes'
             WHERE rule_instance_id = $1 AND profile_key = 'student'",
        )
        .bind(&rule_instance_id)
        .execute(&pool)
        .await
        .unwrap();
        let result = infrastructure::adapters::sqlx_seo_adapter::upsert_verified_rule_instance(
            &pool,
            &write_rule,
        )
        .await;
        assert!(result.is_err(), "curated profile restriction must be preserved");
        let persisted: String = sqlx::query_scalar(
            "SELECT applicability FROM verified.rule_instance_profiles
             WHERE rule_instance_id = $1 AND profile_key = 'student'",
        )
        .bind(&rule_instance_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(persisted, "excludes");
        sqlx::query(
            "UPDATE verified.rule_instance_profiles
             SET applicability = 'applies', condition_json = '{\"v\":1}'::jsonb
             WHERE rule_instance_id = $1 AND profile_key = 'student'",
        )
        .bind(&rule_instance_id)
        .execute(&pool)
        .await
        .unwrap();
        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO pipeline.execution_runs
             (run_id, workflow_run_id, workflow_type, context_key)
             VALUES ($1, $2, 'seo_site_build', $3)",
        )
        .bind(run_id)
        .bind(format!("ci-conditional-{run_id}"))
        .bind(&context_key)
        .execute(&pool)
        .await
        .unwrap();
        let conditional_support =
            infrastructure::adapters::sqlx_seo_adapter::load_verified_support_bundle(
                &pool,
                &run_id.to_string(),
                &context_key,
                "ci-profile-scope",
                "student",
            )
            .await
            .unwrap();
        assert!(conditional_support.is_empty());
        let blob: Vec<u8> = sqlx::query_scalar(
            "SELECT payload_bytes FROM pipeline.execution_run_blobs
             WHERE run_id = $1 AND field_name = 'verified_support_bundle'",
        )
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&blob).unwrap();
        assert!(
            diagnostics["unresolved_rule_count"].as_i64().unwrap_or_default() > 0,
            "conditional applicability must be visible in diagnostics"
        );

        // A verified global exception with an unevaluated condition cannot
        // silently waive or modify the fact for every applicant.
        sqlx::query(
            "UPDATE verified.rule_instance_profiles
             SET condition_json = NULL
             WHERE rule_instance_id = $1 AND profile_key = 'student'",
        )
        .bind(&rule_instance_id)
        .execute(&pool)
        .await
        .unwrap();
        let exception_id = format!("ci_exception_{marker}");
        sqlx::query(
            "INSERT INTO verified.rule_exceptions
             (exception_id, rule_instance_id, exception_key, condition_json,
              override_kind, override_payload, status)
             VALUES ($1, $2, 'conditional_waiver', '{}'::jsonb,
                     'waive', '{}'::jsonb, 'verified')",
        )
        .bind(&exception_id)
        .bind(&rule_instance_id)
        .execute(&pool)
        .await
        .unwrap();
        let exception_run = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO pipeline.execution_runs
             (run_id, workflow_run_id, workflow_type, context_key)
             VALUES ($1, $2, 'seo_site_build', $3)",
        )
        .bind(exception_run)
        .bind(format!("ci-exception-{exception_run}"))
        .bind(&context_key)
        .execute(&pool)
        .await
        .unwrap();
        let exception_support =
            infrastructure::adapters::sqlx_seo_adapter::load_verified_support_bundle(
                &pool,
                &exception_run.to_string(),
                &context_key,
                "ci-profile-scope",
                "student",
            )
            .await
            .unwrap();
        assert!(exception_support.is_empty());
        let exception_blob: Vec<u8> = sqlx::query_scalar(
            "SELECT payload_bytes FROM pipeline.execution_run_blobs
             WHERE run_id = $1 AND field_name = 'verified_support_bundle'",
        )
        .bind(exception_run)
        .fetch_one(&pool)
        .await
        .unwrap();
        let exception_diagnostics: serde_json::Value =
            serde_json::from_slice(&exception_blob).unwrap();
        assert!(exception_diagnostics["unresolved_rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reason_code"] == "unresolved_exception_condition"));
        sqlx::query("DELETE FROM verified.rule_exceptions WHERE exception_id = $1")
            .bind(&exception_id)
            .execute(&pool)
            .await
            .unwrap();

        // A global and a student-only value for the same semantic concept
        // must not be jointly served as two independent "verified" facts.
        let global_raw_text = "Consular fee is 60 EUR";
        let global_section_id: i64 = sqlx::query_scalar(
            "INSERT INTO raw.sections (page_id, content_md)
             VALUES ($1, $2) RETURNING id",
        )
        .bind(page_id)
        .bind(global_raw_text)
        .fetch_one(&pool)
        .await
        .unwrap();
        let global_candidate_id = format!("ci_global_candidate_{marker}");
        let global_rule_id = format!("ci_global_rule_{marker}");
        let mut global_candidate = write_candidate.clone();
        global_candidate.rule_candidate_id = global_candidate_id.clone();
        global_candidate.section_id = global_section_id;
        global_candidate.scope = serde_json::json!({});
        global_candidate.params = serde_json::json!({"amount": 60, "currency": "EUR"});
        global_candidate.evidence_quote = global_raw_text.to_string();
        global_candidate.span_end = global_raw_text.len() as i32;
        infrastructure::adapters::sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication(
            &pool,
            &global_candidate,
        )
        .await
        .unwrap();
        let mut global_rule = write_rule.clone();
        global_rule.rule_instance_id = global_rule_id.clone();
        global_rule.rule_candidate_id = global_candidate_id;
        global_rule.scope = serde_json::json!({});
        global_rule.params = serde_json::json!({"amount": 60, "currency": "EUR"});
        global_rule.evidence_section_id = global_section_id;
        global_rule.evidence_quote = global_raw_text.to_string();
        global_rule.span_end = global_raw_text.len() as i32;
        infrastructure::adapters::sqlx_seo_adapter::upsert_verified_rule_instance(
            &pool,
            &global_rule,
        )
        .await
        .unwrap();
        for profile in ["student", "minor"] {
            let run = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO pipeline.execution_runs
                 (run_id, workflow_run_id, workflow_type, context_key)
                 VALUES ($1, $2, 'seo_site_build', $3)",
            )
            .bind(run)
            .bind(format!("ci-overlap-{run}"))
            .bind(&context_key)
            .execute(&pool)
            .await
            .unwrap();
            let supports =
                infrastructure::adapters::sqlx_seo_adapter::load_verified_support_bundle(
                    &pool,
                    &run.to_string(),
                    &context_key,
                    "ci-profile-scope",
                    profile,
                )
                .await
                .unwrap();
            if profile == "student" {
                assert!(
                    supports.is_empty(),
                    "overlapping global/profile fee values require review"
                );
                let blob: Vec<u8> = sqlx::query_scalar(
                    "SELECT payload_bytes FROM pipeline.execution_run_blobs
                     WHERE run_id = $1 AND field_name = 'verified_support_bundle'",
                )
                .bind(run)
                .fetch_one(&pool)
                .await
                .unwrap();
                let report: serde_json::Value = serde_json::from_slice(&blob).unwrap();
                assert!(report["unresolved_rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| {
                        entry["reason_code"] == "overlapping_profile_and_global_truth"
                    }));
            } else {
                assert_eq!(supports.len(), 1);
                assert_eq!(supports[0].support_ref, global_rule_id);
            }
        }
    }

    // Regression: the strict graph probe must send every Cypher parameter
    // to the real Neo4j driver, not just interpolate it in diagnostic text.
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn graph_capability_probe_binds_context_key() {
        let neo4j = Neo4jHarness::start().await.unwrap();
        let _env = env_overrides::EnvOverrideGuard::apply([
            ("NEO4J_URI".to_string(), neo4j.uri.clone()),
            ("NEO4J_USER".to_string(), neo4j.user.clone()),
            ("NEO4J_PASSWORD".to_string(), neo4j.password.clone()),
            ("GRAPH_CAPABILITY_REQUIRED".to_string(), "true".to_string()),
            ("GRAPH_QUERY_REQUIRED".to_string(), "true".to_string()),
            ("GRAPH_GDS_REQUIRED".to_string(), "false".to_string()),
        ]);
        infrastructure::adapters::graph_capability_adapter::ensure_graph_contract_if_required(
            "graph-probe-regression",
        )
        .await
        .expect("Neo4j query must receive the context_key parameter");
    }

    // Real Qdrant: materialize through the same REST outbox adapter used by
    // workers, then read the point through the independent gRPC client.
    // The container publishes two distinct ephemeral host ports.
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval() {
        use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
        use prost::Message;

        let qdrant = QdrantHarness::start().await.unwrap();
        assert_ne!(
            qdrant.qdrant_url, qdrant.qdrant_rest_url,
            "REST and gRPC must use independently mapped endpoints"
        );
        let _env = env_overrides::EnvOverrideGuard::apply([
            ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
            ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
            ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
        ]);
        let entity_key = format!("raw_section:{}", Uuid::new_v4());
        let collection = "raw_chunks_ctx";
        let vector = vec![0.01_f32; 1024];
        let cmd = QdrantUpsertCommand {
            event_id: String::new(),
            collection_name: collection.to_string(),
            entity_type: "raw_section".to_string(),
            entity_key: entity_key.clone(),
            point_id: Uuid::new_v4().to_string(),
            vector: vector.clone(),
            payload: None,
            distance: "cosine".to_string(),
            vector_size: vector.len() as u32,
            metadata: std::collections::HashMap::from([(
                "retrieval_text".to_string(),
                "authoritative raw section".to_string(),
            )]),
        };
        infrastructure::adapters::projection_materialize_adapter::dispatch_event(
            "qdrant",
            "QdrantUpsertCommand",
            &entity_key,
            "alegria.outbox.qdrant_upsert_command.v1",
            &cmd.encode_to_vec(),
        )
        .await
        .expect("Qdrant outbox adapter must materialize through REST");

        let points = infrastructure::adapters::qdrant_client_adapter::search_dense(
            &qdrant.client,
            collection,
            vector,
            5,
            None,
        )
        .await
        .expect("Qdrant gRPC search must read the REST materialization");
        assert!(
            points.iter().any(|point| {
                infrastructure::adapters::qdrant_client_adapter::scored_point_payload_map(point)
                    .get("entity_key")
                    == Some(&entity_key)
            }),
            "raw_chunks_ctx REST-upserted point is missing from gRPC search"
        );
    }

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
        let infra = InfraHarness::start().await.unwrap();
        let temporal = &infra.temporal;
        let voyage_stub = stub_servers::spawn_voyage_contract_stub().await.unwrap();

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
            // A failed/short-circuited workflow must not consume the next
            // fixture's provider replies. Keep infrastructure shared, but
            // bind each worker to independently queued deterministic stubs.
            let serp_reply = serp_sequence
                .iter()
                .find(|(fixture_id, _)| fixture_id == &fixture.fixture_id)
                .map(|(_, payload)| payload.clone())
                .expect("SERP response for current fixture");
            let serp_stub = stub_servers::spawn_standard_serp_stub(vec![serp_reply])
            .await
            .unwrap();
            let truth_stub = stub_servers::spawn_json_sequence_stub(
                "/v1/chat/completions",
                truth_certification_truth_sequence(std::slice::from_ref(fixture)),
            )
            .await
            .unwrap();
            let editorial_stub = stub_servers::spawn_json_sequence_stub(
                "/v1/chat/completions",
                truth_certification_editorial_sequence(std::slice::from_ref(fixture)),
            )
            .await
            .unwrap();

            let mut worker_env = infra.runtime_env();
            worker_env.insert("VOYAGE_API_KEY".to_string(), "truth-cert-stub".to_string());
            worker_env.insert(
                "VOYAGE_API_BASE_URL".to_string(),
                format!("{}/v1", voyage_stub.base_url),
            );
            worker_env.insert("TEMPORAL_URL".to_string(), temporal.temporal_url.clone());
            worker_env.insert("TEMPORAL_NAMESPACE".to_string(), temporal.namespace.clone());
            worker_env.extend(truth_certification_env(
                &format!(
                    "{}/v3/serp/google/organic/task_post",
                    serp_stub.base_url
                ),
                &format!("{}/v1/chat/completions", truth_stub.base_url),
                &format!("{}/v1/chat/completions", editorial_stub.base_url),
            ));
            let mut worker = spawn_temporal_worker(&worker_env);
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(
                worker.child.try_wait().unwrap().is_none(),
                "temporal_worker exited before fixture {}",
                fixture.fixture_id
            );
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
            let workflow_error = match workflow_result {
                Ok(Ok(_)) => None,
                Ok(Err(err)) => Some(format!("{err:#}")),
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
            if let Some(error) = workflow_error {
                // The Temporal result error is the authoritative failure
                // explanation. The step journal alone can end in a successful
                // sync activity even when a workflow-level drain guard fails.
                let diagnostics =
                    certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                if !expected_workflow_failure_allowed(fixture, &report, &diagnostics) {
                    report.pass = false;
                    report.failure_reasons.push("workflow_failed".to_string());
                }
                report.workflow_failure_diagnostics =
                    Some(format!("workflow_result_error:{error}\n{diagnostics}"));
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
            drop(worker);
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
