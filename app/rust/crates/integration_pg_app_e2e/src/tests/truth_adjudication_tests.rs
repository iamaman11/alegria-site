    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_adjudication_promotes_corroborated_candidates_into_admissible_verified_truth() {
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
        let (page_a, section_a) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-a.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;
        let (page_b, section_b) = seed_raw_page_with_section(
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
                    section_a,
                )),
                local_truth_response(local_truth_candidate_payload(
                    raw_text,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                    section_b,
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
            "truth_governance@1"
        );
        assert!(verified_row
            .get::<String, _>("adjudication_reason")
            .contains("independent_corroboration"));
    }

    /// This is the legacy raw-crawl → verified Truth path, independent of the
    /// V4 Temporal review Activity. Its source registration must honor an
    /// operator's curatorial downgrade even when the crawler classified the
    /// original page as government evidence. Two real PostgreSQL connections
    /// race the same source row, with no mocked lock or permissive fallback.
    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn raw_ingestion_cannot_reactivate_curator_deprecated_source_under_pg_race() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = format!("es:source-curator-race:{}:by", Uuid::new_v4().simple());
        seed_truth_context_and_concept(
            &harness.pool,
            &context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;
        let source_url = format!("https://curated-{}.example.gov/fees", Uuid::new_v4().simple());
        let (page_id, _section_id) = seed_raw_page_with_section(
            &harness.pool,
            &source_url,
            "government",
            "Fees",
            "Consular fee is 35 EUR",
        )
        .await;
        let source_sections = raw_crawl_adapter::load_raw_sections_by_page_ids(
            &harness.pool,
            &[page_id],
        )
        .await
        .expect("load real source evidence for curatorial downgrade");
        assert_eq!(source_sections.len(), 1);
        raw_crawl_adapter::ensure_source(&harness.pool, &source_sections[0])
            .await
            .expect("initial crawler registration is a proposal, not authority");

        // Hold the curator's uncommitted row update before starting ingestion.
        // The crawler must block at its real FOR SHARE source-registry read,
        // then see the committed downgrade and refuse extraction/verification.
        let mut curator = harness.pool.begin().await.expect("open curator transaction");
        sqlx::query(
            "UPDATE kb.sources
             SET status='deprecated', source_type='low_trust', trust_level=1,
                 authority_class='forum', override_eligible=false
             WHERE source_key=$1",
        )
        .bind(&source_url)
        .execute(&mut *curator)
        .await
        .expect("curator obtains the registry row first");
        let ingestion_pool = harness.pool.clone();
        let ingest_context = context_key.clone();
        let ingest_task = tokio::spawn(async move {
            raw_crawl_adapter::ingest_raw_pages_into_verified(
                &ingestion_pool,
                &Uuid::new_v4().to_string(),
                &ingest_context,
                &[page_id],
            )
            .await
        });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let blocked_on_registry: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                   SELECT 1 FROM pg_stat_activity
                   WHERE state='active' AND wait_event_type='Lock'
                     AND query ILIKE '%kb.sources%'
                )",
            )
            .fetch_one(&harness.pool)
            .await
            .expect("observe actual PostgreSQL registry row lock wait");
            if blocked_on_registry {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "crawler did not reach the expected real kb.sources lock wait"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        curator.commit().await.expect("commit curator downgrade before crawler read");
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), ingest_task)
            .await
            .expect("crawler unblocks after curator commit")
            .expect("ingestion task did not panic");
        assert_eq!(
            result.as_ref().expect_err("crawler accepted deprecated source authority").class_str(),
            "conflict_violation",
            "ingestion must reject deprecated authority before writing Truth"
        );
        let stored: (String, String, i32, String) = sqlx::query_as(
            "SELECT status, source_type, trust_level, authority_class
             FROM kb.sources WHERE source_key=$1",
        )
        .bind(&source_url)
        .fetch_one(&harness.pool)
        .await
        .expect("read curatorial registry verdict");
        assert_eq!(stored, ("deprecated".into(), "low_trust".into(), 1, "forum".into()));
        let proposed: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM extracted.rule_candidates WHERE context_key=$1",
        )
        .bind(&context_key)
        .fetch_one(&harness.pool)
        .await
        .expect("inspect candidate side effects");
        let verified: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key=$1",
        )
        .bind(&context_key)
        .fetch_one(&harness.pool)
        .await
        .expect("inspect verified Truth side effects");
        assert_eq!((proposed, verified), (0, 0), "no stale Truth may be promoted");

        // Repeated crawls must not silently grant a new trust level or
        // re-activate the registry after the competing editor has departed.
        assert_eq!(
            raw_crawl_adapter::ensure_source(&harness.pool, &source_sections[0])
                .await
                .expect_err("crawler reactivated a curated-deprecated source")
                .class_str(),
            "conflict_violation"
        );

        // A curator may retain active status but deliberately reduce trust.
        // The crawler must not use a government dtype to silently GREATEST()
        // that field back up to 5 on a subsequent ingest.
        let trust_only_url = format!(
            "https://curated-active-{}.example.gov/fees",
            Uuid::new_v4().simple()
        );
        let (trust_only_page, _) = seed_raw_page_with_section(
            &harness.pool,
            &trust_only_url,
            "government",
            "Fees",
            "Fee schedule under review",
        )
        .await;
        let trust_only_sections = raw_crawl_adapter::load_raw_sections_by_page_ids(
            &harness.pool,
            &[trust_only_page],
        )
        .await
        .unwrap();
        raw_crawl_adapter::ensure_source(&harness.pool, &trust_only_sections[0])
            .await
            .expect("register a second source before curatorial trust change");
        sqlx::query(
            "UPDATE kb.sources SET trust_level=1, authority_class='agency'
             WHERE source_key=$1",
        )
        .bind(&trust_only_url)
        .execute(&harness.pool)
        .await
        .expect("curator lowers trust but leaves source active");
        raw_crawl_adapter::ensure_source(&harness.pool, &trust_only_sections[0])
            .await
            .expect("active low-trust source can be crawled without reclassification");
        let (curated_trust, curated_class): (i32, String) = sqlx::query_as(
            "SELECT trust_level, authority_class FROM kb.sources WHERE source_key=$1",
        )
        .bind(&trust_only_url)
        .fetch_one(&harness.pool)
        .await
        .expect("inspect persisted curator trust downgrade");
        assert_eq!((curated_trust, curated_class.as_str()), (1, "agency"));
        let effective_registry =
            infrastructure::adapters::sqlx_source_projection_adapter::load_source_registry_entries(
                &harness.pool,
            )
            .await
            .expect("load actual governance registry from PostgreSQL");
        assert_eq!(
            effective_registry
                .get(&trust_only_url)
                .expect("active curated source remains available to governance")
                .trust_level,
            1,
            "government crawler type must not override the database curator's tier"
        );
        eprintln!("CI_PG_V7_CURATOR_DOWNGRADE_INGEST_RACE=PASS");
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn raw_ingestion_refuses_llm_answer_after_concurrent_pg_evidence_rewrite() {
        use std::sync::Arc;
        use tokio::sync::{oneshot, Mutex};

        let harness = PostgresHarness::start().await.unwrap();
        let context_key = format!("es:llm-raw-race:{}:by", Uuid::new_v4().simple());
        seed_truth_context_and_concept(
            &harness.pool,
            &context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;
        let original_text = "Consular fee is 35 EUR";
        let (page_id, section_id) = seed_raw_page_with_section(
            &harness.pool,
            &format!("https://evidence-race-{}.example.gov", Uuid::new_v4().simple()),
            "government",
            "Fees",
            original_text,
        )
        .await;

        // A deterministic HTTP barrier records when the real extractor has
        // consumed the original SQL source and is awaiting an LLM answer.
        // While the network response is held, a second PostgreSQL connection
        // commits changed evidence. No test-only fake source row is involved.
        let (llm_entered_tx, llm_entered_rx) = oneshot::channel::<()>();
        let (release_llm_tx, release_llm_rx) = oneshot::channel::<()>();
        let entered = Arc::new(Mutex::new(Some(llm_entered_tx)));
        let release = Arc::new(Mutex::new(Some(release_llm_rx)));
        let payload = local_truth_response(local_truth_candidate_payload(
            original_text,
            "FEE_ITEM",
            "consular_fee",
            serde_json::json!({"amount": 35, "currency": "EUR"}),
            section_id,
        ));
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post({
                let entered = entered.clone();
                let release = release.clone();
                move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    let payload = payload.clone();
                    async move {
                        if let Some(sender) = entered.lock().await.take() {
                            let _ = sender.send(());
                        }
                        if let Some(receiver) = release.lock().await.take() {
                            let _ = receiver.await;
                        }
                        axum::Json(payload)
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind gated LLM stub");
        let stub_url = format!("http://{}", listener.local_addr().unwrap());
        let stub_task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve gated LLM response");
        });
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{stub_url}/v1/chat/completions"
        )));
        let ingestion_pool = harness.pool.clone();
        let ingest_context = context_key.clone();
        let ingest_task = tokio::spawn(async move {
            raw_crawl_adapter::ingest_raw_pages_into_verified(
                &ingestion_pool,
                &Uuid::new_v4().to_string(),
                &ingest_context,
                &[page_id],
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), llm_entered_rx)
            .await
            .expect("real LLM HTTP request did not arrive")
            .expect("LLM gate dropped before request");
        sqlx::query(
            "UPDATE raw.sections
             SET content_md='Consular fee is 99 EUR', content_hash='updated-evidence'
             WHERE id=$1",
        )
        .bind(section_id)
        .execute(&harness.pool)
        .await
        .expect("direct PostgreSQL raw source writer commits during LLM request");
        release_llm_tx.send(()).expect("resume old LLM response");
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), ingest_task)
            .await
            .expect("legacy extraction finishes after source rewrite")
            .expect("legacy extraction task did not panic");
        stub_task.abort();
        assert_eq!(
            result.as_ref().expect_err("stale extractor answer was accepted").class_str(),
            "conflict_violation",
            "Truth ingestion must reject changed raw evidence before candidate persistence"
        );
        let candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM extracted.rule_candidates WHERE context_key=$1",
        )
        .bind(&context_key)
        .fetch_one(&harness.pool)
        .await
        .expect("inspect stale candidate writes");
        let verified: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key=$1",
        )
        .bind(&context_key)
        .fetch_one(&harness.pool)
        .await
        .expect("inspect stale verified writes");
        assert_eq!((candidates, verified), (0, 0));
        eprintln!("CI_PG_V7_STALE_LLM_SOURCE_REWRITE=PASS");
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_adjudication_marks_incomplete_candidate_as_needs_hitl() {
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
        let (page_id, section_id) = seed_raw_page_with_section(
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
                section_id,
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
    async fn truth_adjudication_keeps_conflicting_structured_candidates_out_of_verified_storage() {
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
        let (page_a, section_a) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-d.example.gov/fees",
            "government",
            "Fees",
            raw_text_a,
        )
        .await;
        let (page_b, section_b) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-e.example.gov/fees",
            "government",
            "Fees",
            raw_text_b,
        )
        .await;

        // Two passes of the same official claims: replay must preserve the
        // adjudicated HITL state without promoting or duplicating either claim.
        let responses = vec![
            local_truth_response(local_truth_candidate_payload(
                raw_text_a,
                "FEE_ITEM",
                "consular_fee",
                serde_json::json!({"amount": 35, "currency": "EUR"}),
                section_a,
            )),
            local_truth_response(local_truth_candidate_payload(
                raw_text_b,
                "FEE_ITEM",
                "consular_fee",
                serde_json::json!({"amount": 80, "currency": "EUR"}),
                section_b,
            )),
        ];
        let mut replay_responses = responses.clone();
        replay_responses.extend(responses);
        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            replay_responses,
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
        let replay = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_a, page_b],
        )
        .await
        .unwrap();
        assert_eq!(replay.extracted_rule_count, 2);
        assert_eq!(replay.verified_rule_count, 0);
        assert_eq!(replay.needs_hitl_candidate_count, 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*)::bigint FROM extracted.rule_candidates WHERE context_key = $1",
            )
            .bind(context_key)
            .fetch_one(&harness.pool)
            .await
            .unwrap(),
            2,
            "replayed official conflict must retain two canonical candidates",
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 \
                 AND publish_admissibility = 'admissible'",
            )
            .bind(context_key)
            .fetch_one(&harness.pool)
            .await
            .unwrap(),
            0,
            "replay must not promote disputed Truth",
        );
    }
