// The real SQLx adapter is the only evidence source. Neither Qdrant nor
// Neo4j is connected: retrieval projections cannot supply verified rules.

/// A13: the real editorial source-context port reads disposable Qdrant through
/// Voyage's stubbed HTTP boundary, but never treats a vector hit as PG Truth.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn knowledge_a13_editorial_source_context_uses_real_qdrant_and_typed_port() {
    use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
    use projection_delivery::projection_materialize_adapter;
    use prost::Message;
    use seo_application::knowledge::retrieve_source_context;
    use seo_ports::knowledge::{SourceContextCollection, SourceContextQuery};

    let pg = PostgresHarness::start().await.unwrap();
    let qdrant = QdrantHarness::start().await.unwrap();
    let voyage = stub_servers::spawn_voyage_contract_stub().await.unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
        ("VOYAGE_API_KEY".to_string(), "editorial-a13-stub".to_string()),
        ("VOYAGE_API_BASE_URL".to_string(), format!("{}/v1", voyage.base_url)),
        ("RETRIEVAL_CAPABILITY_REQUIRED".to_string(), "true".to_string()),
    ]);
    let reader = SqlxSeoRuntimeRepository::new(&pg.pool);
    let query = SourceContextQuery {
        run_id: Uuid::new_v4().to_string(),
        text: "Official visa requirements".into(),
        limit: 2,
    };
    let empty = retrieve_source_context(&reader, &query).await.unwrap();
    assert!(empty.is_empty(), "a successful empty Qdrant read must stay empty");

    // Insert through the production Outbox materialization effect. The search
    // result is non-authoritative even when its metadata claims source trust.
    let command = QdrantUpsertCommand {
        event_id: String::new(),
        collection_name: "raw_chunks_4".into(),
        entity_type: "raw_section".into(),
        entity_key: "a13:source-context".into(),
        point_id: Uuid::new_v4().to_string(),
        vector: vec![0.01; 1024],
        payload: None,
        distance: "cosine".into(),
        vector_size: 1024,
        metadata: std::collections::HashMap::from([
            ("retrieval_text".into(), "Official visa requirements".into()),
            ("source_url".into(), "https://example.gov/visa".into()),
            ("source_domain".into(), "example.gov".into()),
            ("section_type".into(), "requirements".into()),
            ("heading_path".into(), "Visa / requirements".into()),
        ]),
    };
    projection_materialize_adapter::dispatch_event(
        "qdrant",
        "QdrantUpsertCommand",
        &command.entity_key,
        "alegria.outbox.qdrant_upsert_command.v1",
        &command.encode_to_vec(),
    )
    .await
    .unwrap();

    let first = retrieve_source_context(&reader, &query).await.unwrap();
    let replay = retrieve_source_context(&reader, &query).await.unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].collection, SourceContextCollection::RawStandard);
    assert_eq!(first[0].chunk_key, "raw_chunks_4:a13:source-context");
    assert_eq!(first[0].source_domain, "example.gov");
    assert_eq!(first[0].content_md, "Official visa requirements");
    assert_eq!(first[0].collection.usage_policy(),
               "standard_source_neighborhood_not_fact_support");

    let verified: i64 = sqlx::query_scalar("SELECT count(*) FROM verified.rule_instances")
        .fetch_one(&pg.pool).await.unwrap();
    assert_eq!(verified, 0, "Qdrant evidence cannot create PostgreSQL verified Truth");
}
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn knowledge_a13_verified_context_reads_only_admissible_global_truth() {
    use primitives::errors::ErrorClass;
    use seo_application::knowledge::generate_verified_blocks;
    use seo_ports::{knowledge::VerifiedRulesQuery, VerifiedRulesReader};

    let pg = PostgresHarness::start().await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string();
    let context_key = format!("knowledge-a13-{suffix}");
    let run_id = Uuid::new_v4().to_string();
    let source_key = format!("https://knowledge-{suffix}.example.gov/fee");
    let country_code = format!("K{}", &suffix[..8]);

    sqlx::query(
        "INSERT INTO kb.visa_contexts (context_key,country_code,visa_family,citizenship_code) \
         VALUES ($1,$2,'tourist','BY')",
    )
    .bind(&context_key)
    .bind(&country_code)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb.sources (source_key,source_type,source_label,trust_level) \
         VALUES ($1,'government','Official source',5)",
    )
    .bind(&source_key)
    .execute(&pg.pool)
    .await
    .unwrap();

    for (idx, status, admissibility, amount) in [
        (1, "verified", "admissible", 35.0),
        (2, "verified", "not_admissible", 900.0),
        (3, "disputed", "admissible", 800.0),
        (4, "verified", "admissible", 700.0),
    ] {
        sqlx::query(
            "INSERT INTO verified.rule_instances \
             (rule_instance_id,context_key,rule_type_key,concept_key,role_type,params,status,source_key,publish_admissibility) \
             VALUES ($1,$2,'consular_fee','consular_fee','fee_item',$3,$4,$5,$6)",
        )
        .bind(format!("knowledge-{suffix}-{idx:02}"))
        .bind(&context_key)
        .bind(sqlx::types::Json(serde_json::json!({"amount": amount, "currency": "EUR"})))
        .bind(status)
        .bind(&source_key)
        .bind(admissibility)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO kb.applicant_profiles (profile_key,profile_type,label_ru) \
         VALUES ('knowledge-a13-student','status','Student') ON CONFLICT DO NOTHING",
    )
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO verified.rule_instance_profiles (rule_instance_id,profile_key) \
         VALUES ($1,'knowledge-a13-student')",
    )
    .bind(format!("knowledge-{suffix}-04"))
    .execute(&pg.pool)
    .await
    .unwrap();

    let query = VerifiedRulesQuery {
        run_id,
        context_key: context_key.clone(),
    };
    let reader = SqlxSeoRuntimeRepository::new(&pg.pool);
    let verified = reader.read_verified_rules(&query).await.unwrap().unwrap();
    assert_eq!(verified.context_key, context_key);
    assert_eq!(verified.rules.len(), 1, "pending, inadmissible or profile-bound rules must not leak");
    assert_eq!(verified.rules[0].role.as_str(), "fee_item");
    assert_eq!(verified.rules[0].source_key.as_deref(), Some(source_key.as_str()));
    assert_eq!(verified.rules[0].params.primary_value_text().as_deref(), Some("35"));

    let first = generate_verified_blocks(&reader, &query).await.unwrap();
    let replay = generate_verified_blocks(&reader, &query).await.unwrap();
    assert_eq!(first, replay, "read-only replay must be deterministic");
    assert!(first["fees_table"].contains("35"));
    for excluded in ["900", "800", "700"] {
        assert!(!first["fees_table"].contains(excluded));
    }

    // PostgreSQL can contain a verified/admissible JSON object without the
    // fields needed to render a numerical claim. It must not become 0 EUR or
    // a fabricated zero-day timeline, even though its status says verified.
    for (idx, role, params, missing_field) in [
        (5, "fee_item", serde_json::json!({"amount": 35.0}), "currency"),
        (6, "fee_item", serde_json::json!({"currency": "EUR"}), "amount"),
        (7, "timeline_item", serde_json::json!({}), "days"),
    ] {
        let rule_id = format!("knowledge-{suffix}-{idx:02}");
        sqlx::query(
            "INSERT INTO verified.rule_instances \
             (rule_instance_id,context_key,rule_type_key,concept_key,role_type,params,status,source_key,publish_admissibility) \
             VALUES ($1,$2,'consular_fee','consular_fee',$3,$4,'verified',$5,'admissible')",
        )
        .bind(&rule_id)
        .bind(&context_key)
        .bind(role)
        .bind(sqlx::types::Json(params))
        .bind(&source_key)
        .execute(&pg.pool)
        .await
        .unwrap();

        let invalid = reader.read_verified_rules(&query).await.unwrap_err();
        assert_eq!(invalid.class(), ErrorClass::ContractViolation);
        assert!(invalid.to_string().contains("stage=required_params"));
        assert!(invalid.to_string().contains(&query.run_id));
        assert!(invalid.to_string().contains(&rule_id));
        assert!(invalid.to_string().contains(missing_field));

        sqlx::query(
            "UPDATE verified.rule_instances SET publish_admissibility='not_admissible' \
             WHERE rule_instance_id=$1",
        )
        .bind(&rule_id)
        .execute(&pg.pool)
        .await
        .unwrap();
        assert_eq!(reader.read_verified_rules(&query).await.unwrap().unwrap().rules.len(), 1);
    }

    sqlx::query(
        "UPDATE verified.rule_instances SET publish_admissibility='not_admissible' \
         WHERE context_key=$1 AND rule_instance_id=$2",
    )
    .bind(&context_key)
    .bind(&verified.rules[0].rule_instance_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    let no_truth = generate_verified_blocks(&reader, &query).await.unwrap_err();
    assert_eq!(no_truth.class(), ErrorClass::ValidationFailure);
    assert!(no_truth.to_string().contains("stage=verified_support"));

    sqlx::query("UPDATE kb.visa_contexts SET status='deprecated' WHERE context_key=$1")
        .bind(&context_key)
        .execute(&pg.pool)
        .await
        .unwrap();
    assert!(reader.read_verified_rules(&query).await.unwrap().is_none());
    let inactive = generate_verified_blocks(&reader, &query).await.unwrap_err();
    assert_eq!(inactive.class(), ErrorClass::ValidationFailure);
    assert!(inactive.to_string().contains("stage=active_context"));
}

