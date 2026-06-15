fn blank_as_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn owned_blank_as_none(value: &str) -> Option<String> {
    blank_as_none(value).map(ToOwned::to_owned)
}

fn scope_fields(scope: Option<&SeoScopePayload>, fallback_scope_signature: &str) -> ScopeFields {
    let scope_signature = scope
        .and_then(|s| blank_as_none(&s.scope_signature))
        .unwrap_or(fallback_scope_signature)
        .to_string();
    let market = scope
        .and_then(|s| blank_as_none(&s.market))
        .unwrap_or("global")
        .to_string();
    let locale = scope
        .and_then(|s| blank_as_none(&s.locale))
        .unwrap_or("und")
        .to_string();

    ScopeFields {
        scope_signature,
        market,
        locale,
        country_code: scope.and_then(|s| owned_blank_as_none(&s.country_code)),
        visa_type: scope.and_then(|s| owned_blank_as_none(&s.visa_type)),
        applicant_profile: scope.and_then(|s| owned_blank_as_none(&s.applicant_profile)),
    }
}

fn non_empty(value: &str, label: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(validation_failure(format!(
            "SEO persistence requires {label}"
        )));
    }
    Ok(())
}

async fn upsert_runtime_blob<T: RuntimeProtoPayload>(
    pool: &PgPool,
    run_id: &str,
    field_name: &str,
    value: &T,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let (payload_bytes, payload_hash) = encode_runtime_payload(value)?;
    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type = EXCLUDED.payload_type,
            schema_version = EXCLUDED.schema_version,
            payload_bytes = EXCLUDED.payload_bytes,
            payload_hash = EXCLUDED.payload_hash,
            updated_at = now()
        "#,
    )
    .bind(uuid)
    .bind(field_name)
    .bind(T::payload_type())
    .bind(T::schema_version())
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

async fn upsert_runtime_json_blob(
    pool: &PgPool,
    run_id: &str,
    field_name: &str,
    payload_type: &str,
    value: &Value,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let payload_bytes =
        serde_json::to_vec(value).map_err(|e| contract_violation(format!("json encode: {e}")))?;
    let payload_hash = primitives::hash::blake3_hex(&payload_bytes);
    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, $2, $3, 1, $4, $5)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type = EXCLUDED.payload_type,
            payload_bytes = EXCLUDED.payload_bytes,
            payload_hash = EXCLUDED.payload_hash,
            updated_at = now()
        "#,
    )
    .bind(uuid)
    .bind(field_name)
    .bind(payload_type)
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

fn make_idempotency_key(aggregate_key: &str, event_type: &str, payload_bytes: &[u8]) -> String {
    let payload_hash = primitives::hash::blake3_hex(payload_bytes);
    primitives::hash::content_hash_v1(&format!("{aggregate_key}|{event_type}|{payload_hash}"))
}

fn seo_graph_projection_event(
    artifact_type: &str,
    artifact_key: &str,
    scope_signature: &str,
) -> OutboxEnvelope {
    let payload_bytes = encode_payload(&SeoGraphProjectionPayload {
        artifact_type: artifact_type.to_string(),
        artifact_key: artifact_key.to_string(),
        scope_signature: scope_signature.to_string(),
    });
    OutboxEnvelope {
        run_id: String::new(),
        aggregate_type: artifact_type.to_string(),
        aggregate_key: artifact_key.to_string(),
        target_system: "neo4j".to_string(),
        event_type: "SeoGraphProjectionUpserted".to_string(),
        payload_type: "alegria.outbox.seo_graph_projection.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(
            artifact_key,
            "SeoGraphProjectionUpserted",
            &payload_bytes,
        ),
        payload_bytes,
    }
}

fn fingerprint_vector(text: &str) -> Vec<f32> {
    let digest = primitives::hash::blake3_hex(text.as_bytes());
    let mut vector = Vec::with_capacity(16);
    for chunk in digest.as_bytes().chunks(2).take(16) {
        let Ok(hex) = std::str::from_utf8(chunk) else {
            continue;
        };
        let value = u8::from_str_radix(hex, 16).unwrap_or(0);
        vector.push((value as f32 / 127.5) - 1.0);
    }
    if vector.is_empty() {
        vector.push(0.0);
    }
    vector
}

fn seo_qdrant_projection_event(
    collection_name: &str,
    artifact_type: &str,
    artifact_key: &str,
    scope_signature: &str,
    embedding_text: &str,
    mut metadata: HashMap<String, String>,
) -> OutboxEnvelope {
    metadata.insert("artifact_type".to_string(), artifact_type.to_string());
    metadata.insert("artifact_key".to_string(), artifact_key.to_string());
    metadata.insert("scope_signature".to_string(), scope_signature.to_string());
    metadata.insert("retrieval_text".to_string(), embedding_text.to_string());
    metadata.insert(
        "embedding_model".to_string(),
        "deterministic-fingerprint".to_string(),
    );
    metadata.insert(
        "embedding_version".to_string(),
        "seo_projection_bootstrap@1".to_string(),
    );

    let vector = fingerprint_vector(embedding_text);
    let payload_bytes = encode_payload(&QdrantUpsertCommand {
        event_id: String::new(),
        collection_name: collection_name.to_string(),
        entity_type: artifact_type.to_string(),
        entity_key: artifact_key.to_string(),
        point_id: qdrant_point_id_v1(collection_name, artifact_type, artifact_key),
        vector,
        payload: None,
        distance: "cosine".to_string(),
        vector_size: 16,
        metadata,
    });
    OutboxEnvelope {
        run_id: String::new(),
        aggregate_type: artifact_type.to_string(),
        aggregate_key: artifact_key.to_string(),
        target_system: "qdrant".to_string(),
        event_type: "QdrantUpsertCommand".to_string(),
        payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(artifact_key, "QdrantUpsertCommand", &payload_bytes),
        payload_bytes,
    }
}
