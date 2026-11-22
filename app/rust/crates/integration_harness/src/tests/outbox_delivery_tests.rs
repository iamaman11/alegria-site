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
