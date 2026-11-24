/// A real disposable PostgreSQL upgrade from the revision-only Outbox schema.
///
/// The harness normally bootstraps the latest schema. Rewind only the new
/// same-transaction fence DDL, retain the earlier causal-revision model,
/// seed an authentic historical row, then apply the actual migration bytes.
/// This proves NOT VALID quarantine and migration idempotency without
/// fabricating a stage xid or touching an external provider.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn outbox_a14_pg_transaction_fence_upgrade_quarantines_existing_history() {
    use infrastructure::adapters::sqlx_outbox_adapter::{
        claim_outbox_batch_for_target, lock_and_ensure_verified_rules_writable_in_tx,
        VerifiedRuleTruthFenceError,
    };
    use infrastructure::adapters::verified_rule_projection_adapter::stage_rule_upserts_in_tx;
    use seo_ports::knowledge::{
        VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionCandidate, VerifiedRuleUpsertBatch,
    };

    const NEW_MIGRATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../db/migrations/20261124_0002_verified_rule_transaction_fence.sql"
    ));

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;

    // Disposable harness only. This exactly removes the new DDL capabilities
    // that did not exist when 20261124_0001 was the last migration.
    sqlx::raw_sql(
        "DROP TRIGGER IF EXISTS trg_verified_rule_outbox_xid ON system.sync_outbox;
         DROP TRIGGER IF EXISTS trg_verified_rule_outbox_delete ON system.sync_outbox;
         DROP TRIGGER IF EXISTS trg_verified_rule_revision_head_delete
             ON system.verified_rule_projection_heads;
         DROP TRIGGER IF EXISTS trg_verified_rule_registry_commit ON kb.qdrant_points;
         DROP TRIGGER IF EXISTS trg_verified_rule_truth_commit ON verified.rule_instances;
         ALTER TABLE system.sync_outbox
             DROP CONSTRAINT IF EXISTS sync_outbox_verified_rule_xid_check;
         ALTER TABLE kb.qdrant_points
             DROP CONSTRAINT IF EXISTS qdrant_points_verified_rule_entity_type_check;
         CREATE OR REPLACE FUNCTION system.guard_verified_rule_outbox_identity()
             RETURNS trigger AS $$
         BEGIN
             IF ROW(OLD.run_id,OLD.aggregate_type,OLD.aggregate_key,OLD.target_system,
                    OLD.event_type,OLD.payload_type,OLD.schema_version,OLD.idempotency_key,
                    OLD.payload_bytes,OLD.payload_hash,OLD.projection_revision)
                IS DISTINCT FROM
                ROW(NEW.run_id,NEW.aggregate_type,NEW.aggregate_key,NEW.target_system,
                    NEW.event_type,NEW.payload_type,NEW.schema_version,NEW.idempotency_key,
                    NEW.payload_bytes,NEW.payload_hash,NEW.projection_revision) THEN
                 RAISE EXCEPTION 'verified-rule Outbox identity and revision are immutable'
                     USING ERRCODE='23514';
             END IF;
             RETURN NEW;
         END;
         $$ LANGUAGE plpgsql;
         ALTER TABLE system.sync_outbox DROP COLUMN IF EXISTS staged_xid;",
    ).execute(pool).await.unwrap();

    let historical_rule = format!("historical-projected-{}", Uuid::new_v4().simple());
    let historical_event = Uuid::new_v4();
    let historical_point = primitives::qdrant_point_id::qdrant_point_id_v1(
        "verified_rules_4", "rule_instance", &historical_rule,
    );
    let mut old_tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO system.verified_rule_projection_heads (aggregate_key,current_revision)
         VALUES ($1,1)",
    ).bind(&historical_rule).execute(&mut *old_tx).await.unwrap();
    sqlx::query(
        "INSERT INTO kb.qdrant_points
         (point_id,entity_type,entity_key,collection_name,embedding_model,embedding_version)
         VALUES ($1,'rule_instance',$2,'verified_rules_4','legacy','legacy@1')",
    ).bind(&historical_point).bind(&historical_rule)
        .execute(&mut *old_tx).await.unwrap();
    sqlx::query(
        "INSERT INTO system.sync_outbox
         (event_id,run_id,aggregate_type,aggregate_key,target_system,event_type,
          payload_type,schema_version,idempotency_key,payload_bytes,payload_hash,
          projection_revision,status)
         VALUES ($1,'historical-run','rule_instance',$2,'qdrant','QdrantUpsertCommand',
                 'alegria.outbox.qdrant_upsert_command.v1',1,'legacy-intent',decode('01','hex'),
                 'legacy-digest',1,'pending')",
    ).bind(historical_event).bind(&historical_rule)
        .execute(&mut *old_tx).await.unwrap();
    old_tx.commit().await.unwrap();

    // Run the real migration, not a reimplementation of its SQL contract.
    sqlx::raw_sql(NEW_MIGRATION).execute(pool).await.unwrap();
    // It must also be safe for an operator to reapply after an interrupted
    // deployment; no historical receipts may be silently rewritten.
    sqlx::raw_sql(NEW_MIGRATION).execute(pool).await.unwrap();
    let (legacy_revision, unproven_xid, legacy_count): (i64, bool, i64) = sqlx::query_as(
        "SELECT projection_revision, staged_xid IS NULL,
                (SELECT count(*) FROM system.sync_outbox
                 WHERE aggregate_key=$1 AND target_system='qdrant')
         FROM system.sync_outbox WHERE event_id=$2",
    ).bind(&historical_rule).bind(historical_event)
        .fetch_one(pool).await.unwrap();
    assert_eq!((legacy_revision, unproven_xid, legacy_count), (1, true, 1),
        "upgrade must preserve provenance ambiguity, not fabricate a writer transaction");

    let mut forbidden_tx = pool.begin().await.unwrap();
    let blocked = lock_and_ensure_verified_rules_writable_in_tx(
        &mut forbidden_tx, std::slice::from_ref(&historical_rule),
    ).await.unwrap_err();
    assert!(matches!(blocked, VerifiedRuleTruthFenceError::LegacyUnproven {
        aggregate_key, event_id
    } if aggregate_key == historical_rule && event_id == historical_event),
        "a committed old revision without xid must remain quarantined");
    forbidden_tx.rollback().await.unwrap();

    // Test the actual Rust leasing API, not an artificial direct status UPDATE.
    // A pre-migration pending intent must never become claimable after upgrade.
    let old_claim = claim_outbox_batch_for_target(
        pool, "migration-legacy-claim", "qdrant", 1, 120, Some(historical_event),
    ).await.unwrap_err();
    let old_claim_db_error = old_claim.downcast_ref::<sqlx::Error>()
        .expect("legacy claim must fail at the PostgreSQL constraint");
    assert_eq!(
        old_claim_db_error.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23514"),
        "legacy receipt must not be silently claimed or rewritten"
    );
    let durable_status: String = sqlx::query_scalar(
        "SELECT status FROM system.sync_outbox WHERE event_id=$1",
    ).bind(historical_event).fetch_one(pool).await.unwrap();
    assert_eq!(durable_status, "pending", "a rejected claim must not mutate the legacy receipt");
    let direct_registry_change = sqlx::query(
        "UPDATE kb.qdrant_points SET embedding_version='unfenced'
         WHERE collection_name='verified_rules_4' AND entity_key=$1",
    ).bind(&historical_rule).execute(pool).await.unwrap_err();
    assert_eq!(
        direct_registry_change.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23514"),
        "historical receipt cannot authorize a new direct registry write"
    );

    // A brand-new, unrelated identity can still use the production owner API.
    // Quarantine is per point and must not block the whole projection system.
    let fresh_rule = format!("fresh-projected-{}", Uuid::new_v4().simple());
    let fresh_batch = VerifiedRuleUpsertBatch {
        run_id: Uuid::new_v4().to_string(),
        context_key: format!("fresh-context-{}", Uuid::new_v4().simple()),
        upserts: vec![VerifiedRuleProjectionCandidate {
            rule_instance_id: fresh_rule.clone(),
            role_type: "document_required".into(),
            concept_key: "passport".into(),
            source_key: "https://fixture.example.gov/passport".into(),
            evidence_quote: "A passport is required".into(),
        }],
    };
    let mut fresh_tx = pool.begin().await.unwrap();
    stage_rule_upserts_in_tx(&mut fresh_tx, &fresh_batch, VerifiedRuleEmbeddingBatch {
        model: "fixture".into(),
        vectors: vec![vec![0.1; 1024]],
    }).await.unwrap();
    fresh_tx.commit().await.unwrap();
    let fresh_revision: (i64, bool) = sqlx::query_as(
        "SELECT projection_revision, staged_xid IS NOT NULL FROM system.sync_outbox
         WHERE aggregate_key=$1 AND target_system='qdrant'",
    ).bind(&fresh_rule).fetch_one(pool).await.unwrap();
    assert_eq!(fresh_revision, (1, true));
}
