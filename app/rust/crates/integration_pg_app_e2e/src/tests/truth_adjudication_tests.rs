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
