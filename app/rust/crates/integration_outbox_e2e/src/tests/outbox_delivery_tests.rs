/// A14: projection metadata and the corresponding Outbox intent are one PG
/// transaction; the same event can materialize more than once in real Qdrant.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_verified_rule_projection_atomic_stage_replay_and_qdrant_delete() {
    use projection_delivery::projection_materialize_adapter::{
        dispatch_leased_event, DispatchDisposition,
    };
    use infrastructure::adapters::{
        sqlx_outbox_adapter::{
            claim_outbox_batch_for_target, dispatch_verified_rule_with_fence, mark_done,
            LeaseUpdate, VerifiedRuleDispatchOutcome,
        },
        verified_rule_projection_adapter::{
            stage_rule_point_deletes_in_tx, stage_rule_upserts_in_tx, VerifiedRuleEmbeddingAdapter,
        },
    };
    use vector_qdrant::VerifiedRuleReadbackPoint;
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
    let (point_id, upsert_event_id): (String, Uuid) = {
        let point_id: String = sqlx::query_scalar(
            "SELECT point_id FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
        ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
        let upsert_event_id: Uuid = sqlx::query_scalar(
            "SELECT event_id FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'",
        ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
        (point_id, upsert_event_id)
    };
    let expected = [VerifiedRuleReadbackPoint {
        point_id, rule_instance_id: rule_id.clone(),
        context_key: context_key.clone(), concept_key: "passport".into(),
        source_key: "https://example.gov/passport".into(),
    }];
    assert!(!vector_qdrant::verify_verified_rule_readback(&expected).await.unwrap());
    let mut initial_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-rule-first", "qdrant", 1, 120, Some(upsert_event_id),
    )
    .await
    .unwrap();
    assert_eq!(initial_claim.len(), 1);
    assert_eq!(
        dispatch_leased_event(&pg.pool, &initial_claim.pop().unwrap())
            .await
            .unwrap(),
        DispatchDisposition::AcknowledgedByFence,
    );
    assert!(claim_outbox_batch_for_target(
        &pg.pool, "a14-rule-replay", "qdrant", 1, 120, Some(upsert_event_id),
    )
    .await
    .unwrap()
    .is_empty(), "acknowledged rule effect was claimable for duplicate delivery");
    assert!(vector_qdrant::verify_verified_rule_readback(&expected).await.unwrap());

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
    let (independent_point_id, independent_upsert_id): (String, Uuid) = sqlx::query_as(
        "SELECT p.point_id, o.event_id FROM kb.qdrant_points p \
         JOIN system.sync_outbox o ON o.aggregate_key=p.entity_key \
         WHERE p.entity_key=$1 AND p.collection_name='verified_rules_4' \
           AND o.target_system='qdrant' AND o.event_type='QdrantUpsertCommand'",
    )
    .bind(&second_rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    let independent_expected = [VerifiedRuleReadbackPoint {
        point_id: independent_point_id,
        rule_instance_id: second_rule_id.clone(),
        context_key: context_key.clone(),
        concept_key: "passport".into(),
        source_key: "https://example.gov/passport".into(),
    }];

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
    assert!(vector_qdrant::verify_verified_rule_readback(&expected)
        .await
        .unwrap());
    sqlx::query("ALTER TABLE system.sync_outbox DROP CONSTRAINT reject_qdrant_delete_outbox")
        .execute(&pg.pool)
        .await
        .unwrap();

    // A genuine second canonical Truth transaction can stage a new upsert
    // before the later tombstone. Delay only its Outbox delivery, never fake
    // a verified-rule event through the forbidden standalone emitter.
    let delayed_run_id = Uuid::new_v4().to_string();
    let delayed_batch = VerifiedRuleUpsertBatch {
        run_id: delayed_run_id.clone(),
        ..command.clone()
    };
    let mut delayed_tx = pg.pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut delayed_tx, &delayed_batch, embeddings.clone())
        .await
        .unwrap();
    delayed_tx.commit().await.unwrap();
    let delayed_upsert_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM system.sync_outbox \
         WHERE run_id=$1 AND aggregate_key=$2 AND event_type='QdrantUpsertCommand'",
    )
    .bind(&delayed_run_id)
    .bind(&rule_id)
    .fetch_one(&pg.pool)
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
    assert!(vector_qdrant::verify_verified_rule_readback(&expected)
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
    // Replaying the ORIGINAL run after its tombstone is not a fresh intent.
    // The writer must fail closed and roll back its registry mutation, rather
    // than silently taking the old idempotency key as permission to restore.
    let mut obsolete_replay = pg.pool.begin().await.unwrap();
    assert!(
        stage_rule_upserts_in_tx(&mut obsolete_replay, &command, embeddings.clone())
            .await
            .is_err(),
        "obsolete run replay rewrote Truth despite a newer tombstone"
    );
    obsolete_replay.rollback().await.unwrap();
    let still_deleted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points \
         WHERE entity_key=$1 AND collection_name='verified_rules_4'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(still_deleted, 0, "rejected obsolete replay mutated the canonical registry");
    let tombstone_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND event_type='QdrantDeleteCommand'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    // The causal intent is allocated under the Truth writer's per-rule PG
    // lock. In particular, the same event's idempotent replay must not create
    // a fourth revision, and an unrelated rule starts its OWN sequence.
    let (first_revision, delayed_revision, tombstone_revision, independent_revision):
        (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT projection_revision FROM system.sync_outbox WHERE event_id=$1), \
         (SELECT projection_revision FROM system.sync_outbox WHERE event_id=$2), \
         (SELECT projection_revision FROM system.sync_outbox WHERE event_id=$3), \
         (SELECT projection_revision FROM system.sync_outbox WHERE event_id=$4)",
    )
    .bind(upsert_event_id)
    .bind(delayed_upsert_id)
    .bind(tombstone_id)
    .bind(independent_upsert_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(
        (first_revision, delayed_revision, tombstone_revision, independent_revision),
        (1, 2, 3, 1),
        "replays, rollback and unrelated rule identities must not consume revisions"
    );

    // An intentionally adversarial, disposable PG fixture: make the first
    // and delayed intents share a timestamp, and make the LATEST tombstone
    // look oldest by created_at. Neither timestamps nor UUID lexical order
    // are allowed to pick which real Qdrant effect may execute.
    let disturbed = sqlx::query(
        "UPDATE system.sync_outbox \
         SET created_at = CASE \
           WHEN event_id=$3 THEN TIMESTAMPTZ '2020-01-01 00:00:00+00' \
           ELSE TIMESTAMPTZ '2030-01-01 00:00:00+00' END \
         WHERE event_id IN ($1,$2,$3)",
    )
    .bind(upsert_event_id)
    .bind(delayed_upsert_id)
    .bind(tombstone_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    assert_eq!(disturbed.rows_affected(), 3);
    let reversed_and_tied: bool = sqlx::query_scalar(
        "SELECT original.created_at = delayed.created_at \
            AND tombstone.created_at < delayed.created_at \
         FROM system.sync_outbox original \
         JOIN system.sync_outbox delayed ON delayed.event_id=$2 \
         JOIN system.sync_outbox tombstone ON tombstone.event_id=$3 \
         WHERE original.event_id=$1",
    )
    .bind(upsert_event_id)
    .bind(delayed_upsert_id)
    .bind(tombstone_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert!(reversed_and_tied, "test fixture did not invert timestamp ordering");

    // Concurrent delivery of three DISTINCT, actually claimed Outbox events.
    // The stale upsert must be ACKed as SkippedStale without invoking the
    // callback; the latest tombstone executes through the leased PG→Qdrant
    // fence. An unrelated rule's upsert executes independently at the same
    // time. If the stale callback is invoked, the injected error fails this test.
    let mut delayed_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-stale-owner", "qdrant", 1, 120, Some(delayed_upsert_id),
    )
    .await
    .unwrap();
    let mut delete_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-delete-owner", "qdrant", 1, 120, Some(tombstone_id),
    )
    .await
    .unwrap();
    let mut independent_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-independent-owner", "qdrant", 1, 120, Some(independent_upsert_id),
    )
    .await
    .unwrap();
    assert_eq!(delayed_claim.len(), 1);
    assert_eq!(delete_claim.len(), 1);
    assert_eq!(independent_claim.len(), 1);
    let expired_delayed_owner = delayed_claim.pop().unwrap();
    // A genuinely pending effect remains reclaimable after lease expiry, but
    // the former worker's PG ownership CAS must fail after a new claim.
    sqlx::query(
        "UPDATE system.sync_outbox SET locked_until=now()-interval '1 second' \
         WHERE event_id=$1",
    )
    .bind(delayed_upsert_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    let mut reclaim_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-reclaimed-stale-owner", "qdrant", 1, 120, Some(delayed_upsert_id),
    )
    .await
    .unwrap();
    assert_eq!(reclaim_claim.len(), 1, "expired pending lease was not reclaimed");
    let delayed_event = reclaim_claim.pop().unwrap();
    assert_ne!(expired_delayed_owner.lease_worker_id, delayed_event.lease_worker_id);
    assert_eq!(
        mark_done(&pg.pool, &expired_delayed_owner).await.unwrap(),
        LeaseUpdate::Lost,
        "expired worker acknowledged a newly owned Qdrant event"
    );
    let tombstone_event = delete_claim.pop().unwrap();
    let independent_event = independent_claim.pop().unwrap();
    let (stale_dispatch, tombstone_dispatch, independent_dispatch) = tokio::join!(
        dispatch_verified_rule_with_fence(&pg.pool, &delayed_event, || async {
            Err::<(), anyhow::Error>(anyhow::anyhow!("stale upsert reached Qdrant effect"))
        }),
        dispatch_leased_event(&pg.pool, &tombstone_event),
        dispatch_leased_event(&pg.pool, &independent_event),
    );
    assert_eq!(stale_dispatch.unwrap(), VerifiedRuleDispatchOutcome::SkippedStale);
    assert_eq!(tombstone_dispatch.unwrap(), DispatchDisposition::AcknowledgedByFence);
    assert_eq!(independent_dispatch.unwrap(), DispatchDisposition::AcknowledgedByFence);
    for event_id in [delayed_upsert_id, tombstone_id, independent_upsert_id] {
        assert!(claim_outbox_batch_for_target(
            &pg.pool, "a14-repeat-owner", "qdrant", 1, 120, Some(event_id),
        )
        .await
        .unwrap()
        .is_empty(), "ACKed rule effect was claimable again");
    }
    assert!(
        !vector_qdrant::verify_verified_rule_readback(&expected)
            .await
            .unwrap(),
        "concurrent delayed upsert resurrected a rule after its tombstone"
    );
    assert!(
        vector_qdrant::verify_verified_rule_readback(&independent_expected)
            .await
            .unwrap(),
        "unrelated rule was blocked by the deleted rule's causal revision"
    );

    // A later canonical re-verification is allowed to intentionally restore
    // the fact. Giving it the EARLIEST timestamp tests the reverse transition:
    // a newer upsert must supersede the tombstone by revision alone.
    let restoration_run_id = Uuid::new_v4().to_string();
    let restoration_batch = VerifiedRuleUpsertBatch {
        run_id: restoration_run_id.clone(),
        ..command
    };
    let mut restoration_tx = pg.pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut restoration_tx, &restoration_batch, embeddings)
        .await
        .unwrap();
    restoration_tx.commit().await.unwrap();
    let (restoration_id, restoration_revision): (Uuid, i64) = sqlx::query_as(
        "SELECT event_id, projection_revision FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND run_id=$2 AND event_type='QdrantUpsertCommand'",
    )
    .bind(&rule_id)
    .bind(&restoration_run_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(restoration_revision, 4, "intent resurrection lost per-rule causality");
    sqlx::query(
        "UPDATE system.sync_outbox SET created_at=TIMESTAMPTZ '2010-01-01 00:00:00+00' \
         WHERE event_id=$1",
    )
    .bind(restoration_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    let mut restoration_claim = claim_outbox_batch_for_target(
        &pg.pool, "a14-restoration-owner", "qdrant", 1, 120, Some(restoration_id),
    )
    .await
    .unwrap();
    assert_eq!(restoration_claim.len(), 1);
    assert_eq!(
        dispatch_leased_event(&pg.pool, &restoration_claim.pop().unwrap())
            .await
            .unwrap(),
        DispatchDisposition::AcknowledgedByFence,
    );
    assert!(
        vector_qdrant::verify_verified_rule_readback(&expected).await.unwrap(),
        "later canonical rule revision was suppressed by an older tombstone"
    );
}

/// A14/P0: the real Truth SQL writers compose into one transaction, including
/// source registration, canonical candidates, HITL review and rule instances.
/// Failure at the SECOND verified rule cannot expose the first verified fact.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_truth_write_multi_candidate_late_failure_rolls_back_and_retries() {
    use infrastructure::adapters::{raw_crawl_adapter, sqlx_seo_adapter};
    use postgres_store::sqlx_hitl_adapter;
    use infrastructure::adapters::verified_rule_projection_adapter::stage_rule_upserts_in_tx;
    use seo_ports::knowledge::{
        VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch,
    };

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
    // This is a genuine projected Truth write, not a generic Outbox event.
    // The PostgreSQL owner must reserve the revision and update the registry
    // alongside the command before the completed-step journal can commit.
    stage_rule_upserts_in_tx(
        &mut fresh_tx,
        &VerifiedRuleUpsertBatch {
            run_id: fresh_run.clone(),
            context_key: context_key.clone(),
            upserts: vec![VerifiedRuleProjectionCandidate {
                rule_instance_id: newest_rule.rule_instance_id.clone(),
                role_type: newest_rule.role_type.clone(),
                concept_key: newest_rule.concept_key.clone(),
                source_key: newest_rule.source_key.clone(),
                evidence_quote: newest_rule.evidence_quote.clone(),
            }],
        },
        VerifiedRuleEmbeddingBatch {
            model: "fixture".into(),
            vectors: vec![vec![0.1; 1024]],
        },
    ).await.unwrap();
    let fresh_output = format!("newest-truth-output-{marker}");
    ledger::complete_step_execution_with_output_in_tx(
        &mut fresh_tx, &fresh_run, "verified_truth_write", 1, &fresh_input, &fresh_output,
    )
    .await
    .unwrap();
    fresh_tx.commit().await.unwrap();

    let (staged_revision, staged_this_tx, registry_points): (i64, bool, i64) = sqlx::query_as(
        "SELECT o.projection_revision, o.staged_xid IS NOT NULL,
                (SELECT count(*) FROM kb.qdrant_points p
                 WHERE p.collection_name='verified_rules_4' AND p.entity_type='rule_instance'
                   AND p.entity_key=o.aggregate_key)
         FROM system.sync_outbox o
         WHERE o.run_id=$1 AND o.aggregate_key=$2 AND o.target_system='qdrant'",
    )
    .bind(&fresh_run)
    .bind(&newest_rule.rule_instance_id)
    .fetch_one(pool).await.unwrap();
    assert_eq!((staged_revision, staged_this_tx, registry_points), (1, true, 1),
        "newest Truth must stage a causal, server-stamped Qdrant intent and registry point");

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
async fn outbox_a14_target_scoped_claim_never_leases_another_provider_event() {
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_target, mark_done, LeaseUpdate, OutboxEnvelope,
    };
    use infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_one;

    let pg = PostgresHarness::start().await.unwrap();
    let run_id = Uuid::new_v4().to_string();
    let make_event = |target_system: &str, event_type: &str| OutboxEnvelope {
        run_id: run_id.clone(),
        aggregate_type: "claim-target-test".into(),
        aggregate_key: Uuid::new_v4().to_string(),
        target_system: target_system.to_owned(),
        event_type: event_type.to_owned(),
        payload_type: "a14.target-lease-fixture.v1".into(),
        schema_version: 1,
        idempotency_key: Uuid::new_v4().to_string(),
        payload_bytes: vec![0x01],
    };
    let graph_id = outbox_emit_one(&pg.pool, &make_event("neo4j", "RuleInstanceUpserted"))
        .await
        .unwrap();
    let vector_id = outbox_emit_one(&pg.pool, &make_event("qdrant", "QdrantUpsertCommand"))
        .await
        .unwrap();

    // Competing graph owners and a vector owner all lease concurrently. Only
    // one graph worker may acquire the graph row, while the vector worker must
    // still be able to acquire its own row without waiting for graph delivery.
    let (graph_claims_a, graph_claims_b, vector_claims) = tokio::join!(
        claim_outbox_batch_for_target(&pg.pool, "graph-owner-a", "neo4j", 16, 120, None),
        claim_outbox_batch_for_target(&pg.pool, "graph-owner-b", "neo4j", 16, 120, None),
        claim_outbox_batch_for_target(&pg.pool, "vector-only-worker", "qdrant", 16, 120, None),
    );
    let mut graph_claims = graph_claims_a.unwrap();
    graph_claims.extend(graph_claims_b.unwrap());
    assert_eq!(graph_claims.len(), 1, "two graph owners both claimed the same event");
    let graph = graph_claims.pop().unwrap();
    assert_eq!(graph.event_id.to_string(), graph_id);
    assert_eq!(graph.target_system, "neo4j");
    assert!(matches!(graph.lease_worker_id.as_str(), "graph-owner-a" | "graph-owner-b"));

    let other_status: (String, Option<String>) = sqlx::query_as(
        "SELECT status, worker_id FROM system.sync_outbox WHERE event_id=$1",
    )
    .bind(Uuid::parse_str(&vector_id).unwrap())
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(
        other_status,
        ("processing".into(), Some("vector-only-worker".into())),
        "graph owner stole a concurrently claimed Qdrant lease"
    );

    let mut vector_claims = vector_claims.unwrap();
    assert_eq!(vector_claims.len(), 1);
    let vector = vector_claims.pop().unwrap();
    assert_eq!(vector.event_id.to_string(), vector_id);
    assert_eq!(vector.target_system, "qdrant");
    assert_eq!(vector.lease_worker_id, "vector-only-worker");
    assert!(claim_outbox_batch_for_target(&pg.pool, "cms-only-worker", "cms", 16, 120, None)
        .await
        .unwrap()
        .is_empty());

    // Release both fixture leases through the real PostgreSQL ownership CAS.
    assert_eq!(mark_done(&pg.pool, &graph).await.unwrap(), LeaseUpdate::Applied);
    assert_eq!(mark_done(&pg.pool, &vector).await.unwrap(), LeaseUpdate::Applied);
}

