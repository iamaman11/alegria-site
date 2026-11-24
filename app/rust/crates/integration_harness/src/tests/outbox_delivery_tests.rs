/// A14: projection metadata and the corresponding Outbox intent are one PG
/// transaction; the same event can materialize more than once in real Qdrant.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_verified_rule_projection_atomic_stage_replay_and_qdrant_delete() {
    use infrastructure::adapters::{
        projection_materialize_adapter::dispatch_event,
        qdrant_client_adapter::{self, VerifiedRuleReadbackPoint},
        verified_rule_projection_adapter::{
            stage_rule_point_deletes_in_tx, stage_rule_upserts_in_tx, VerifiedRuleEmbeddingAdapter,
        },
    };
    use seo_application::knowledge::prepare_verified_rule_embedding_batch;
    use seo_ports::knowledge::{VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch};

    let pg = PostgresHarness::start().await.unwrap();
    let qdrant = QdrantHarness::start().await.unwrap();
    let voyage = stub_servers::spawn_voyage_contract_stub().await.unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("DATABASE_URL".to_string(), pg.database_url.clone()),
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
        ("VOYAGE_API_KEY".to_string(), "projection-a14-stub".to_string()),
        ("VOYAGE_API_BASE_URL".to_string(), format!("{}/v1", voyage.base_url)),
        ("RETRIEVAL_CAPABILITY_REQUIRED".to_string(), "true".to_string()),
    ]);
    let run_id = Uuid::new_v4().to_string();
    let context_key = format!("projection-a14-{}", Uuid::new_v4().simple());
    let rule_id = format!("rule-a14-{}", Uuid::new_v4().simple());
    let make_candidate = |rule_id: &str| VerifiedRuleProjectionCandidate {
        rule_instance_id: rule_id.to_string(),
        role_type: "document_required".into(),
        concept_key: "passport".into(),
        source_key: "https://example.gov/passport".into(),
        evidence_quote: "A valid passport is mandatory".into(),
    };
    let command = VerifiedRuleUpsertBatch {
        run_id: run_id.clone(),
        context_key: context_key.clone(),
        upserts: vec![make_candidate(&rule_id)],
    };
    let adapter = VerifiedRuleEmbeddingAdapter;
    let embeddings = prepare_verified_rule_embedding_batch(&adapter, &command)
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        let mut tx = pg.pool.begin().await.unwrap();
        stage_rule_upserts_in_tx(&mut tx, &command, embeddings.clone())
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let points: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!((points, events), (1, 1), "replay must leave one registry point and one event");
    let (point_id, payload): (String, Vec<u8>) = {
        let point_id: String = sqlx::query_scalar(
            "SELECT point_id FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
        ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
        let payload: Vec<u8> = sqlx::query_scalar(
            "SELECT payload_bytes FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'",
        ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
        (point_id, payload)
    };
    let expected = [VerifiedRuleReadbackPoint {
        point_id, rule_instance_id: rule_id.clone(),
        context_key: context_key.clone(), concept_key: "passport".into(),
        source_key: "https://example.gov/passport".into(),
    }];
    assert!(!qdrant_client_adapter::verify_verified_rule_readback(&expected).await.unwrap());
    for _ in 0..2 {
        dispatch_event("qdrant", "QdrantUpsertCommand", &rule_id,
            "alegria.outbox.qdrant_upsert_command.v1", &payload).await.unwrap();
    }
    assert!(qdrant_client_adapter::verify_verified_rule_readback(&expected).await.unwrap());

    // Force a real database error *between* point upsert and Outbox insert.
    // The transaction must roll back both rather than leaving a false receipt.
    sqlx::query(
        "CREATE FUNCTION system.reject_projection_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.event_type = 'QdrantUpsertCommand' THEN RAISE EXCEPTION 'injected outbox rejection'; \
         END IF; RETURN NEW; END $$",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_projection_outbox BEFORE INSERT ON system.sync_outbox \
         FOR EACH ROW EXECUTE FUNCTION system.reject_projection_outbox()",
    ).execute(&pg.pool).await.unwrap();
    let second_rule_id = format!("rule-a14-next-{}", Uuid::new_v4().simple());
    let next = VerifiedRuleUpsertBatch {
        upserts: vec![make_candidate(&second_rule_id)], ..command.clone()
    };
    let next_embeddings = prepare_verified_rule_embedding_batch(&adapter, &next)
        .await
        .unwrap()
        .unwrap();
    let mut failing_tx = pg.pool.begin().await.unwrap();
    assert!(stage_rule_upserts_in_tx(&mut failing_tx, &next, next_embeddings.clone())
        .await
        .is_err());
    failing_tx.rollback().await.unwrap();
    let rollback_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1",
    ).bind(&second_rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(rollback_count, 0, "failed Outbox insertion must undo point metadata");
    sqlx::query("DROP TRIGGER reject_projection_outbox ON system.sync_outbox")
        .execute(&pg.pool).await.unwrap();
    sqlx::query("DROP FUNCTION system.reject_projection_outbox()")
        .execute(&pg.pool).await.unwrap();
    let mut retry_tx = pg.pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut retry_tx, &next, next_embeddings)
        .await
        .unwrap();
    retry_tx.commit().await.unwrap();
    let recovered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1",
    ).bind(&second_rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(recovered, 1, "next independent revision must be retryable");

    let delete_rule_ids = vec![rule_id.clone()];
    // A late SQL/Outbox failure must not delete the external Qdrant point or
    // the canonical registry receipt. Deletion is a durable command first.
    sqlx::query(
        "ALTER TABLE system.sync_outbox ADD CONSTRAINT reject_qdrant_delete_outbox \
         CHECK (event_type <> 'QdrantDeleteCommand')",
    )
    .execute(&pg.pool)
    .await
    .unwrap();
    let mut failing_delete_tx = pg.pool.begin().await.unwrap();
    assert!(stage_rule_point_deletes_in_tx(
        &mut failing_delete_tx, &run_id, &context_key, &delete_rule_ids,
    )
    .await
    .is_err());
    failing_delete_tx.rollback().await.unwrap();
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(retained, 1, "failed Qdrant delete Outbox must roll back registry");
    assert!(qdrant_client_adapter::verify_verified_rule_readback(&expected)
        .await
        .unwrap());
    sqlx::query("ALTER TABLE system.sync_outbox DROP CONSTRAINT reject_qdrant_delete_outbox")
        .execute(&pg.pool)
        .await
        .unwrap();

    for _ in 0..2 {
        let mut tx = pg.pool.begin().await.unwrap();
        stage_rule_point_deletes_in_tx(
            &mut tx, &run_id, &context_key, &delete_rule_ids,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    // Qdrant is eventually consistent via Outbox; SQL state is atomic at commit.
    assert!(qdrant_client_adapter::verify_verified_rule_readback(&expected)
        .await
        .unwrap());
    let deleted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(deleted, 0);
    let deletion_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND event_type='QdrantDeleteCommand'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(deletion_events, 1, "same deletion replay must not duplicate Outbox");
    let delete_payload: Vec<u8> = sqlx::query_scalar(
        "SELECT payload_bytes FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND event_type='QdrantDeleteCommand'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    // An old delayed insert may arrive after the delete intent. The
    // rule-instance projection uses the newest SQL intent as its fencing key.
    dispatch_event(
        "qdrant",
        "QdrantUpsertCommand",
        &rule_id,
        "alegria.outbox.qdrant_upsert_command.v1",
        &payload,
    )
    .await
    .unwrap();
    for _ in 0..2 {
        dispatch_event(
            "qdrant",
            "QdrantDeleteCommand",
            &rule_id,
            "alegria.outbox.qdrant_delete_command.v1",
            &delete_payload,
        )
        .await
        .unwrap();
    }
    assert!(!qdrant_client_adapter::verify_verified_rule_readback(&expected)
        .await
        .unwrap());
    dispatch_event(
        "qdrant",
        "QdrantUpsertCommand",
        &rule_id,
        "alegria.outbox.qdrant_upsert_command.v1",
        &payload,
    )
    .await
    .unwrap();
    assert!(!qdrant_client_adapter::verify_verified_rule_readback(&expected)
        .await
        .unwrap(), "stale Qdrant upsert resurrected a deleted rule");
}

/// A14/P0: the real Truth SQL writers compose into one transaction, including
/// source registration, canonical candidates, HITL review and rule instances.
/// Failure at the SECOND verified rule cannot expose the first verified fact.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_truth_write_multi_candidate_late_failure_rolls_back_and_retries() {
    use infrastructure::adapters::{raw_crawl_adapter, sqlx_hitl_adapter, sqlx_seo_adapter};

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let context_key = format!("a14-truth-atomic-{marker}");
    let first_concept = format!("a14-truth-first-{marker}");
    let second_concept = format!("a14-truth-second-{marker}");
    let hold_concept = format!("a14-truth-hold-{marker}");
    seed_truth_context_and_concept(pool, &context_key, &first_concept, "fee", "First fee").await;
    for concept in [&second_concept, &hold_concept] {
        sqlx::query(
            "INSERT INTO kb.concepts (concept_key,concept_type,label_ru) VALUES ($1,'fee','Atomic fee')",
        )
        .bind(concept)
        .execute(pool)
        .await
        .unwrap();
    }
    let first_url = format!("https://truth-atomic-first-{marker}.example.gov/fee");
    let second_url = format!("https://truth-atomic-second-{marker}.example.gov/fee");
    let quote = "Consular fee is 35 EUR";
    let (_, first_section) =
        seed_raw_page_with_section(pool, &first_url, "government", "Fee", quote).await;
    let (_, second_section) =
        seed_raw_page_with_section(pool, &second_url, "government", "Fee", quote).await;
    let sections = raw_crawl_adapter::load_raw_sections_by_ids(
        pool,
        &[first_section, second_section],
    )
    .await
    .unwrap();
    assert_eq!(sections.len(), 2);

    let candidates = [
        (first_section, &first_url, &first_concept, "verified"),
        (second_section, &second_url, &second_concept, "verified"),
        (first_section, &first_url, &hold_concept, "needs_hitl"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (section_id, source_key, concept_key, status))| {
        sqlx_seo_adapter::VerifiedTruthCandidateWrite {
            rule_candidate_id: format!("a14-truth-candidate-{index}-{marker}"),
            context_key: context_key.clone(),
            section_id,
            role: "FEE_ITEM".into(),
            concept_canonical_key: concept_key.clone(),
            evidence_quote: quote.into(),
            params: serde_json::json!({"amount":35,"currency":"EUR"}),
            scope: serde_json::json!({}),
            severity: "mandatory".into(),
            is_numeric: true,
            is_incomplete: false,
            confidence: 0.98,
            span_start: 0,
            span_end: quote.len() as i32,
            source_key: source_key.clone(),
            source_snapshot_hash: format!("a14-truth-section-{section_id}"),
            provider_key: "chat_manual".into(),
            model_key: "gpt-6".into(),
            prompt_version: "a14-atomic-test@1".into(),
            epistemic_status: status.into(),
        }
    })
    .collect::<Vec<_>>();
    let rules = candidates[..2]
        .iter()
        .enumerate()
        .map(|(index, candidate)| sqlx_seo_adapter::VerifiedRuleInstanceWrite {
            rule_instance_id: format!(
                "a14-truth-{}-{marker}",
                if index == 0 { "first" } else { "second" }
            ),
            context_key: context_key.clone(),
            rule_type_key: "fee_item".into(),
            concept_key: candidate.concept_canonical_key.clone(),
            role_type: "fee_item".into(),
            params: candidate.params.clone(),
            scope: candidate.scope.clone(),
            source_key: candidate.source_key.clone(),
            confidence: candidate.confidence,
            rule_candidate_id: candidate.rule_candidate_id.clone(),
            evidence_section_id: candidate.section_id,
            evidence_quote: quote.into(),
            span_start: 0,
            span_end: quote.len() as i32,
            source_snapshot_hash: candidate.source_snapshot_hash.clone(),
            verification_method: "a14-test".into(),
            adjudication_reason: "synthetic corroborated authoritative source".into(),
            publish_admissibility: "admissible".into(),
            freshness_class: "fresh".into(),
            completeness_class: "complete".into(),
        })
        .collect::<Vec<_>>();

    sqlx::query(
        "ALTER TABLE verified.rule_instances ADD CONSTRAINT a14_truth_atomic_late_fault \
         CHECK (rule_instance_id NOT LIKE 'a14-truth-second-%')",
    )
    .execute(pool)
    .await
    .unwrap();
    for attempt in 0..2 {
        let mut tx = pool.begin().await.unwrap();
        for section in &sections {
            raw_crawl_adapter::ensure_source_in_tx(&mut tx, section)
                .await
                .unwrap();
        }
        for candidate in &candidates {
            sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication_in_tx(
                &mut tx,
                candidate,
            )
            .await
            .unwrap();
        }
        // A second extractor identity for identical evidence must reuse the
        // first FK through a SAVEPOINT; a unique violation must not poison
        // the surrounding Truth transaction or invent another verified fact.
        let mut alias = candidates[0].clone();
        alias.rule_candidate_id = format!("a14-truth-duplicate-evidence-{marker}");
        let canonical_id =
            sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication_in_tx(
                &mut tx,
                &alias,
            )
            .await
            .unwrap();
        assert_eq!(canonical_id, candidates[0].rule_candidate_id);
        assert_eq!(
            sqlx_hitl_adapter::reconcile_truth_candidate_reviews_in_tx(
                &mut tx,
                &context_key,
                &Uuid::new_v4().to_string(),
            )
            .await
            .unwrap(),
            1,
        );
        sqlx_seo_adapter::upsert_verified_rule_instance_in_tx(&mut tx, &rules[0])
            .await
            .unwrap();
        let second =
            sqlx_seo_adapter::upsert_verified_rule_instance_in_tx(&mut tx, &rules[1]).await;
        if attempt == 0 {
            assert!(second.is_err(), "late rule fault must abort the whole Truth write");
            tx.rollback().await.unwrap();
            for (query, expected) in [
                ("SELECT count(*) FROM kb.sources WHERE source_key LIKE 'https://truth-atomic-%'", 0_i64),
                ("SELECT count(*) FROM extracted.rule_candidates WHERE context_key = $1", 0_i64),
                ("SELECT count(*) FROM verified.rule_instances WHERE context_key = $1", 0_i64),
                ("SELECT count(*) FROM site.seo_hitl_tasks WHERE scope_signature = $1 AND task_type='truth_candidate_review'", 0_i64),
            ] {
                let count: i64 = if query.contains("$1") {
                    sqlx::query_scalar(query).bind(&context_key).fetch_one(pool).await.unwrap()
                } else {
                    sqlx::query_scalar(query).fetch_one(pool).await.unwrap()
                };
                assert_eq!(count, expected, "late Truth fault leaked state: {query}");
            }
            sqlx::query("ALTER TABLE verified.rule_instances DROP CONSTRAINT a14_truth_atomic_late_fault")
                .execute(pool)
                .await
                .unwrap();
        } else {
            second.unwrap();
            tx.commit().await.unwrap();
        }
    }
    let (candidate_count, verified_count, review_count): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM extracted.rule_candidates WHERE context_key=$1),
            (SELECT count(*) FROM verified.rule_instances WHERE context_key=$1),
            (SELECT count(*) FROM site.seo_hitl_tasks WHERE scope_signature=$1 AND task_type='truth_candidate_review')",
    )
    .bind(&context_key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((candidate_count, verified_count, review_count), (3, 2, 1));

    // Production Truth commits its completed step result in the SAME PG
    // transaction as canonical verified rules. A worker that loses its ACK
    // must read that original result, even after a newer run has updated the
    // same semantic slot. Exercise the actual ledger API and SQL write, not
    // an in-memory retry approximation. String is a registered typed runtime
    // payload; the activity registers its own VerifiedTruthWrite payloads.
    use infrastructure::adapters::sqlx_step_ledger_adapter as ledger;
    let run_a = Uuid::new_v4().to_string();
    let run_b = Uuid::new_v4().to_string();
    let input_a = format!("truth-run-A-input-{marker}");
    let input_b = format!("truth-run-B-input-{marker}");
    let output_a = format!("truth-run-A-original-output-{marker}");
    let output_b = format!("truth-run-B-newer-output-{marker}");
    for (run_id, input, output, amount) in [
        (&run_a, &input_a, &output_a, 35_i64),
        (&run_b, &input_b, &output_b, 55_i64),
    ] {
        sqlx::query(
            "INSERT INTO pipeline.execution_runs \
             (run_id,workflow_run_id,workflow_type,context_key) \
             VALUES ($1,$2,'seo_site_build',$3)",
        )
        .bind(Uuid::parse_str(run_id).unwrap())
        .bind(format!("truth-replay-{run_id}"))
        .bind(&context_key)
        .execute(pool)
        .await
        .unwrap();
        let (input_hash, idempotency_key) =
            ledger::derive_step_keys(run_id, "verified_truth_write", 1, input).unwrap();
        assert!(ledger::begin_step_execution(
            pool, run_id, "verified_truth_write", 1, &input_hash, &idempotency_key,
        )
        .await
        .unwrap());
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("truth-context|{context_key}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let cached: Option<String> = ledger::load_completed_step_result_in_tx(
            &mut tx, run_id, "verified_truth_write", 1, input,
        )
        .await
        .unwrap();
        assert!(cached.is_none(), "a fresh run must not inherit another run's result");
        let mut write = rules[0].clone();
        write.params = serde_json::json!({"amount": amount, "currency":"EUR"});
        sqlx_seo_adapter::upsert_verified_rule_instance_in_tx(&mut tx, &write)
            .await
            .unwrap();
        ledger::complete_step_execution_with_output_in_tx(
            &mut tx, run_id, "verified_truth_write", 1, input, output,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let (input_hash_a, idempotency_key_a) =
        ledger::derive_step_keys(&run_a, "verified_truth_write", 1, &input_a).unwrap();
    assert!(!ledger::begin_step_execution(
        pool, &run_a, "verified_truth_write", 1, &input_hash_a, &idempotency_key_a,
    )
    .await
    .unwrap());
    let mut replay_tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{context_key}"))
        .execute(&mut *replay_tx)
        .await
        .unwrap();
    let replay_a: Option<String> = ledger::load_completed_step_result_in_tx(
        &mut replay_tx, &run_a, "verified_truth_write", 1, &input_a,
    )
    .await
    .unwrap();
    assert_eq!(replay_a.as_deref(), Some(output_a.as_str()));
    replay_tx.commit().await.unwrap();
    let (current_amount, rule_count): (i64, i64) = sqlx::query_as(
        "SELECT (params->>'amount')::bigint, \
         (SELECT count(*) FROM verified.rule_instances WHERE rule_instance_id=$1) \
         FROM verified.rule_instances WHERE rule_instance_id=$1",
    )
    .bind(&rules[0].rule_instance_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((current_amount, rule_count), (55, 1), "stale run A replay overwrote run B");

    // A different hazard is an older run whose FIRST Truth activity starts
    // only after a newer run has already committed. It has no cached result,
    // so the committed-step supersession fence must reject it before any
    // candidate, canonical rule or projection Outbox mutation.
    let old_first_run = Uuid::new_v4().to_string();
    let fresh_run = Uuid::new_v4().to_string();
    let tied_run = Uuid::new_v4().to_string();
    let old_input = format!("truth-old-first-{marker}");
    let fresh_input = format!("truth-new-first-{marker}");
    let tied_input = format!("truth-tied-first-{marker}");
    for (run, input) in [
        (&old_first_run, &old_input),
        (&fresh_run, &fresh_input),
        (&tied_run, &tied_input),
    ] {
        sqlx::query(
            "INSERT INTO pipeline.execution_runs \
             (run_id,workflow_run_id,workflow_type,context_key) \
             VALUES ($1,$2,'seo_site_build',$3)",
        )
        .bind(Uuid::parse_str(run).unwrap())
        .bind(format!("truth-stale-first-{run}"))
        .bind(&context_key)
        .execute(pool)
        .await
        .unwrap();
        let (hash, key) = ledger::derive_step_keys(run, "verified_truth_write", 1, input)
            .unwrap();
        assert!(ledger::begin_step_execution(
            pool, run, "verified_truth_write", 1, &hash, &key,
        )
        .await
        .unwrap());
    }
    // Precise chronology makes CI independent of timestamp resolution.
    sqlx::query(
        "UPDATE pipeline.execution_runs SET created_at= \
         CASE WHEN run_id=$1::uuid THEN now()-interval '2 hours' \
              ELSE now()+interval '1 hour' END \
         WHERE run_id=ANY($2::uuid[])",
    )
    .bind(Uuid::parse_str(&old_first_run).unwrap())
    .bind(vec![
        Uuid::parse_str(&old_first_run).unwrap(),
        Uuid::parse_str(&fresh_run).unwrap(),
        Uuid::parse_str(&tied_run).unwrap(),
    ])
    .execute(pool)
    .await
    .unwrap();
    let mut fresh_tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{context_key}"))
        .execute(&mut *fresh_tx)
        .await
        .unwrap();
    sqlx_seo_adapter::reject_stale_truth_run_in_tx(&mut fresh_tx, &fresh_run, &context_key)
        .await
        .unwrap();
    let mut newest_rule = rules[0].clone();
    newest_rule.params = serde_json::json!({"amount":75,"currency":"EUR"});
    sqlx_seo_adapter::upsert_verified_rule_instance_in_tx(&mut fresh_tx, &newest_rule)
        .await
        .unwrap();
    let projection = seo_steps::outbox_builder::qdrant_rule_upsert(
        &fresh_run, &newest_rule.rule_instance_id, &context_key,
        &newest_rule.rule_type_key, &newest_rule.concept_key, &newest_rule.role_type,
        &newest_rule.source_key, "verified_rules_4", vec![0.1; 1024],
    );
    let event = infrastructure::adapters::sqlx_outbox_adapter::OutboxEnvelope {
        run_id: fresh_run.clone(),
        aggregate_type: projection.aggregate_type,
        aggregate_key: projection.aggregate_key,
        target_system: projection.target_system,
        event_type: projection.event_type,
        payload_type: projection.payload_type,
        schema_version: projection.schema_version,
        idempotency_key: projection.idempotency_key,
        payload_bytes: projection.payload_bytes,
    };
    assert_eq!(
        infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(
            &mut fresh_tx, &[event],
        )
        .await
        .unwrap(),
        1
    );
    let fresh_output = format!("newest-truth-output-{marker}");
    ledger::complete_step_execution_with_output_in_tx(
        &mut fresh_tx, &fresh_run, "verified_truth_write", 1, &fresh_input, &fresh_output,
    )
    .await
    .unwrap();
    fresh_tx.commit().await.unwrap();

    for (stale_run, stale_input) in [
        (&old_first_run, &old_input),
        (&tied_run, &tied_input),
    ] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("truth-context|{context_key}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let cached: Option<String> = ledger::load_completed_step_result_in_tx(
            &mut tx, stale_run, "verified_truth_write", 1, stale_input,
        )
        .await
        .unwrap();
        assert!(cached.is_none(), "the stale activity has never executed before");
        let err = sqlx_seo_adapter::reject_stale_truth_run_in_tx(
            &mut tx, stale_run, &context_key,
        )
        .await
        .unwrap_err();
        assert_eq!(err.class_str(), "conflict_violation");
        tx.rollback().await.unwrap();
    }
    let (amount, candidates_after, events_after): (i64, i64, i64) = sqlx::query_as(
        "SELECT (r.params->>'amount')::bigint, \
         (SELECT count(*) FROM extracted.rule_candidates WHERE context_key=$2), \
         (SELECT count(*) FROM system.sync_outbox WHERE run_id=$3) \
         FROM verified.rule_instances r WHERE r.rule_instance_id=$1",
    )
    .bind(&rules[0].rule_instance_id)
    .bind(&context_key)
    .bind(&fresh_run)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        (amount, candidates_after, events_after), (75, 3, 1),
        "stale first Truth activity changed the newer canonical rule or Outbox"
    );
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_postgres_qdrant_crash_replay_and_lease_fencing() {
    use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
    use infrastructure::adapters::projection_materialize_adapter::dispatch_event;
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_run_target, mark_done, mark_retry, LeaseUpdate, OutboxEnvelope,
    };
    use infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_one;
    use prost::Message;

    let pg = PostgresHarness::start().await.unwrap();
    let qdrant = QdrantHarness::start().await.unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
    ]);
    let run_id = Uuid::new_v4().to_string();
    let entity_key = format!("outbox-a14:{}", Uuid::new_v4());
    let point_id = Uuid::new_v4().to_string();
    let collection = "raw_chunks_ctx";
    let make_event = |revision: &str, vector: Vec<f32>| {
        let cmd = QdrantUpsertCommand {
            event_id: String::new(),
            collection_name: collection.to_string(),
            entity_type: "raw_section".to_string(),
            entity_key: entity_key.clone(),
            point_id: point_id.clone(),
            vector_size: vector.len() as u32,
            vector,
            payload: None,
            distance: "cosine".to_string(),
            metadata: std::collections::HashMap::from([("revision".to_string(), revision.to_string())]),
        };
        OutboxEnvelope {
            run_id: run_id.clone(),
            aggregate_type: "raw_section".to_string(),
            aggregate_key: entity_key.clone(),
            target_system: "qdrant".to_string(),
            event_type: "QdrantUpsertCommand".to_string(),
            payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
            schema_version: 1,
            idempotency_key: format!("{entity_key}:{revision}"),
            payload_bytes: cmd.encode_to_vec(),
        }
    };

    let first = make_event("revision-1", vec![0.01; 1024]);
    let first_id = outbox_emit_one(&pg.pool, &first).await.unwrap();
    let mut claimed = claim_outbox_batch_for_run_target(&pg.pool, "crashed-worker", &run_id, "qdrant", 1, 120)
        .await.unwrap();
    assert_eq!(claimed.len(), 1);
    let first_claim = claimed.pop().unwrap();
    assert_eq!(first_claim.run_id, run_id);
    assert_eq!(first_claim.event_id.to_string(), first_id);
    dispatch_event(
        &first_claim.target_system, &first_claim.event_type, &first_claim.aggregate_key,
        &first_claim.payload_type, &first_claim.payload_bytes,
    ).await.unwrap();

    // Simulate a process dying after the real Qdrant effect but before mark_done.
    // Once the lease expires, the stale worker cannot acknowledge a new owner.
    sqlx::query("UPDATE system.sync_outbox SET locked_until = now() - interval '1 second' WHERE event_id = $1")
        .bind(first_claim.event_id).execute(&pg.pool).await.unwrap();
    let mut new_claims = claim_outbox_batch_for_run_target(&pg.pool, "replacement-worker", &run_id, "qdrant", 1, 120)
        .await.unwrap();
    let new_claim = new_claims.pop().unwrap();
    assert_eq!(new_claim.event_id, first_claim.event_id);
    assert_eq!(mark_done(&pg.pool, &first_claim).await.unwrap(), LeaseUpdate::Lost);
    assert_eq!(mark_retry(&pg.pool, &first_claim, "stale", 5).await.unwrap(), LeaseUpdate::Lost);
    // Expire this claim, then use the real outbox worker on the retry.
    sqlx::query("UPDATE system.sync_outbox SET locked_until = now() - interval '1 second' WHERE event_id = $1")
        .bind(new_claim.event_id).execute(&pg.pool).await.unwrap();
    outbox_worker::worker::run_worker_loop(&pg.pool, &pg.database_url, "a14-retry", Some(1))
        .await.unwrap();

    let status: (String, i32) = sqlx::query_as(
        "SELECT status,retry_count FROM system.sync_outbox WHERE event_id=$1",
    ).bind(first_claim.event_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(status, ("done".to_string(), 0));
    assert_eq!(outbox_emit_one(&pg.pool, &first).await.unwrap(), first_id);

    let second = make_event("revision-2", vec![0.2; 1024]);
    let second_id = outbox_emit_one(&pg.pool, &second).await.unwrap();
    assert_ne!(first_id, second_id);
    outbox_worker::worker::run_worker_loop(&pg.pool, &pg.database_url, "a14-next-revision", Some(1))
        .await.unwrap();
    let total_done: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM system.sync_outbox WHERE run_id=$1 AND status='done'",
    ).bind(&run_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(total_done, 2);

    let points = infrastructure::adapters::qdrant_client_adapter::search_dense(
        &qdrant.client, collection, vec![0.2; 1024], 10, None,
    ).await.unwrap();
    let matching: Vec<_> = points.iter().filter(|p| {
        infrastructure::adapters::qdrant_client_adapter::scored_point_payload_map(p)
            .get("entity_key") == Some(&entity_key)
    }).collect();
    assert_eq!(matching.len(), 1, "replay must not create a second logical point");
    let fields = infrastructure::adapters::qdrant_client_adapter::scored_point_payload_map(matching[0]);
    assert_eq!(fields.get("revision").map(String::as_str), Some("revision-2"));
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_terminal_failure_and_dead_letter_are_atomic() {
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_run_target, mark_failed_with_dead_letter, OutboxEnvelope,
    };
    use infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_one;
    use primitives::errors::ErrorClass;

    let pg = PostgresHarness::start().await.unwrap();
    let run_id = Uuid::new_v4().to_string();
    let event = OutboxEnvelope {
        run_id: run_id.clone(),
        aggregate_type: "cms".to_string(),
        aggregate_key: Uuid::new_v4().to_string(),
        target_system: "cms".to_string(),
        event_type: "seo_page_published".to_string(),
        payload_type: "alegria.outbox.cms.v1".to_string(),
        schema_version: 1,
        idempotency_key: Uuid::new_v4().to_string(),
        payload_bytes: vec![0xff],
    };
    let event_id = outbox_emit_one(&pg.pool, &event).await.unwrap();
    sqlx::query("UPDATE system.sync_outbox SET retry_count=9 WHERE event_id=$1")
        .bind(Uuid::parse_str(&event_id).unwrap()).execute(&pg.pool).await.unwrap();
    outbox_worker::worker::run_worker_loop(&pg.pool, &pg.database_url, "a14-terminal", Some(1))
        .await.unwrap();
    let durable: (String, String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT o.status, o.run_id, d.error_class, d.run_id FROM system.sync_outbox o \
         JOIN system.dead_letter_queue d ON d.workflow_id=o.event_id::text WHERE o.event_id=$1",
    ).bind(Uuid::parse_str(&event_id).unwrap()).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(durable, ("failed".into(), run_id.clone(), "contract_violation".into(), None));

    let mut second_event = event.clone();
    second_event.aggregate_key = Uuid::new_v4().to_string();
    second_event.idempotency_key = Uuid::new_v4().to_string();
    let second_id = outbox_emit_one(&pg.pool, &second_event).await.unwrap();
    let second_claim = claim_outbox_batch_for_run_target(&pg.pool, "a14-dlq", &run_id, "cms", 1, 120)
        .await.unwrap().pop().unwrap();
    assert_eq!(second_claim.event_id.to_string(), second_id);

    // Disposable test database only: force the INSERT to fail and prove that
    // the status UPDATE rolls back too. A later attempt must still be possible.
    sqlx::query("DROP TABLE system.dead_letter_queue")
        .execute(&pg.pool).await.unwrap();
    let failure = mark_failed_with_dead_letter(&pg.pool, &second_claim, "sync_outbox_dispatch",
        ErrorClass::ContractViolation, "a14-test", "invalid payload").await;
    assert!(failure.is_err());
    let status: String = sqlx::query_scalar("SELECT status FROM system.sync_outbox WHERE event_id=$1")
        .bind(second_claim.event_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(status, "processing", "no terminal failure without durable dead letter");
}