/// Domain ownership changes cannot alter the registered Temporal JSON shape.
/// Keep this serialization assertion at the transport boundary, not in Domain.
#[test]
fn knowledge_canonical_mapping_temporal_wire_contract_is_stable() {
    use seo_domain::canonical_mapping::{execute, CanonicalMappingInput, MentionForMapping};

    let input = CanonicalMappingInput {
        section_id: "section-1".into(),
        mentions: vec![MentionForMapping {
            raw_text: "ПАСПОРТ".into(),
            entity_type: "concept".into(),
        }],
    };
    assert_eq!(
        serde_json::to_value(&input).unwrap(),
        serde_json::json!({
            "section_id": "section-1",
            "mentions": [{"raw_text": "ПАСПОРТ", "entity_type": "concept"}]
        })
    );
    let output = execute(&input);
    let json = serde_json::to_value(&output).unwrap();
    assert_eq!(json["mappings"][0]["matching_stage"], "normalized_alias");
    assert_eq!(json["mappings"][0]["canonical_key"], "passport");
    assert_eq!(json["mappings"][0]["needs_hitl"], false);
    assert_eq!(json["mappings"][0]["qdrant_score"], serde_json::Value::Null);
}

/// A13: exercise the exact production canonical-search adapter against real
/// Qdrant, with only the paid Voyage HTTP boundary replaced by a local stub.
/// A vector suggestion changes a mapping, never PostgreSQL verified Truth.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn knowledge_a13_canonical_vector_retrieval_uses_real_qdrant_and_hitl_gate() {
    use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
    use projection_delivery::projection_materialize_adapter;
    use runtime_composition::adapters::canonical_vector_search_adapter::CanonicalVectorSearchAdapter;
    use prost::Message;
    use seo_application::knowledge::resolve_canonical_vector_mappings;
    use seo_domain::canonical_mapping::{
        execute, CanonicalMappingInput, MatchingStage, MentionForMapping,
    };
    use seo_ports::{knowledge::CanonicalVectorLookup, CanonicalVectorSearchPort};

    let qdrant = QdrantHarness::start().await.unwrap();
    let voyage = stub_servers::spawn_voyage_contract_stub().await.unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".to_string()),
        ("VOYAGE_API_KEY".to_string(), "canonical-e2e-stub".to_string()),
        (
            "VOYAGE_API_BASE_URL".to_string(),
            format!("{}/v1", voyage.base_url),
        ),
        ("VOYAGE_CANONICAL_COLLECTION".to_string(), "kb_canonical_4".to_string()),
        ("CANONICAL_VECTOR_RETRIEVAL_REQUIRED".to_string(), "true".to_string()),
        ("VOYAGE_RERANK_REQUIRED".to_string(), "true".to_string()),
    ]);
    let adapter = CanonicalVectorSearchAdapter;
    let mapping_input = CanonicalMappingInput {
        section_id: "canonical-qdrant-e2e".into(),
        mentions: vec![MentionForMapping {
            raw_text: "Unseen consular concept".into(),
            entity_type: "concept".into(),
        }],
    };
    let symbolic = execute(&mapping_input).mappings;
    assert_eq!(symbolic[0].matching_stage, MatchingStage::VectorQdrant);

    // A valid, empty live collection cannot certify the symbolic candidate.
    assert_eq!(
        adapter
            .search_canonical_concepts(&mapping_input.mentions[0].raw_text)
            .await
            .unwrap(),
        CanonicalVectorLookup::Candidates(vec![])
    );
    let empty = resolve_canonical_vector_mappings(&adapter, symbolic.clone())
        .await
        .unwrap();
    assert!(empty[0].needs_hitl);
    assert!(empty[0].canonical_key.is_none());
    assert_eq!(empty[0].match_method, "qdrant_no_candidates");

    // Both hits pass through the same REST materialization used by the Outbox
    // worker. The deterministic Voyage query vector is positive; the second
    // Qdrant vector is opposite, so ranking remains unambiguous before rerank.
    for (concept_key, vector_value) in [
        ("canonical:official_concept", 0.01_f32),
        ("canonical:unrelated_concept", -0.01_f32),
    ] {
        let entity_key = format!("canonical-e2e:{concept_key}");
        let command = QdrantUpsertCommand {
            event_id: String::new(),
            collection_name: "kb_canonical_4".into(),
            entity_type: "canonical_concept".into(),
            entity_key: entity_key.clone(),
            point_id: Uuid::new_v4().to_string(),
            vector: vec![vector_value; 1024],
            payload: None,
            distance: "cosine".into(),
            vector_size: 1024,
            metadata: std::collections::HashMap::from([
                ("concept_key".into(), concept_key.into()),
                ("label_ru".into(), format!("Label for {concept_key}")),
            ]),
        };
        projection_materialize_adapter::dispatch_event(
            "qdrant",
            "QdrantUpsertCommand",
            &entity_key,
            "alegria.outbox.qdrant_upsert_command.v1",
            &command.encode_to_vec(),
        )
        .await
        .unwrap();
    }

    let candidates = adapter
        .search_canonical_concepts(&mapping_input.mentions[0].raw_text)
        .await
        .unwrap();
    let CanonicalVectorLookup::Candidates(candidates) = candidates else {
        panic!("required provider must produce a real Qdrant retrieval result");
    };
    assert_eq!(candidates.len(), 2, "both real Qdrant points must survive retrieval");
    assert_eq!(candidates[0].canonical_key.as_deref(), Some("canonical:official_concept"));
    assert_eq!(candidates[0].score, 1.0, "Voyage rerank must set the top score");
    assert_eq!(candidates[1].score, 0.5, "Voyage rerank must score the second hit");

    let resolved = resolve_canonical_vector_mappings(&adapter, symbolic)
        .await
        .unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].canonical_key.as_deref(), Some("canonical:official_concept"));
    assert_eq!(resolved[0].matching_stage, MatchingStage::VectorQdrant);
    assert_eq!(resolved[0].mapping_type, "auto_map");
    assert!(!resolved[0].needs_hitl);
    assert_eq!(resolved[0].match_method, "qdrant_retrieval");

    // The same real Qdrant and Voyage endpoints also serve the typed
    // read-only neighborhood contract used by Truth diagnostics.
    use runtime_composition::adapters::semantic_neighbor_search_adapter::SemanticNeighborSearchAdapter;
    use seo_application::knowledge::retrieve_semantic_neighbors;
    use seo_ports::knowledge::{SemanticNeighborCollection, SemanticNeighborQuery};

    let neighborhood = retrieve_semantic_neighbors(
        &SemanticNeighborSearchAdapter,
        &SemanticNeighborQuery {
            text: mapping_input.mentions[0].raw_text.clone(),
            primary: SemanticNeighborCollection::CanonicalConcepts,
            fallback: None,
            limit: 5,
            retrieval_required: true,
            contextual_required: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(neighborhood.collection_used.as_deref(), Some("kb_canonical_4"));
    assert_eq!(neighborhood.neighbors.len(), 2);
    assert_eq!(neighborhood.neighbors[0].score, 1.0);
    assert_eq!(neighborhood.reason_codes, vec!["semantic_neighbors_reranked"]);

    let empty_fallback = retrieve_semantic_neighbors(
        &SemanticNeighborSearchAdapter,
        &SemanticNeighborQuery {
            text: mapping_input.mentions[0].raw_text.clone(),
            primary: SemanticNeighborCollection::RawContextual,
            fallback: Some(SemanticNeighborCollection::RawStandard),
            limit: 4,
            retrieval_required: true,
            contextual_required: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(empty_fallback.collection_used.as_deref(), Some("raw_chunks_4"));
    assert!(empty_fallback.neighbors.is_empty());
    assert_eq!(
        empty_fallback.reason_codes,
        vec!["raw_chunks_ctx:empty", "semantic_neighbors_empty"]
    );
}
