async fn run_ontology_backfill_plan(
    database_url: Option<String>,
    concept_key: Option<String>,
    limit: i64,
    apply_neo4j: bool,
    apply_qdrant: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let voyage_api_key = env::var("VOYAGE_API_KEY").ok();
    let voyage_model = env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
    let rows = sqlx::query(
        r#"
        SELECT
            c.concept_key,
            c.concept_type,
            c.status,
            c.reg_version,
            COALESCE(c.label_ru, '') AS label_ru,
            COALESCE(COUNT(a.alias_id), 0)::bigint AS alias_count,
            COUNT(r.rule_instance_id)::bigint AS verified_rule_count
        FROM kb.concepts c
        LEFT JOIN kb.concept_aliases a
          ON a.concept_key = c.concept_key
         AND a.status IN ('active','pending')
        LEFT JOIN verified.rule_instances r
          ON r.concept_key = c.concept_key
         AND r.status = 'verified'
        WHERE ($1::text IS NULL OR c.concept_key = $1)
          AND ($1::text IS NOT NULL OR c.status IN ('approved','active'))
        GROUP BY c.concept_key, c.concept_type, c.status, c.reg_version, c.label_ru, c.updated_at
        ORDER BY c.updated_at DESC, c.concept_key ASC
        LIMIT $2
        "#,
    )
    .bind(concept_key.clone())
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    let mut concept_records = Vec::new();
    for row in rows {
        let concept_key: String = row.get("concept_key");
        let alias_rows = sqlx::query(
            r#"
            SELECT alias_text
            FROM kb.concept_aliases
            WHERE concept_key = $1
              AND status IN ('active','pending')
            ORDER BY confidence DESC NULLS LAST, alias_text ASC
            "#,
        )
        .bind(&concept_key)
        .fetch_all(&pool)
        .await?;
        let aliases = alias_rows
            .into_iter()
            .map(|alias_row| alias_row.get::<String, _>("alias_text"))
            .collect::<Vec<_>>();
        concept_records.push((
            concept_key,
            row.get::<String, _>("concept_type"),
            row.get::<String, _>("status"),
            row.get::<i32, _>("reg_version"),
            row.get::<String, _>("label_ru"),
            row.get::<i64, _>("alias_count"),
            row.get::<i64, _>("verified_rule_count"),
            aliases,
        ));
    }

    let voyage_vectors = if apply_qdrant {
        let texts = concept_records
            .iter()
            .map(
                |(
                    concept_key,
                    concept_type,
                    status,
                    _reg_version,
                    label_ru,
                    _alias_count,
                    _verified_rule_count,
                    aliases,
                )| {
                    format!(
                        "{} {} {} {} {}",
                        concept_key,
                        concept_type,
                        status,
                        label_ru,
                        aliases.join(" ")
                    )
                },
            )
            .collect::<Vec<_>>();
        if let Some(api_key) = voyage_api_key.clone() {
            if texts.is_empty() {
                Vec::new()
            } else {
                VoyageClient::new(api_key, voyage_model.clone())
                    .embed_all_with_settings(
                        &texts,
                        &VoyageEmbeddingOptions {
                            input_type: Some(VoyageInputType::Document),
                            output_dimension: Some(1024),
                            output_dtype: Some(VoyageOutputDtype::Float),
                            truncation: Some(false),
                        },
                    )
                    .await?
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut voyage_iter = voyage_vectors.into_iter();
    let qdrant_client = if apply_qdrant {
        Some(qdrant_client_adapter::connect_qdrant(&qdrant_url).await?)
    } else {
        None
    };

    let mut concepts = Vec::new();
    let mut failures = Vec::new();

    for (
        concept_key,
        concept_type,
        status,
        reg_version,
        label_ru,
        alias_count,
        verified_rule_count,
        aliases,
    ) in concept_records
    {
        let mut neo4j_result = json!({ "status": "skipped" });
        if apply_neo4j {
            match infrastructure::adapters::neo4j_materialization_adapter::materialize_concept(
                &concept_key,
            )
            .await
            {
                Ok(()) => {
                    neo4j_result = json!({ "status": "materialized" });
                }
                Err(err) => {
                    neo4j_result = json!({
                        "status": "failed",
                        "error": err.to_string(),
                    });
                    failures.push(json!({
                        "concept_key": concept_key,
                        "failure_class": "neo4j_materialization_failed",
                        "error": err.to_string(),
                    }));
                }
            }
        }

        let mut qdrant_result = json!({ "status": "skipped" });
        if let Some(client) = qdrant_client.as_ref() {
            let embedding_text = format!(
                "{} {} {} {} {}",
                concept_key,
                concept_type,
                status,
                label_ru,
                aliases.join(" ")
            )
            .trim()
            .to_string();
            let (vector, embedding_model, embedding_version) =
                if let Some(vector) = voyage_iter.next() {
                    (vector, voyage_model.as_str(), "ontology_voyage@1")
                } else {
                    (
                        fingerprint_vector(&embedding_text),
                        "deterministic-fingerprint",
                        "ontology_fingerprint@1",
                    )
                };
            let point_id = qdrant_point_id_v1("ontology", "concept", &concept_key);
            let mut payload = BTreeMap::new();
            payload.insert("concept_key".to_string(), concept_key.clone());
            payload.insert("concept_type".to_string(), concept_type.clone());
            payload.insert("status".to_string(), status.clone());
            payload.insert("label_ru".to_string(), label_ru.clone());
            payload.insert("aliases".to_string(), aliases.join(" | "));
            payload.insert("reg_version".to_string(), reg_version.to_string());
            payload.insert(
                "embedding_version".to_string(),
                embedding_version.to_string(),
            );

            match async {
                qdrant_client_adapter::ensure_default_dense_collection(
                    client,
                    "ontology",
                    vector.len() as u64,
                )
                .await?;
                qdrant_client_adapter::upsert_embedding_points(
                    client,
                    "ontology",
                    vec![qdrant_client_adapter::DenseEmbeddingPoint {
                        point_id: point_id.clone(),
                        vector,
                        payload,
                    }],
                )
                .await?;
                sqlx::query(
                    r#"
                    INSERT INTO kb.qdrant_points
                        (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
                    VALUES ($1, 'concept', $2, 'ontology', $3, $4)
                    ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
                    SET point_id = EXCLUDED.point_id,
                        embedding_model = EXCLUDED.embedding_model,
                        embedding_version = EXCLUDED.embedding_version,
                        updated_at = now()
                    "#,
                )
                .bind(&point_id)
                .bind(&concept_key)
                .bind(embedding_model)
                .bind(embedding_version)
                .execute(&pool)
                .await?;
                Result::<()>::Ok(())
            }
            .await
            {
                Ok(()) => {
                    qdrant_result = json!({
                        "status": "materialized",
                        "collection_name": "ontology",
                        "point_id": point_id,
                        "embedding_model": embedding_model,
                        "embedding_version": embedding_version,
                    });
                }
                Err(err) => {
                    qdrant_result = json!({
                        "status": "failed",
                        "error": err.to_string(),
                    });
                    failures.push(json!({
                        "concept_key": concept_key,
                        "failure_class": "qdrant_materialization_failed",
                        "error": err.to_string(),
                    }));
                }
            }
        }

        concepts.push(json!({
            "concept_key": concept_key,
            "concept_type": concept_type,
            "status": status,
            "reg_version": reg_version,
            "label_ru": label_ru,
            "alias_count": alias_count,
            "aliases": aliases,
            "verified_rule_count": verified_rule_count,
            "planned_actions": [
                "graph_materialize",
                "retrieval_reindex"
            ],
            "neo4j": neo4j_result,
            "qdrant": qdrant_result,
        }));
    }

    let payload = json!({
        "status": if failures.is_empty() { "ok" } else { "partial_failure" },
        "apply_neo4j": apply_neo4j,
        "apply_qdrant": apply_qdrant,
        "concept_count": concepts.len(),
        "concepts": concepts,
        "failures": failures,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if failures.is_empty() { 0 } else { 2 })
}

