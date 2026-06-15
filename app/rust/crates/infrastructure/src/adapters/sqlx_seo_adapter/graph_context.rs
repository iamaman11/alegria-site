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

