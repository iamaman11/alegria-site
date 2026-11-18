fn graph_reason_payload(
    reason_code: &str,
    topic_keys: &[String],
    triple_refs: &[String],
    support_refs: &[String],
    graph_confidence: f64,
) -> Value {
    json!({
        "reason_code": reason_code,
        "topic_keys": topic_keys,
        "triple_refs": triple_refs,
        "support_refs": support_refs,
        "graph_confidence": graph_confidence,
        "source": "graph_planning_context",
    })
}

async fn load_latest_step_output_blob<T: DeserializeOwned>(
    pool: &PgPool,
    run_id: Uuid,
    step_name: &str,
) -> Result<Option<T>, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT payload_bytes
        FROM pipeline.step_payload_blobs
        WHERE run_id = $1
          AND step_name = $2
          AND payload_kind = 'output'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(step_name)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let payload_bytes: Vec<u8> = row.get("payload_bytes");
    serde_json::from_slice::<T>(&payload_bytes)
        .map(Some)
        .map_err(|e| contract_violation(format!("decode {step_name} output blob: {e}")))
}

pub async fn load_seo_site_build_input(
    pool: &PgPool,
    run_id: &str,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    let uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    let run_row = sqlx::query(
        r#"
        SELECT context_key
        FROM pipeline.execution_runs
        WHERE run_id = $1
          AND workflow_type = 'seo_site_build'
        "#,
    )
    .bind(uuid)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure("seo_site_build execution_run row is required"))?;
    let context_key: String = run_row.get("context_key");

    let blob = sqlx::query(
        r#"
        SELECT payload_type, payload_bytes
        FROM pipeline.execution_run_blobs
        WHERE run_id = $1
          AND field_name = 'input_payload'
        LIMIT 1
        "#,
    )
    .bind(uuid)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload input_payload blob is required"))?;
    let payload_type: String = blob.get("payload_type");
    if payload_type != SeoSiteBuildInputPayload::payload_type() {
        return Err(contract_violation(format!(
            "expected {}, got {payload_type}",
            SeoSiteBuildInputPayload::payload_type()
        )));
    }
    let payload_bytes: Vec<u8> = blob.get("payload_bytes");
    let mut input = SeoSiteBuildInputPayload::decode_payload_bytes(&payload_bytes)?;
    if input.run_id != run_id {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload.run_id must match workflow id",
        ));
    }
    if input.context_key != context_key {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload.context_key must match execution_run.context_key",
        ));
    }
    let scope = input
        .scope
        .as_ref()
        .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload.scope is required"))?;
    let validated_scope = identity::derive_scope_from_payload(scope)?;
    validate_applicant_profile_reference(pool, &validated_scope.applicant_profile).await?;
    if input.queries.iter().all(|q| q.trim().is_empty()) {
        return Err(validation_failure(
            "SeoSiteBuildInputPayload requires at least one non-empty query",
        ));
    }

    let ctx = sqlx::query(
        r#"
        SELECT country_code, visa_family, visa_subtype, citizenship_code
        FROM kb.visa_contexts
        WHERE context_key = $1
          AND status = 'active'
        "#,
    )
    .bind(&context_key)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| {
        validation_failure("context_key must reference an active kb.visa_contexts row")
    })?;
    let truth = identity::derive_truth_identity(
        &ctx.get::<String, _>("country_code"),
        &ctx.get::<String, _>("visa_family"),
        ctx.get::<Option<String>, _>("visa_subtype").as_deref(),
        &ctx.get::<String, _>("citizenship_code"),
    )?;
    identity::assert_scope_matches_context(&validated_scope, &truth)?;

    if input.query_batch_key.trim().is_empty() {
        input.query_batch_key = primitives::seo::seo_artifact_key(
            "query_batch",
            &[run_id, &validated_scope.scope_signature],
        );
    }
    if input.run_mode.trim().is_empty() {
        input.run_mode = "publish_with_hitl".to_string();
    }
    Ok(input)
}