/// A14: once a real PostgreSQL lease crosses the Qdrant uncertainty boundary,
/// neither lease expiry nor a newer revision can issue an unproven effect.
/// The durable `failed` marker is committed *before* invoking the provider.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_verified_rule_uncertain_effect_quarantines_claim_and_new_revision() {
    use infrastructure::adapters::verified_rule_projection_adapter::{
        stage_rule_point_deletes_in_tx, stage_rule_upserts_in_tx,
    };
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_run_target, dispatch_verified_rule_with_fence, OutboxEnvelope,
    };
    use infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx;
    use infrastructure::adapters::sqlx_seo_adapter::projection_backlog_counts;
    use infrastructure::adapters::sqlx_reconcile_adapter::{
        reconcile_target_system, ReconcileOptionsRecord,
    };
    use seo_ports::knowledge::{
        VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch,
    };
    use tokio::sync::oneshot;

    let pg = PostgresHarness::start().await.unwrap();
    let run_id = Uuid::new_v4().to_string();
    let rule_id = format!("fence-a14-{}", Uuid::new_v4().simple());
    let canonical_batch = VerifiedRuleUpsertBatch {
        run_id: run_id.clone(),
        context_key: format!("fence-context-{}", Uuid::new_v4().simple()),
        upserts: vec![VerifiedRuleProjectionCandidate {
            rule_instance_id: rule_id.clone(),
            role_type: "document_required".into(),
            concept_key: "passport".into(),
            source_key: "https://example.gov/passport".into(),
            evidence_quote: "A passport is required".into(),
        }],
    };
    let embeddings = VerifiedRuleEmbeddingBatch {
        model: "fixture-1024".into(),
        vectors: vec![vec![0.1; 1024]],
    };
    let mut original_tx = pg.pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut original_tx, &canonical_batch, embeddings.clone())
        .await
        .unwrap();
    original_tx.commit().await.unwrap();
    let (first_id, first_payload): (Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT event_id, payload_bytes FROM system.sync_outbox \
         WHERE run_id=$1 AND aggregate_key=$2 AND event_type='QdrantUpsertCommand'",
    )
    .bind(&run_id)
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    let mut first_claim = claim_outbox_batch_for_run_target(
        &pg.pool, "first-fenced-owner", &run_id, "qdrant", 1, 120,
    )
    .await
    .unwrap();
    assert_eq!(first_claim.len(), 1);
    let first_claim = first_claim.pop().unwrap();
    assert_eq!(first_claim.event_id, first_id);

    // Hold the provider callback open. Its entrance proves the quarantine
    // marker has been committed even though the remote outcome is unknown.
    let (entered_tx, entered_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let effect_pool = pg.pool.clone();
    let effect_task = tokio::spawn(async move {
        dispatch_verified_rule_with_fence(&effect_pool, &first_claim, move || async move {
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Err::<(), anyhow::Error>(anyhow::anyhow!("injected unknown provider outcome"))
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .expect("provider callback did not start")
        .expect("provider callback stopped unexpectedly");

    let (status, last_error): (String, Option<String>) = sqlx::query_as(
        "SELECT status, last_error FROM system.sync_outbox WHERE event_id=$1",
    )
    .bind(first_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(status, "failed", "effect started before durable quarantine");
    assert_eq!(last_error.as_deref(), Some("projection_outcome_uncertain:v1"));
    let quarantined = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((quarantined.pending_events, quarantined.failed_events), (0, 1),
        "Temporal drain must not mistake durable unknown external outcome for delivery success");

    // An expired lease does not override an unknown external outcome. Two
    // simultaneous replacement workers must both observe zero eligible rows.
    sqlx::query(
        "UPDATE system.sync_outbox SET locked_until=now()-interval '1 second' WHERE event_id=$1",
    )
    .bind(first_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    let (second_owner, third_owner) = tokio::join!(
        claim_outbox_batch_for_run_target(&pg.pool, "second-fenced-owner", &run_id, "qdrant", 1, 120),
        claim_outbox_batch_for_run_target(&pg.pool, "third-fenced-owner", &run_id, "qdrant", 1, 120),
    );
    assert!(second_owner.unwrap().is_empty());
    assert!(third_owner.unwrap().is_empty());

    // Truth's canonical tombstone attempts the same per-point advisory lock.
    // While the remote RPC is unresolved, it must wait, not commit a newer
    // registry state behind the older in-flight provider mutation.
    let truth_pool = pg.pool.clone();
    let truth_run_id = run_id.clone();
    let truth_rule_id = rule_id.clone();
    let (truth_started_tx, truth_started_rx) = oneshot::channel::<()>();
    let mut competing_truth = tokio::spawn(async move {
        let mut truth_tx = truth_pool.begin().await.unwrap();
        truth_started_tx.send(()).unwrap();
        let result = stage_rule_point_deletes_in_tx(
            &mut truth_tx,
            &truth_run_id,
            "uncertain-rule-context",
            std::slice::from_ref(&truth_rule_id),
        )
        .await;
        truth_tx.rollback().await.unwrap();
        result
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), truth_started_rx)
        .await
        .expect("competing Truth transaction did not start")
        .expect("competing Truth transaction stopped unexpectedly");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut competing_truth)
            .await
            .is_err(),
        "Truth mutation completed while previous Qdrant callback was in flight"
    );

    release_tx.send(()).unwrap();
    let unknown_outcome = effect_task.await.unwrap();
    assert!(unknown_outcome.is_err(), "unknown provider outcome was acknowledged");
    assert!(
        competing_truth.await.unwrap().is_err(),
        "Truth mutation ignored uncertainty after the Qdrant callback failed"
    );

    // Canonical Truth transaction writers must share the same per-rule fence.
    // They may not stage a tombstone after a provider effect whose outcome is
    // unknown: rolling back must leave both registry and intent unchanged.
    let mut forbidden_delete = pg.pool.begin().await.unwrap();
    let delete_error = stage_rule_point_deletes_in_tx(
        &mut forbidden_delete,
        &run_id,
        "uncertain-rule-context",
        std::slice::from_ref(&rule_id),
    )
    .await;
    assert!(delete_error.is_err(), "Truth tombstone bypassed unresolved Qdrant effect");
    forbidden_delete.rollback().await.unwrap();
    let retained_points: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE collection_name='verified_rules_4' AND entity_key=$1",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(retained_points, 1, "failed Truth mutation changed canonical point registry");
    let delete_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantDeleteCommand'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(delete_events, 0, "failed Truth mutation enqueued a new effect");

    // Re-verification must also fail closed: no new canonical projection or
    // Outbox revision may be committed until an operator reconciles the
    // unknown old RPC. Both canonical staging and generic in-tx emission
    // use the same point lock and uncertainty gate.
    let newer_batch = VerifiedRuleUpsertBatch {
        run_id: Uuid::new_v4().to_string(),
        ..canonical_batch
    };
    let mut forbidden_upsert = pg.pool.begin().await.unwrap();
    assert!(stage_rule_upserts_in_tx(&mut forbidden_upsert, &newer_batch, embeddings)
        .await
        .is_err(), "Truth re-verification bypassed unresolved Qdrant effect");
    forbidden_upsert.rollback().await.unwrap();

    let mut generic_tx = pg.pool.begin().await.unwrap();
    let generic_reenqueue = OutboxEnvelope {
        run_id: newer_batch.run_id,
        aggregate_type: "rule_instance".into(),
        aggregate_key: rule_id.clone(),
        target_system: "qdrant".into(),
        event_type: "QdrantUpsertCommand".into(),
        payload_type: "alegria.outbox.qdrant_upsert_command.v1".into(),
        schema_version: 1,
        idempotency_key: format!("a14-guarded-reenqueue:{}", Uuid::new_v4()),
        payload_bytes: first_payload,
    };
    assert!(outbox_emit_many_in_tx(&mut generic_tx, &[generic_reenqueue])
        .await
        .is_err(), "generic writer re-enqueued a quarantined verified rule");
    generic_tx.rollback().await.unwrap();

    let mut reconcile_options = ReconcileOptionsRecord {
        max_retry_count: 10,
        batch_limit: 100,
        dry_run: true,
        requeue_base_delay_sec: 0,
        requeue_jitter_sec: 0,
    };
    let dry_run = reconcile_target_system(&pg.pool, "qdrant", &reconcile_options)
        .await
        .unwrap();
    assert_eq!(dry_run.failed_candidates, 0, "dry-run offered unknown effect for replay");
    reconcile_options.dry_run = false;
    let mutating_reconcile = reconcile_target_system(&pg.pool, "qdrant", &reconcile_options)
        .await
        .unwrap();
    assert_eq!(mutating_reconcile.requeued_failed, 0, "reconciler released unknown effect");

    let (final_status, final_error): (String, Option<String>) = sqlx::query_as(
        "SELECT status, last_error FROM system.sync_outbox WHERE event_id=$1",
    )
    .bind(first_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(final_status, "failed");
    assert_eq!(final_error.as_deref(), Some("projection_outcome_uncertain:v1"));
    let blocked = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((blocked.pending_events, blocked.failed_events), (0, 1),
        "reconciliation must not silently clear uncertain effects or unblock Temporal");
    let number_of_outbox_intents: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox WHERE aggregate_type='rule_instance' AND aggregate_key=$1 AND target_system='qdrant'",
    )
    .bind(&rule_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(number_of_outbox_intents, 1, "Truth bypass created a later Outbox intent");
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_postgres_qdrant_crash_replay_and_lease_fencing() {
    use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
    use projection_delivery::projection_materialize_adapter::dispatch_event;
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_run_target, mark_done, mark_retry, LeaseUpdate, OutboxEnvelope,
    };
    use infrastructure::adapters::sqlx_runtime_outbox_adapter::outbox_emit_one;
    use infrastructure::adapters::sqlx_seo_adapter::projection_backlog_counts;
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
    // No Outbox process exists yet. A Temporal status/drain observer must
    // observe the unfinished Qdrant intent rather than report success.
    let stalled = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((stalled.pending_events, stalled.failed_events), (1, 0));

    // A different provider's command in the SAME run must remain untouched
    // by a target-scoped Qdrant worker, even after the worker is restarted.
    let unrelated = OutboxEnvelope {
        run_id: run_id.clone(),
        aggregate_type: "cms".into(),
        aggregate_key: Uuid::new_v4().to_string(),
        target_system: "cms".into(),
        event_type: "seo_page_published".into(),
        payload_type: "alegria.outbox.cms.v1".into(),
        schema_version: 1,
        idempotency_key: Uuid::new_v4().to_string(),
        payload_bytes: vec![0xff],
    };
    let unrelated_id = outbox_emit_one(&pg.pool, &unrelated).await.unwrap();
    let mut claimed = claim_outbox_batch_for_run_target(&pg.pool, "crashed-worker", &run_id, "qdrant", 1, 120)
        .await.unwrap();
    assert_eq!(claimed.len(), 1);
    let first_claim = claimed.pop().unwrap();
    assert_eq!(first_claim.run_id, run_id);
    assert_eq!(first_claim.event_id.to_string(), first_id);
    let in_flight = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((in_flight.pending_events, in_flight.failed_events), (1, 0),
        "a claimed processing lease is NOT a delivered projection");
    dispatch_event(
        &first_claim.target_system, &first_claim.event_type, &first_claim.aggregate_key,
        &first_claim.payload_type, &first_claim.payload_bytes,
    ).await.unwrap();

    // Simulate a process dying after the real Qdrant effect but before mark_done.
    // Once the lease expires, the stale worker cannot acknowledge a new owner.
    sqlx::query("UPDATE system.sync_outbox SET locked_until = now() - interval '1 second' WHERE event_id = $1")
        .bind(first_claim.event_id).execute(&pg.pool).await.unwrap();
    // Both replacements race for the *expired* lease. The real PostgreSQL
    // SKIP LOCKED transition must grant exactly one fresh lease generation.
    let (replacement_a, replacement_b) = tokio::join!(
        claim_outbox_batch_for_run_target(&pg.pool, "replacement-worker-a", &run_id, "qdrant", 1, 120),
        claim_outbox_batch_for_run_target(&pg.pool, "replacement-worker-b", &run_id, "qdrant", 1, 120),
    );
    let mut new_claims = replacement_a.unwrap();
    new_claims.extend(replacement_b.unwrap());
    assert_eq!(new_claims.len(), 1, "expired lease had two concurrent owners");
    let new_claim = new_claims.pop().unwrap();
    assert_eq!(new_claim.event_id, first_claim.event_id);
    assert_eq!(mark_done(&pg.pool, &first_claim).await.unwrap(), LeaseUpdate::Lost);
    assert_eq!(mark_retry(&pg.pool, &first_claim, "stale", 5).await.unwrap(), LeaseUpdate::Lost);
    // Expire this claim. Until an actual standalone worker is started, a
    // pending event is not a completed projection, even after an external
    // effect may already exist because the previous process crashed.
    sqlx::query("UPDATE system.sync_outbox SET locked_until = now() - interval '1 second' WHERE event_id = $1")
        .bind(new_claim.event_id).execute(&pg.pool).await.unwrap();
    let before_restart = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((before_restart.pending_events, before_restart.failed_events), (1, 0));

    // Execute the real separately compiled Outbox binary, never a Temporal
    // Activity or an in-process call to the delivery function. The worker
    // handles the retry after the lease generation changes.
    let worker_env = BTreeMap::from([
        ("DATABASE_URL".to_string(), pg.database_url.clone()),
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
        ("OUTBOX_TARGET_SYSTEM".to_string(), "qdrant".to_string()),
        ("OUTBOX_MAX_CYCLES".to_string(), "1".to_string()),
    ]);
    let mut restarted_worker = spawn_outbox_worker(&worker_env);
    let exit = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(status) = restarted_worker.child.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("standalone Qdrant Outbox worker timed out after crash recovery");
    assert!(exit.success(), "standalone Qdrant Outbox worker crashed: {exit}");

    let status: (String, i32) = sqlx::query_as(
        "SELECT status,retry_count FROM system.sync_outbox WHERE event_id=$1",
    ).bind(first_claim.event_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(status, ("done".to_string(), 0));
    let completed = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((completed.pending_events, completed.failed_events), (0, 0));
    let cms_backlog = projection_backlog_counts(&pg.pool, &run_id, "cms").await.unwrap();
    assert_eq!((cms_backlog.pending_events, cms_backlog.failed_events), (1, 0));
    let cms_status: String = sqlx::query_scalar(
        "SELECT status FROM system.sync_outbox WHERE event_id=$1",
    ).bind(Uuid::parse_str(&unrelated_id).unwrap()).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(cms_status, "pending", "Qdrant worker claimed an unrelated CMS event");
    assert_eq!(outbox_emit_one(&pg.pool, &first).await.unwrap(), first_id);

    let second = make_event("revision-2", vec![0.2; 1024]);
    let second_id = outbox_emit_one(&pg.pool, &second).await.unwrap();
    assert_ne!(first_id, second_id);
    let before_second_process = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((before_second_process.pending_events, before_second_process.failed_events), (1, 0));
    let mut next_worker = spawn_outbox_worker(&worker_env);
    let exit = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(status) = next_worker.child.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("standalone Qdrant Outbox worker timed out after next revision");
    assert!(exit.success(), "standalone Qdrant Outbox worker crashed: {exit}");
    let total_done: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM system.sync_outbox WHERE run_id=$1 AND status='done'",
    ).bind(&run_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(total_done, 2);
    let final_backlog = projection_backlog_counts(&pg.pool, &run_id, "qdrant").await.unwrap();
    assert_eq!((final_backlog.pending_events, final_backlog.failed_events), (0, 0));
    let cms_after_second_worker = projection_backlog_counts(&pg.pool, &run_id, "cms").await.unwrap();
    assert_eq!((cms_after_second_worker.pending_events, cms_after_second_worker.failed_events), (1, 0));

    let points = vector_qdrant::search_dense(
        &qdrant.client, collection, vec![0.2; 1024], 10, None,
    ).await.unwrap();
    let matching: Vec<_> = points.iter().filter(|p| {
        vector_qdrant::scored_point_payload_map(p)
            .get("entity_key") == Some(&entity_key)
    }).collect();
    assert_eq!(matching.len(), 1, "replay must not create a second logical point");
    let fields = vector_qdrant::scored_point_payload_map(matching[0]);
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
    use infrastructure::adapters::sqlx_seo_adapter::projection_backlog_counts;
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
    let dead_lettered = projection_backlog_counts(&pg.pool, &run_id, "cms").await.unwrap();
    assert_eq!((dead_lettered.pending_events, dead_lettered.failed_events), (0, 1),
        "a terminal provider failure must block its run/target projection gate");

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
    let after_rollback = projection_backlog_counts(&pg.pool, &run_id, "cms").await.unwrap();
    assert_eq!((after_rollback.pending_events, after_rollback.failed_events), (1, 1),
        "rolled-back terminal transition remains in-flight; the original dead letter stays failed");
}

/// PostgreSQL itself rejects a verified_rules_4 registry change unless the
/// final projection command was reserved in the SAME transaction. Requiring
/// only an historical event ID would allow a direct SQL writer to reuse it.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_pg_commit_fence_rejects_direct_registry_and_truth_bypass() {
    use infrastructure::adapters::verified_rule_projection_adapter::{
        stage_rule_point_deletes_in_tx, stage_rule_upserts_in_tx,
    };
    use seo_ports::knowledge::{
        VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch,
    };

    fn assert_commit_fence(error: &sqlx::Error) {
        let code = error.as_database_error()
            .and_then(|db| db.code())
            .map(|code| code.to_string());
        assert_eq!(code.as_deref(), Some("23514"), "unexpected PG failure: {error}");
    }

    let pg = PostgresHarness::start().await.unwrap();
    let rule_id = format!("commit-fence-{}", Uuid::new_v4().simple());
    let point_id = primitives::qdrant_point_id::qdrant_point_id_v1(
        "verified_rules_4", "rule_instance", &rule_id,
    );
    let direct_insert = sqlx::query(
        "INSERT INTO kb.qdrant_points
         (point_id,entity_type,entity_key,collection_name,embedding_model,embedding_version)
         VALUES ($1,'rule_instance',$2,'verified_rules_4','fixture','v1')",
    )
    .bind(&point_id).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&direct_insert);

    let context_key = format!("commit-context-{}", Uuid::new_v4().simple());
    let country_code = format!("C{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query(
        "INSERT INTO kb.visa_contexts (context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,$2,'tourist','BY')",
    ).bind(&context_key).bind(&country_code).execute(&pg.pool).await.unwrap();
    let batch = VerifiedRuleUpsertBatch {
        run_id: Uuid::new_v4().to_string(),
        context_key: context_key.clone(),
        upserts: vec![VerifiedRuleProjectionCandidate {
            rule_instance_id: rule_id.clone(),
            role_type: "document_required".into(),
            concept_key: "passport".into(),
            source_key: "https://fixture.example.gov/passport".into(),
            evidence_quote: "A passport is required".into(),
        }],
    };
    let embeddings = VerifiedRuleEmbeddingBatch {
        model: "fixture".into(),
        vectors: vec![vec![0.1; 1024]],
    };
    let mut tx = pg.pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut tx, &batch, embeddings).await.unwrap();
    sqlx::query(
        "INSERT INTO verified.rule_instances
         (rule_instance_id,context_key,rule_type_key,concept_key,role_type,params,status)
         VALUES ($1,$2,'passport','passport','document_required','{}','verified')",
    ).bind(&rule_id).bind(&context_key).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let evidence: (i64, bool) = sqlx::query_as(
        "SELECT projection_revision, staged_xid IS NOT NULL FROM system.sync_outbox
         WHERE aggregate_key=$1 AND target_system='qdrant' AND aggregate_type='rule_instance'",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(evidence, (1, true));

    let forged_rebind = sqlx::query(
        "UPDATE system.sync_outbox SET staged_xid=pg_current_xact_id()
         WHERE aggregate_key=$1 AND target_system='qdrant' AND aggregate_type='rule_instance'",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&forged_rebind);

    let forbidden_receipt_delete = sqlx::query(
        "DELETE FROM system.sync_outbox
         WHERE aggregate_key=$1 AND target_system='qdrant' AND aggregate_type='rule_instance'",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&forbidden_receipt_delete);
    let forbidden_head_delete = sqlx::query(
        "DELETE FROM system.verified_rule_projection_heads WHERE aggregate_key=$1",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&forbidden_head_delete);

    let direct_update = sqlx::query(
        "UPDATE verified.rule_instances SET status='disputed' WHERE rule_instance_id=$1",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&direct_update);
    let direct_registry_change = sqlx::query(
        "UPDATE kb.qdrant_points SET embedding_version='unfenced' WHERE entity_key=$1
         AND collection_name='verified_rules_4'",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&direct_registry_change);
    let direct_delete = sqlx::query(
        "DELETE FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
    ).bind(&rule_id).execute(&pg.pool).await.unwrap_err();
    assert_commit_fence(&direct_delete);

    let mut tx = pg.pool.begin().await.unwrap();
    stage_rule_point_deletes_in_tx(&mut tx, &batch.run_id, &context_key, &[rule_id.clone()])
        .await.unwrap();
    tx.commit().await.unwrap();
    let latest_revision: i64 = sqlx::query_scalar(
        "SELECT current_revision FROM system.verified_rule_projection_heads WHERE aggregate_key=$1",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(latest_revision, 2, "deletion must reserve a new causal intent");
}

/// Constraint triggers must validate the final committed state, not a
/// savepoint's transient event or a committed receipt from an older run.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_pg_commit_fence_savepoint_rollback_never_reuses_intent() {
    use infrastructure::adapters::verified_rule_projection_adapter::stage_rule_upserts_in_tx;
    use seo_ports::knowledge::{
        VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch,
    };

    let pg = PostgresHarness::start().await.unwrap();
    let rule_id = format!("commit-savepoint-{}", Uuid::new_v4().simple());
    let batch = VerifiedRuleUpsertBatch {
        run_id: Uuid::new_v4().to_string(),
        context_key: format!("commit-context-{}", Uuid::new_v4().simple()),
        upserts: vec![VerifiedRuleProjectionCandidate {
            rule_instance_id: rule_id.clone(),
            role_type: "document_required".into(),
            concept_key: "passport".into(),
            source_key: "https://fixture.example.gov/passport".into(),
            evidence_quote: "A passport is required".into(),
        }],
    };
    let mut tx = pg.pool.begin().await.unwrap();
    sqlx::query("SAVEPOINT stage_intent").execute(&mut *tx).await.unwrap();
    stage_rule_upserts_in_tx(&mut tx, &batch, VerifiedRuleEmbeddingBatch {
        model: "fixture".into(), vectors: vec![vec![0.1; 1024]],
    }).await.unwrap();
    sqlx::query("ROLLBACK TO SAVEPOINT stage_intent").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO kb.qdrant_points
         (point_id,entity_type,entity_key,collection_name,embedding_model,embedding_version)
         VALUES ($1,'rule_instance',$2,'verified_rules_4','fixture','v1')",
    ).bind(Uuid::new_v4().to_string()).bind(&rule_id)
        .execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    let error_code = error.as_database_error().and_then(|db| db.code());
    assert_eq!(error_code.as_deref(), Some("23514"),
        "rolled-back intent must not authorize registry writes: {error}");
    let lingering: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb.qdrant_points WHERE entity_key=$1 AND collection_name='verified_rules_4'",
    ).bind(&rule_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(lingering, 0);
}
