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
    projection_delivery::projection_materialize_adapter::dispatch_event(
        "qdrant",
        "QdrantUpsertCommand",
        &entity_key,
        "alegria.outbox.qdrant_upsert_command.v1",
        &cmd.encode_to_vec(),
    )
    .await
    .expect("Qdrant outbox adapter must materialize through REST");

    let points = vector_qdrant::search_dense(&qdrant.client, collection, vector, 5, None)
        .await
        .expect("Qdrant gRPC search must read the REST materialization");
    assert!(
        points.iter().any(|point| {
            vector_qdrant::scored_point_payload_map(point).get("entity_key") == Some(&entity_key)
        }),
        "raw_chunks_ctx REST-upserted point is missing from gRPC search"
    );
}