pub async fn load_graph_planning_context(
    pool: &PgPool,
    run_id: &str,
    scope_signature: &str,
) -> Result<GraphPlanningContext, DomainError> {
    let run_uuid = Uuid::parse_str(run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;

    let topic_signals = load_latest_step_output_blob::<EditorialExtractionSweepOutputBlob>(
        pool,
        run_uuid,
        "editorial_extraction",
    )
    .await?
    .map(|payload| {
        payload
            .sections
            .into_iter()
            .flat_map(|section| {
                section
                    .topics
                    .into_iter()
                    .map(|topic| GraphPlanningTopicSignal {
                        topic_key: topic.topic_key_candidate.clone(),
                        topic_type: topic.topic_type,
                        support_refs: vec![format!(
                            "step://editorial_extraction/{}",
                            topic.topic_key_candidate
                        )],
                        graph_confidence: 0.82,
                    })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let triple_signals = load_latest_step_output_blob::<TripleBuilderSweepOutputBlob>(
        pool,
        run_uuid,
        "triple_builder",
    )
    .await?
    .map(|payload| {
        payload
            .sections
            .into_iter()
            .flat_map(|section| {
                section
                    .triples
                    .into_iter()
                    .map(|triple| GraphPlanningTripleSignal {
                        triple_id: triple.triple_id,
                        subject_key: triple.subject_key,
                        relation_type: triple.relation_type,
                        object_key: triple.object_key,
                        support_refs: vec![format!("section://{}", triple.evidence_section_id)],
                        graph_confidence: triple.confidence as f64,
                    })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let keyword_clusters = sqlx::query(
        r#"
        SELECT cluster_key, scope_signature, seed_keyword, dominant_intent, status, cluster_version,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.keyword_clusters
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, cluster_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| KeywordCluster {
        cluster_key: row.get("cluster_key"),
        scope_signature: row.get("scope_signature"),
        seed_keyword: row.get("seed_keyword"),
        dominant_intent: row.get("dominant_intent"),
        status: row.get("status"),
        cluster_version: row.get::<i32, _>("cluster_version") as u32,
        reason_code: row.get("reason_code"),
        topic_keys: row.get::<Json<Vec<String>>, _>("topic_keys").0,
        triple_refs: row.get::<Json<Vec<String>>, _>("triple_refs").0,
        graph_confidence: row.get("graph_confidence"),
        support_refs: row.get::<Json<Vec<String>>, _>("support_refs").0,
    })
    .collect::<Vec<_>>();

    let page_nodes = sqlx::query(
        r#"
        SELECT page_node_key, scope_signature, COALESCE(keyword_cluster_key, '') AS keyword_cluster_key,
               COALESCE(blueprint_key, '') AS blueprint_key, page_type_key, dominant_intent,
               canonical_slug, canonical_url_path, lifecycle_state
        FROM site.page_nodes
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, page_node_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| PageNode {
        page_node_key: row.get("page_node_key"),
        scope_signature: row.get("scope_signature"),
        keyword_cluster_key: row.get("keyword_cluster_key"),
        blueprint_key: row.get("blueprint_key"),
        page_type_key: row.get("page_type_key"),
        dominant_intent: row.get("dominant_intent"),
        canonical_slug: row.get("canonical_slug"),
        canonical_url_path: row.get("canonical_url_path"),
        lifecycle_state: row.get("lifecycle_state"),
    })
    .collect::<Vec<_>>();

    let content_gaps = sqlx::query(
        r#"
        SELECT content_gap_key, scope_signature, COALESCE(page_node_key, '') AS page_node_key,
               missing_topic, severity, status,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.content_gaps
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, content_gap_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .map(|row| ContentGap {
        content_gap_key: row.get("content_gap_key"),
        scope_signature: row.get("scope_signature"),
        page_node_key: row.get("page_node_key"),
        missing_topic: row.get("missing_topic"),
        severity: row.get("severity"),
        status: row.get("status"),
        reason_code: row.get("reason_code"),
        topic_keys: row.get::<Json<Vec<String>>, _>("topic_keys").0,
        triple_refs: row.get::<Json<Vec<String>>, _>("triple_refs").0,
        graph_confidence: row.get("graph_confidence"),
        support_refs: row.get::<Json<Vec<String>>, _>("support_refs").0,
    })
    .collect::<Vec<_>>();

    let link_recommendation_rows = sqlx::query(
        r#"
        SELECT link_recommendation_key, scope_signature, source_page_key, target_page_key,
               link_role, anchor_strategy, required_flag, score::double precision AS score, status,
               COALESCE(reason_payload->>'reason_code', '') AS reason_code,
               COALESCE(reason_payload->'topic_keys', '[]'::jsonb) AS topic_keys,
               COALESCE(reason_payload->'triple_refs', '[]'::jsonb) AS triple_refs,
               COALESCE(reason_payload->>'graph_confidence', '0') AS graph_confidence,
               COALESCE(reason_payload->'support_refs', '[]'::jsonb) AS support_refs
        FROM site.link_recommendations
        WHERE scope_signature = $1
        ORDER BY updated_at DESC, link_recommendation_key
        "#,
    )
    .bind(scope_signature)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    let mut link_recommendations = Vec::with_capacity(link_recommendation_rows.len());
    for row in link_recommendation_rows {
        let recommendation = LinkRecommendation {
            link_recommendation_key: row
                .try_get("link_recommendation_key")
                .map_err(classify_sqlx)?,
            scope_signature: row.try_get("scope_signature").map_err(classify_sqlx)?,
            source_page_key: row.try_get("source_page_key").map_err(classify_sqlx)?,
            target_page_key: row.try_get("target_page_key").map_err(classify_sqlx)?,
            link_role: row.try_get("link_role").map_err(classify_sqlx)?,
            anchor_strategy: row.try_get("anchor_strategy").map_err(classify_sqlx)?,
            required_flag: row.try_get("required_flag").map_err(classify_sqlx)?,
            score: row.try_get::<f64, _>("score").map_err(classify_sqlx)?,
            status: row.try_get("status").map_err(classify_sqlx)?,
            reason_code: row.try_get("reason_code").map_err(classify_sqlx)?,
            topic_keys: row
                .try_get::<Json<Vec<String>>, _>("topic_keys")
                .map_err(classify_sqlx)?
                .0,
            triple_refs: row
                .try_get::<Json<Vec<String>>, _>("triple_refs")
                .map_err(classify_sqlx)?
                .0,
            graph_confidence: row.try_get("graph_confidence").map_err(classify_sqlx)?,
            support_refs: row
                .try_get::<Json<Vec<String>>, _>("support_refs")
                .map_err(classify_sqlx)?
                .0,
        };
        link_recommendations.push(recommendation);
    }

    let coverage_signals = page_nodes
        .iter()
        .map(|page| {
            let cluster_topics = keyword_clusters
                .iter()
                .find(|cluster| cluster.cluster_key == page.keyword_cluster_key)
                .map(|cluster| cluster.topic_keys.clone())
                .unwrap_or_default();
            let missing_topic_keys = content_gaps
                .iter()
                .filter(|gap| {
                    gap.page_node_key == page.page_node_key || gap.page_node_key.is_empty()
                })
                .map(|gap| gap.missing_topic.clone())
                .collect::<Vec<_>>();
            GraphPlanningCoverageSignal {
                page_node_key: page.page_node_key.clone(),
                keyword_cluster_key: page.keyword_cluster_key.clone(),
                covered_topic_keys: cluster_topics,
                missing_topic_keys,
                graph_confidence: 0.75,
            }
        })
        .collect::<Vec<_>>();

    Ok(GraphPlanningContext {
        scope_signature: scope_signature.to_string(),
        topic_signals,
        triple_signals,
        coverage_signals,
        keyword_clusters,
        page_nodes,
        content_gaps,
        link_recommendations,
    })
}

pub async fn load_section_templates(
    pool: &PgPool,
    page_type_key: &str,
    dominant_intent: &str,
) -> Result<Vec<SectionTemplateBinding>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT section_template_key, page_type_key, dominant_intent, section_role,
               template_version, template_body
        FROM site.section_templates
        WHERE page_type_key = $1
          AND dominant_intent = $2
          AND status = 'active'
        ORDER BY section_role, template_version DESC
        "#,
    )
    .bind(page_type_key)
    .bind(dominant_intent)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut seen_roles = std::collections::BTreeSet::new();
    let mut templates = Vec::new();
    for row in rows {
        let section_role: String = row.get("section_role");
        if !seen_roles.insert(section_role.clone()) {
            continue;
        }
        templates.push(SectionTemplateBinding {
            template_key: row.get("section_template_key"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            section_role: section_role.clone(),
            heading: runtime_models::seo_blocks::heading_for_role(&section_role).to_string(),
            template_body: row.get("template_body"),
            template_version: row.get::<i32, _>("template_version") as u32,
            required: true,
        });
    }
    Ok(templates)
}



// Retained processing is an explicit property of a registered canonical run.
// Acquisition receipts and immutable raw snapshots remain owned by the original
// run. Never infer reuse from a matching keyword, or fall back to paid search.
pub async fn load_terminal_retained_source(
    pool: &PgPool,
    run_id: &str,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    let id = Uuid::parse_str(run_id)
        .map_err(|_| validation_failure("retained source run must be a UUID"))?;
    let eligible: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            WHERE r.run_id = $1 AND r.workflow_type = 'seo_site_build'
              AND r.status IN ('done','failed')
              AND NOT EXISTS (
                  SELECT 1 FROM pipeline.step_executions e
                  WHERE e.run_id = r.run_id AND e.status = 'running'
              )
         )",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    if !eligible {
        return Err(validation_failure("retained source run must be terminal with no running steps"));
    }
    let source = load_seo_site_build_input(pool, run_id).await?;
    if !source.retained_source_run_id.is_empty() {
        return Err(validation_failure("select the original acquisition run, not a retained copy"));
    }
    Ok(source)
}

pub fn verify_retained_scope(
    current: &SeoSiteBuildInputPayload,
    source: &SeoSiteBuildInputPayload,
) -> Result<(), DomainError> {
    if current.run_id == source.run_id
        || current.retained_source_run_id != source.run_id
        || current.context_key != source.context_key
        || current.scope.is_none()
        || current.scope != source.scope
        || current.queries != source.queries
        || current.query_batch_key == source.query_batch_key
        || !source.retained_source_run_id.is_empty()
    {
        return Err(validation_failure("retained corpus identity, scope or demand differs from original acquisition"));
    }
    Ok(())
}

async fn retained_acquisition_owner(
    pool: &PgPool,
    run_id: &str,
    batch: &str,
) -> Result<Option<SeoSiteBuildInputPayload>, DomainError> {
    let id = Uuid::parse_str(run_id)
        .map_err(|_| validation_failure("invalid acquisition run UUID"))?;
    let stored = sqlx::query(
        "SELECT payload_type, payload_bytes, payload_hash
         FROM pipeline.execution_run_blobs
         WHERE run_id = $1 AND field_name = 'input_payload'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;
    // Standalone diagnostic activities have no registered site input.
    let Some(row) = stored else { return Ok(None); };
    let ty: String = row.get("payload_type");
    let bytes: Vec<u8> = row.get("payload_bytes");
    let hash: String = row.get("payload_hash");
    if ty != SeoSiteBuildInputPayload::payload_type()
        || primitives::hash::blake3_hex(&bytes) != hash
    {
        return Err(contract_violation("invalid registered acquisition input"));
    }
    let current = SeoSiteBuildInputPayload::decode_payload_bytes(&bytes)?;
    if current.retained_source_run_id.is_empty() {
        return Ok(None);
    }
    if current.run_id != run_id || current.query_batch_key != batch {
        return Err(validation_failure("retained acquisition request differs from registered input"));
    }
    let source = load_terminal_retained_source(pool, &current.retained_source_run_id).await?;
    verify_retained_scope(&current, &source)?;
    Ok(Some(source))
}

async fn retained_step_outputs<T: RuntimeProtoPayload>(
    pool: &PgPool,
    source_run: &str,
    step_name: &str,
) -> Result<Vec<T>, DomainError> {
    let id = Uuid::parse_str(source_run)
        .map_err(|_| validation_failure("invalid retained source run UUID"))?;
    let rows = sqlx::query(
        "SELECT b.payload_type, b.payload_bytes, b.payload_hash, e.output_hash
         FROM pipeline.step_executions e
         JOIN pipeline.step_payload_blobs b ON b.run_id=e.run_id
           AND b.step_name=e.step_name AND b.idempotency_key=e.idempotency_key
           AND b.payload_kind='output'
         WHERE e.run_id=$1 AND e.step_name=$2 AND e.status='done'
         ORDER BY e.created_at, e.step_execution_id",
    )
    .bind(id)
    .bind(step_name)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    if rows.is_empty() {
        return Err(validation_failure(format!("retained acquisition has no completed {step_name} receipt")));
    }
    rows.into_iter().map(|row| {
        let ty: String = row.get("payload_type");
        let bytes: Vec<u8> = row.get("payload_bytes");
        let hash: String = row.get("payload_hash");
        let step_hash: Option<String> = row.get("output_hash");
        if ty != T::payload_type()
            || step_hash.as_deref() != Some(hash.as_str())
            || primitives::hash::blake3_hex(&bytes) != hash
        {
            return Err(contract_violation(format!("retained {step_name} receipt hash/type mismatch")));
        }
        T::decode_payload_bytes(&bytes)
    }).collect()
}

pub async fn retained_serp_ingest(
    pool: &PgPool,
    input: &SerpIngestInputPayload,
) -> Result<Option<SerpIngestOutputPayload>, DomainError> {
    let Some(source) = retained_acquisition_owner(pool, &input.run_id, &input.query_batch_key).await? else {
        return Ok(None);
    };
    if input.queries != source.queries || input.scope != source.scope {
        return Err(validation_failure("retained SERP demand or scope mismatch"));
    }
    let prior = retained_step_outputs::<SerpIngestOutputPayload>(
        pool, &source.run_id, "serp_ingest",
    ).await?;
    let [receipt] = prior.as_slice() else {
        return Err(validation_failure("retained SERP receipt is ambiguous"));
    };
    let expected = u32::try_from(source.queries.len())
        .map_err(|_| validation_failure("retained query count overflow"))?;
    if receipt.status != "done" || receipt.query_batch_key != source.query_batch_key
        || receipt.query_count != expected || receipt.persisted_snapshot_count != expected
    {
        return Err(validation_failure("retained SERP acquisition incomplete"));
    }

    // Copy only the exact successful provider results, never a synthetic query
    // observation or an unpaid/uncertain task POST receipt. Preserve provenance.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    for query in &source.queries {
        let rows = sqlx::query(
            "SELECT job_id, raw_result FROM serp.raw_snapshots
             WHERE run_id=$1 AND query=$2 AND job_id NOT LIKE 'dataforseo-standard-task-%'
               AND raw_result->>'status_code'='20000'
               AND raw_result->'tasks'->0->>'status_code'='20000'
               AND jsonb_typeof(raw_result->'tasks'->0->'result')='array'",
        )
        .bind(&source.run_id)
        .bind(query)
        .fetch_all(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        let [row] = rows.as_slice() else {
            return Err(validation_failure("retained query has missing or ambiguous provider result"));
        };
        let source_job: String = row.get("job_id");
        let mut result: Value = row.get("raw_result");
        if result["tasks"][0]["result"].as_array().is_none_or(Vec::is_empty) {
            return Err(validation_failure("retained provider result is empty"));
        }
        result["retained_provenance"] = json!({
            "source_run_id": source.run_id, "source_job_id": source_job,
        });
        let key = primitives::seo::seo_artifact_key(
            "retained_serp_result", &[&input.run_id, &source.run_id, &source_job],
        );
        sqlx::query(
            "INSERT INTO serp.raw_snapshots (run_id,job_id,query,raw_result)
             VALUES ($1,$2,$3,$4) ON CONFLICT (run_id,job_id) DO NOTHING",
        )
        .bind(&input.run_id).bind(&key).bind(query).bind(Json(&result))
        .execute(&mut *tx).await.map_err(classify_sqlx)?;
        let saved: Value = sqlx::query_scalar(
            "SELECT raw_result FROM serp.raw_snapshots WHERE run_id=$1 AND job_id=$2 AND query=$3",
        )
        .bind(&input.run_id).bind(&key).bind(query)
        .fetch_one(&mut *tx).await.map_err(classify_sqlx)?;
        if saved != result {
            return Err(contract_violation("retained SERP result changed on retry"));
        }
    }
    tx.commit().await.map_err(classify_sqlx)?;
    let output = SerpIngestOutputPayload {
        query_batch_key: input.query_batch_key.clone(),
        query_count: expected,
        persisted_snapshot_count: expected,
        status: "done".into(),
    };
    persist_serp_ingest_output(pool, input, &output).await?;
    tracing::info!(run_id=%input.run_id, source_run_id=%source.run_id, query_count=expected,
        "reused acquired SERP results without provider calls");
    Ok(Some(output))
}

pub async fn retained_crawl_sources(
    pool: &PgPool,
    input: &contracts::generated::alegria::temporal::v1::CrawlSourcesInputPayload,
) -> Result<Option<contracts::generated::alegria::temporal::v1::CrawlSourcesOutputPayload>, DomainError> {
    use contracts::generated::alegria::temporal::v1::CrawlSourcesOutputPayload;
    let Some(source) = retained_acquisition_owner(pool, &input.run_id, &input.query_batch_key).await? else {
        return Ok(None);
    };
    if input.sweep_index != 0 {
        return Err(validation_failure("retained corpus must be loaded in a single sweep"));
    }
    let prior = retained_step_outputs::<CrawlSourcesOutputPayload>(pool, &source.run_id, "crawl_sources").await?;
    let mut ids = BTreeSet::new();
    let mut failed_urls = BTreeSet::new();
    for (index, receipt) in prior.iter().enumerate() {
        let final_receipt = index + 1 == prior.len();
        let valid_status = if final_receipt {
            matches!(receipt.status.as_str(), "done" | "partial")
        } else {
            matches!(receipt.status.as_str(), "more" | "pending_retry")
        };
        if !valid_status || receipt.raw_page_count as usize != receipt.raw_page_ids.len() {
            return Err(validation_failure("retained crawl receipt sequence is incomplete"));
        }
        for id in &receipt.raw_page_ids {
            if *id <= 0 || !ids.insert(*id) {
                return Err(validation_failure("retained crawl contains repeated or invalid page identity"));
            }
        }
        failed_urls.extend(receipt.failed_urls.iter().cloned());
    }
    let mut raw_page_ids = Vec::new();
    let mut raw_section_count = 0u32;
    for id in ids {
        let page = sqlx::query(
            "SELECT url, status_code, raw_html, content_hash FROM raw.pages WHERE id=$1",
        )
        .bind(id).fetch_one(pool).await.map_err(classify_sqlx)?;
        let url: String = page.get("url");
        let status: Option<i16> = page.get("status_code");
        let html: Option<String> = page.get("raw_html");
        let hash: Option<String> = page.get("content_hash");
        let html = html.ok_or_else(|| validation_failure(format!("retained page {id} has no snapshot bytes")))?;
        if hash.as_deref() != Some(primitives::hash::content_hash_v1(&html).as_str()) {
            return Err(contract_violation(format!("retained page {id} snapshot hash mismatch")));
        }
        let sections = sqlx::query(
            "SELECT content_md, content_hash FROM raw.sections WHERE page_id=$1 ORDER BY section_order,id",
        )
        .bind(id).fetch_all(pool).await.map_err(classify_sqlx)?;
        if status != Some(200) || html.trim().is_empty()
            || sections.is_empty()
            || sections.iter().all(|row| row.get::<String,_>("content_md").trim().is_empty())
        {
            failed_urls.insert(url);
            continue;
        }
        for section in &sections {
            let text: String = section.get("content_md");
            let actual: String = section.get("content_hash");
            if !text.trim().is_empty() && primitives::hash::content_hash_v1(&text) != actual {
                return Err(contract_violation(format!("retained section hash mismatch on page {id}")));
            }
        }
        raw_section_count = raw_section_count.saturating_add(sections.len() as u32);
        raw_page_ids.push(id);
    }
    if raw_page_ids.is_empty() {
        return Err(validation_failure("retained corpus has no usable source snapshots"));
    }
    let failed_urls: Vec<_> = failed_urls.into_iter().collect();
    tracing::info!(run_id=%input.run_id, source_run_id=%source.run_id,
        pages=raw_page_ids.len(), failed=failed_urls.len(), "reused immutable raw corpus without crawling");
    Ok(Some(CrawlSourcesOutputPayload {
        claimed_count: 0, crawled_count: 0, failed_count: failed_urls.len() as u32,
        raw_page_count: raw_page_ids.len() as u32, raw_section_count,
        qdrant_event_count: 0,
        status: if failed_urls.is_empty() { "done" } else { "partial" }.into(),
        raw_page_ids, failed_urls,
    }))
}

#[cfg(test)]
mod retained_corpus_identity_tests {
    use super::*;

    #[test]
    fn retained_corpus_requires_exact_context_scope_and_demand() {
        let source = SeoSiteBuildInputPayload {
            run_id: "old".into(), context_key: "original".into(),
            scope: Some(SeoScopePayload { locale: "ru-RU".into(), ..Default::default() }),
            query_batch_key: "old-batch".into(), queries: vec!["real query".into()],
            ..Default::default()
        };
        let mut current = SeoSiteBuildInputPayload {
            run_id: "new".into(), query_batch_key: "new-batch".into(),
            retained_source_run_id: source.run_id.clone(), ..source.clone()
        };
        assert!(verify_retained_scope(&current,&source).is_ok());
        current.context_key="different".into();
        assert!(verify_retained_scope(&current,&source).is_err());
        current.context_key=source.context_key.clone();
        current.queries.push("invented demand".into());
        assert!(verify_retained_scope(&current,&source).is_err());
        current.queries=source.queries.clone();
        current.query_batch_key=source.query_batch_key.clone();
        assert!(verify_retained_scope(&current,&source).is_err());
    }
}
