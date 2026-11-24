pub async fn upsert_seo_site_build_input(
    pool: &PgPool,
    input: &SeoSiteBuildInputPayload,
) -> Result<(), DomainError> {
    let uuid = Uuid::parse_str(&input.run_id)
        .map_err(|e| contract_violation(format!("invalid run_id uuid: {e}")))?;
    non_empty(&input.context_key, "context_key")?;
    let scope = input
        .scope
        .as_ref()
        .ok_or_else(|| validation_failure("SeoSiteBuildInputPayload.scope is required"))?;
    let validated_scope = seo_steps::seo_identity::derive_scope_from_payload(scope)?;
    validate_applicant_profile_reference(pool, &validated_scope.applicant_profile).await?;
    let payload_bytes = input.encode_payload_bytes()?;
    let payload_hash = primitives::hash::blake3_hex(&payload_bytes);

    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_runs
            (run_id, workflow_run_id, workflow_type, context_key, status)
        VALUES ($1, $2, 'seo_site_build', $3, 'created')
        ON CONFLICT (run_id) DO UPDATE
        SET workflow_run_id = EXCLUDED.workflow_run_id,
            workflow_type   = EXCLUDED.workflow_type,
            context_key     = EXCLUDED.context_key,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(uuid)
    .bind(&input.run_id)
    .bind(&input.context_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        INSERT INTO pipeline.execution_run_blobs
            (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash)
        VALUES ($1, 'input_payload', $2, $3, $4, $5)
        ON CONFLICT (run_id, field_name) DO UPDATE
        SET payload_type    = EXCLUDED.payload_type,
            schema_version  = EXCLUDED.schema_version,
            payload_bytes   = EXCLUDED.payload_bytes,
            payload_hash    = EXCLUDED.payload_hash,
            updated_at      = now()
        "#,
    )
    .bind(uuid)
    .bind(SeoSiteBuildInputPayload::payload_type())
    .bind(SeoSiteBuildInputPayload::schema_version())
    .bind(payload_bytes)
    .bind(payload_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(())
}

pub async fn persist_serp_ingest_output(
    pool: &PgPool,
    input: &SerpIngestInputPayload,
    output: &SerpIngestOutputPayload,
) -> Result<(), DomainError> {
    let fallback_scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.as_str())
        .filter(|scope| !scope.trim().is_empty())
        .unwrap_or("default");
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);
    let query_batch_key = if output.query_batch_key.trim().is_empty() {
        &input.query_batch_key
    } else {
        &output.query_batch_key
    };
    non_empty(query_batch_key, "query_batch_key")?;

    sqlx::query(
        r#"
        INSERT INTO serp.query_batches
            (query_batch_key, scope_signature, market, locale, source_system, batch_version, status)
        VALUES ($1, $2, $3, $4, 'temporal_serp_ingest', 1, $5)
        ON CONFLICT (query_batch_key) DO UPDATE
        SET scope_signature = EXCLUDED.scope_signature,
            market          = EXCLUDED.market,
            locale          = EXCLUDED.locale,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(query_batch_key)
    .bind(&scope.scope_signature)
    .bind(&scope.market)
    .bind(&scope.locale)
    .bind(&output.status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    for (idx, query) in input.queries.iter().enumerate() {
        if query.trim().is_empty() {
            continue;
        }
        let job_id = format!("seo-query-{:04}", idx + 1);
        sqlx::query(
            r#"
            INSERT INTO serp.raw_snapshots
                (run_id, job_id, query, recorded_at, raw_result)
            VALUES ($1, $2, $3, now(), $4)
            ON CONFLICT (run_id, job_id) DO UPDATE
            SET query       = EXCLUDED.query,
                recorded_at = EXCLUDED.recorded_at,
                raw_result  = EXCLUDED.raw_result
            "#,
        )
        .bind(&input.run_id)
        .bind(job_id)
        .bind(query.trim())
        .bind(Json(json!({
            "query": query.trim(),
            "query_batch_key": query_batch_key,
            "source_system": "seo_site_build_input",
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(())
}

pub async fn persist_serp_normalize_output(
    pool: &PgPool,
    input: &SerpNormalizeInputPayload,
    output: &SerpNormalizeOutputPayload,
) -> Result<(), DomainError> {
    let fallback_scope_signature = if output.scope_signature.trim().is_empty() {
        "default"
    } else {
        &output.scope_signature
    };
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);
    let query_batch_key = output
        .serp_patterns
        .first()
        .and_then(|p| blank_as_none(&p.query_batch_key))
        .or_else(|| blank_as_none(&input.query_batch_key))
        .ok_or_else(|| validation_failure("SEO persistence requires query_batch_key"))?;

    sqlx::query(
        r#"
        INSERT INTO serp.query_batches
            (query_batch_key, scope_signature, market, locale, source_system, batch_version, status)
        VALUES ($1, $2, $3, $4, 'temporal_serp_normalize', 1, 'done')
        ON CONFLICT (query_batch_key) DO UPDATE
        SET scope_signature = EXCLUDED.scope_signature,
            market          = EXCLUDED.market,
            locale          = EXCLUDED.locale,
            status          = EXCLUDED.status,
            updated_at      = now()
        "#,
    )
    .bind(query_batch_key)
    .bind(&scope.scope_signature)
    .bind(&scope.market)
    .bind(&scope.locale)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut projection_events = Vec::new();
    for pattern in &output.serp_patterns {
        non_empty(&pattern.serp_pattern_key, "serp_pattern_key")?;
        non_empty(&pattern.query, "serp_pattern.query")?;
        sqlx::query(
            r#"
            INSERT INTO serp.serp_patterns
                (serp_pattern_key, query_batch_key, scope_signature, query, pattern_type,
                 dominant_intent, reliability_score, evidence_ref, pattern_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, 'seo_serp_pattern@1', $9)
            ON CONFLICT (serp_pattern_key) DO UPDATE
            SET query_batch_key   = EXCLUDED.query_batch_key,
                scope_signature   = EXCLUDED.scope_signature,
                query             = EXCLUDED.query,
                pattern_type      = EXCLUDED.pattern_type,
                dominant_intent   = EXCLUDED.dominant_intent,
                reliability_score = EXCLUDED.reliability_score,
                evidence_ref      = EXCLUDED.evidence_ref,
                status            = EXCLUDED.status,
                updated_at        = now()
            "#,
        )
        .bind(&pattern.serp_pattern_key)
        .bind(&pattern.query_batch_key)
        .bind(&pattern.scope_signature)
        .bind(&pattern.query)
        .bind(&pattern.pattern_type)
        .bind(&pattern.dominant_intent)
        .bind(pattern.reliability_score)
        .bind(blank_as_none(&pattern.evidence_ref))
        .bind(&pattern.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        sqlx::query(
            r#"
            INSERT INTO serp.serp_pattern_observations
                (observation_key, serp_pattern_key, observed_at, observation_payload)
            VALUES ($1, $2, now(), $3)
            ON CONFLICT (observation_key) DO UPDATE
            SET observation_payload = EXCLUDED.observation_payload,
                observed_at = now()
            "#,
        )
        .bind(seo_domain::artifact_identity::seo_artifact_key(
            "serp_pattern_observation",
            &[
                &pattern.serp_pattern_key,
                &pattern.query_batch_key,
                &pattern.query,
            ],
        ))
        .bind(&pattern.serp_pattern_key)
        .bind(Json(json!({
            "query": pattern.query,
            "pattern_type": pattern.pattern_type,
            "dominant_intent": pattern.dominant_intent,
            "status": pattern.status,
            "evidence_ref": pattern.evidence_ref,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if pattern.status == "partial" {
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                VALUES ($1, 'unsafe_serp_pattern', 'open', 'seo_ops', 'seo_ops',
                        $2, 'serp_normalize', $3, 'important', $4, $5)
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    decision_payload = EXCLUDED.decision_payload,
                    updated_at = now()
                "#,
            )
            .bind(seo_domain::artifact_identity::seo_artifact_key(
                "seo_hitl_task",
                &[
                    &pattern.serp_pattern_key,
                    "unsafe_serp_pattern",
                    "serp_normalize@1",
                ],
            ))
            .bind(&pattern.scope_signature)
            .bind(&pattern.serp_pattern_key)
            .bind(Json(json!({
                "query": pattern.query,
                "dominant_intent": pattern.dominant_intent,
                "reliability_score": pattern.reliability_score,
            })))
            .bind(Json(json!([{
                "event": "unsafe_serp_pattern_detected",
                "query": pattern.query,
            }])))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }

        projection_events.push(seo_graph_projection_event_with_revision(
            "serp_pattern",
            &pattern.serp_pattern_key,
            &pattern.scope_signature,
            &format!(
                "{}|{}|{}|{}",
                pattern.query, pattern.pattern_type, pattern.dominant_intent, pattern.status
            ),
        ));
        projection_events.push(seo_qdrant_projection_event(
            &input.run_id,
            "seo_serp_patterns",
            "serp_pattern",
            &pattern.serp_pattern_key,
            &pattern.scope_signature,
            &format!(
                "{} {} {} {}",
                pattern.query, pattern.pattern_type, pattern.dominant_intent, pattern.status
            ),
            HashMap::from([
                ("query".to_string(), pattern.query.clone()),
                ("pattern_type".to_string(), pattern.pattern_type.clone()),
                (
                    "dominant_intent".to_string(),
                    pattern.dominant_intent.clone(),
                ),
                ("status".to_string(), pattern.status.clone()),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_opportunity_build_output(
    pool: &PgPool,
    input: &OpportunityBuildInputPayload,
    output: &OpportunityBuildOutputPayload,
) -> Result<OpportunityBuildOutputPayload, DomainError> {
    let fallback_scope_signature = input
        .scope
        .as_ref()
        .and_then(|s| blank_as_none(&s.scope_signature))
        .unwrap_or("default");
    let scope = scope_fields(input.scope.as_ref(), fallback_scope_signature);

    // Technical SERP family keys can change between retained-corpus passes.
    // The existing natural-key owner must survive: page nodes and graph edges
    // may already reference it. Resolve before persisting *and* before handing
    // the output to downstream IA, rather than weakening the UNIQUE constraint.
    let mut canonical = output.clone();
    for cluster in &mut canonical.keyword_clusters {
        non_empty(&cluster.cluster_key, "cluster_key")?;
        non_empty(&cluster.seed_keyword, "seed_keyword")?;
        let existing: Vec<String> = sqlx::query_scalar(
            r#"
            SELECT cluster_key FROM site.keyword_clusters
            WHERE market = $1 AND locale = $2
              AND country_code IS NOT DISTINCT FROM $3
              AND visa_type IS NOT DISTINCT FROM $4
              AND applicant_profile IS NOT DISTINCT FROM $5
              AND seed_keyword = $6 AND cluster_version = $7
            ORDER BY cluster_key LIMIT 2
            "#,
        )
        .bind(&scope.market)
        .bind(&scope.locale)
        .bind(&scope.country_code)
        .bind(&scope.visa_type)
        .bind(&scope.applicant_profile)
        .bind(&cluster.seed_keyword)
        .bind(cluster.cluster_version as i32)
        .fetch_all(pool)
        .await
        .map_err(classify_sqlx)?;
        if existing.len() > 1 {
            return Err(contract_violation(
                "ambiguous existing keyword natural owner; refuse to reassign references",
            ));
        }
        if let Some(owner_key) = existing.first() {
            cluster.cluster_key = owner_key.clone();
        }
    }

    let mut projection_events = Vec::new();
    for cluster in &canonical.keyword_clusters {
        non_empty(&cluster.cluster_key, "cluster_key")?;
        non_empty(&cluster.seed_keyword, "seed_keyword")?;
        let cluster_scope = if cluster.scope_signature.trim().is_empty() {
            &scope.scope_signature
        } else {
            &cluster.scope_signature
        };
        sqlx::query(
            r#"
            INSERT INTO site.keyword_clusters
                (cluster_key, scope_signature, market, locale, country_code, visa_type,
                 applicant_profile, seed_keyword, dominant_intent, cluster_version,
                 derivation_version, status, reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'seo_cluster@1', $11, $12, 'graph_planning@1')
            ON CONFLICT (cluster_key) DO UPDATE
            SET scope_signature   = EXCLUDED.scope_signature,
                market            = EXCLUDED.market,
                locale            = EXCLUDED.locale,
                country_code      = EXCLUDED.country_code,
                visa_type         = EXCLUDED.visa_type,
                applicant_profile = EXCLUDED.applicant_profile,
                seed_keyword      = EXCLUDED.seed_keyword,
                dominant_intent   = EXCLUDED.dominant_intent,
                cluster_version   = EXCLUDED.cluster_version,
                reason_payload    = EXCLUDED.reason_payload,
                reason_version    = EXCLUDED.reason_version,
                status            = EXCLUDED.status,
                updated_at        = now()
            "#,
        )
        .bind(&cluster.cluster_key)
        .bind(cluster_scope)
        .bind(&scope.market)
        .bind(&scope.locale)
        .bind(&scope.country_code)
        .bind(&scope.visa_type)
        .bind(&scope.applicant_profile)
        .bind(&cluster.seed_keyword)
        .bind(&cluster.dominant_intent)
        .bind(cluster.cluster_version as i32)
        .bind(&cluster.status)
        .bind(Json(graph_reason_payload(
            &cluster.reason_code,
            &cluster.topic_keys,
            &cluster.triple_refs,
            &cluster.support_refs,
            cluster.graph_confidence,
        )))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event_with_revision(
            "keyword_cluster",
            &cluster.cluster_key,
            cluster_scope,
            &format!(
                "{}|{}|{}|{}",
                cluster.seed_keyword,
                cluster.dominant_intent,
                cluster.status,
                cluster.cluster_version
            ),
        ));
        projection_events.push(seo_qdrant_projection_event(
            &input.run_id,
            "seo_keyword_clusters_4",
            "keyword_cluster",
            &cluster.cluster_key,
            cluster_scope,
            &format!(
                "{} {} {} {}",
                cluster.seed_keyword, cluster.dominant_intent, cluster.status, cluster.cluster_key
            ),
            HashMap::from([
                ("seed_keyword".to_string(), cluster.seed_keyword.clone()),
                (
                    "dominant_intent".to_string(),
                    cluster.dominant_intent.clone(),
                ),
                ("status".to_string(), cluster.status.clone()),
            ]),
        ));
    }

    for pattern in &input.serp_patterns {
        if pattern.query.trim().is_empty() {
            continue;
        }
        sqlx::query(
            r#"
            INSERT INTO serp.opportunity_candidates
                (opportunity_key, scope_signature, query_batch_key, serp_pattern_key,
                 seed_keyword, dominant_intent, opportunity_score, recommended_action,
                 status, scoring_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7::numeric,
                    'create_or_refresh_page', 'candidate', 'seo_opportunity@1')
            ON CONFLICT (opportunity_key) DO UPDATE
            SET opportunity_score = EXCLUDED.opportunity_score,
                recommended_action = EXCLUDED.recommended_action,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(seo_domain::artifact_identity::seo_artifact_key(
            "opportunity",
            &[
                &pattern.scope_signature,
                &pattern.query,
                &pattern.dominant_intent,
                "seo_opportunity@1",
            ],
        ))
        .bind(&pattern.scope_signature)
        .bind(blank_as_none(&pattern.query_batch_key))
        .bind(blank_as_none(&pattern.serp_pattern_key))
        .bind(&pattern.query)
        .bind(&pattern.dominant_intent)
        .bind(pattern.reliability_score)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    for gap in &output.content_gaps {
        use seo_domain::artifact_identity::ContentGapDetector;
        non_empty(&gap.content_gap_key, "content_gap_key")?;
        non_empty(&gap.scope_signature, "gap scope_signature")?;
        non_empty(&gap.missing_topic, "gap missing_topic")?;
        let detector = ContentGapDetector::from_reason(&gap.reason_code)
            .ok_or_else(|| contract_violation("unknown content gap detector"))?;
        if !detector.valid_owner(&gap.page_node_key) {
            return Err(contract_violation("content gap detector has invalid owner"));
        }
        // The natural identity is the authority, including NULL page ownership.
        // A migrated legacy record retains its original projection identity.
        let canonical_key = detector.key(&gap.scope_signature, &gap.page_node_key, &gap.missing_topic);
        let persisted_key: String = sqlx::query_scalar(
            r#"
            INSERT INTO site.content_gaps
                (content_gap_key, scope_signature, page_node_key, missing_topic,
                 severity, detector_version, status, reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'graph_planning@1')
            ON CONFLICT (scope_signature, page_node_key, missing_topic, detector_version) DO UPDATE
            SET severity      = EXCLUDED.severity,
                reason_payload = EXCLUDED.reason_payload,
                reason_version = EXCLUDED.reason_version,
                status        = EXCLUDED.status,
                updated_at    = now()
            RETURNING content_gap_key
            "#,
        )
        .bind(&canonical_key)
        .bind(&gap.scope_signature)
        .bind(blank_as_none(&gap.page_node_key))
        .bind(&gap.missing_topic)
        .bind(&gap.severity)
        .bind(detector.version())
        .bind(&gap.status)
        .bind(Json(graph_reason_payload(
            &gap.reason_code,
            &gap.topic_keys,
            &gap.triple_refs,
            &gap.support_refs,
            gap.graph_confidence,
        )))
        .fetch_one(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event_with_revision(
            "content_gap",
            &persisted_key,
            &gap.scope_signature,
            &format!("{}|{}|{}", gap.missing_topic, gap.severity, gap.status),
        ));
        projection_events.push(seo_qdrant_projection_event(
            &input.run_id,
            "seo_content_gaps",
            "content_gap",
            &persisted_key,
            &gap.scope_signature,
            &format!("{} {} {}", gap.missing_topic, gap.severity, gap.status),
            HashMap::from([
                ("missing_topic".to_string(), gap.missing_topic.clone()),
                ("severity".to_string(), gap.severity.clone()),
                ("status".to_string(), gap.status.clone()),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(canonical)
}

pub async fn persist_ia_build_output(
    pool: &PgPool,
    input: &IaBuildInputPayload,
    output: &IaBuildOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    for blueprint in &output.page_blueprints {
        non_empty(&blueprint.blueprint_key, "blueprint_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_blueprints
                (blueprint_key, page_type_key, dominant_intent, scope_class,
                 blueprint_version, title_pattern, section_plan, derivation_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'seo_blueprint@1', $8)
            ON CONFLICT (blueprint_key) DO UPDATE
            SET page_type_key      = EXCLUDED.page_type_key,
                dominant_intent    = EXCLUDED.dominant_intent,
                scope_class        = EXCLUDED.scope_class,
                blueprint_version  = EXCLUDED.blueprint_version,
                title_pattern      = EXCLUDED.title_pattern,
                section_plan       = EXCLUDED.section_plan,
                status             = EXCLUDED.status,
                updated_at         = now()
            "#,
        )
        .bind(&blueprint.blueprint_key)
        .bind(&blueprint.page_type_key)
        .bind(&blueprint.dominant_intent)
        .bind(&blueprint.scope_class)
        .bind(blueprint.blueprint_version as i32)
        .bind(&blueprint.title_pattern)
        .bind(Json(json!({
            "required_sections": blueprint.required_sections.clone(),
            "canonical_url_family": blueprint.canonical_url_family.clone(),
        })))
        .bind(&blueprint.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event_with_revision(
            "page_blueprint",
            &blueprint.blueprint_key,
            "",
            &format!(
                "{}|{}|{}|{}|{:?}",
                blueprint.page_type_key,
                blueprint.dominant_intent,
                blueprint.scope_class,
                blueprint.title_pattern,
                blueprint.required_sections
            ),
        ));
        projection_events.push(seo_qdrant_projection_event(
            &input.run_id,
            "seo_page_blueprints",
            "page_blueprint",
            &blueprint.blueprint_key,
            "",
            &format!(
                "{} {} {} {}",
                blueprint.page_type_key,
                blueprint.dominant_intent,
                blueprint.scope_class,
                blueprint.status
            ),
            HashMap::from([
                ("page_type_key".to_string(), blueprint.page_type_key.clone()),
                (
                    "dominant_intent".to_string(),
                    blueprint.dominant_intent.clone(),
                ),
                ("scope_class".to_string(), blueprint.scope_class.clone()),
                ("status".to_string(), blueprint.status.clone()),
            ]),
        ));
    }

    // Published URLs are durable public identities; a new plan must not rename
    // a committed CMS page. Publication itself owns its revision and status.
    for page in &output.page_nodes {
        let published_path: Option<String> = sqlx::query_scalar(
            "SELECT canonical_url_path FROM site.cms_pages WHERE page_node_key=$1 AND published_at IS NOT NULL",
        )
        .bind(&page.page_node_key).fetch_optional(pool).await.map_err(classify_sqlx)?;
        if published_path.as_ref().is_some_and(|path| path != &page.canonical_url_path) {
            return Err(validation_failure("IA cannot rename a committed public page"));
        }
    }
    for page in &output.page_nodes {
        non_empty(&page.page_node_key, "page_node_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_nodes
                (page_node_key, scope_signature, keyword_cluster_key, blueprint_key,
                 page_type_key, dominant_intent, canonical_slug, canonical_url_path,
                 parent_page_node_key, hierarchy_depth, menu_group, breadcrumb_policy,
                 canonical_url_family, lifecycle_state, derivation_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    'seo_page_node@1')
            ON CONFLICT (page_node_key) DO UPDATE
            SET scope_signature     = EXCLUDED.scope_signature,
                keyword_cluster_key = EXCLUDED.keyword_cluster_key,
                blueprint_key       = EXCLUDED.blueprint_key,
                page_type_key       = EXCLUDED.page_type_key,
                dominant_intent     = EXCLUDED.dominant_intent,
                canonical_slug      = EXCLUDED.canonical_slug,
                canonical_url_path  = EXCLUDED.canonical_url_path,
                parent_page_node_key = EXCLUDED.parent_page_node_key,
                hierarchy_depth     = EXCLUDED.hierarchy_depth,
                menu_group          = EXCLUDED.menu_group,
                breadcrumb_policy   = EXCLUDED.breadcrumb_policy,
                canonical_url_family = EXCLUDED.canonical_url_family,
                lifecycle_state     = EXCLUDED.lifecycle_state,
                updated_at          = now()
            "#,
        )
        .bind(&page.page_node_key)
        .bind(&page.scope_signature)
        .bind(blank_as_none(&page.keyword_cluster_key))
        .bind(blank_as_none(&page.blueprint_key))
        .bind(&page.page_type_key)
        .bind(&page.dominant_intent)
        .bind(&page.canonical_slug)
        .bind(&page.canonical_url_path)
        .bind(blank_as_none(&page.parent_page_node_key))
        .bind(page.hierarchy_depth as i32)
        .bind(&page.menu_group)
        .bind(&page.breadcrumb_policy)
        .bind(&page.canonical_url_family)
        .bind(&page.lifecycle_state)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if !page.blueprint_key.trim().is_empty() {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'blueprint', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(seo_domain::artifact_identity::seo_artifact_key(
                "rebuild_dependency",
                &[&page.page_node_key, "blueprint", &page.blueprint_key],
            ))
            .bind(&page.page_node_key)
            .bind(&page.blueprint_key)
            .bind(Json(json!({
                "page_type_key": page.page_type_key,
                "dominant_intent": page.dominant_intent,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
        if !page.keyword_cluster_key.trim().is_empty() {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'keyword_cluster', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(seo_domain::artifact_identity::seo_artifact_key(
                "rebuild_dependency",
                &[&page.page_node_key, "keyword_cluster", &page.keyword_cluster_key],
            ))
            .bind(&page.page_node_key)
            .bind(&page.keyword_cluster_key)
            .bind(Json(json!({
                "page_type_key": page.page_type_key,
                "dominant_intent": page.dominant_intent,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;

            if let Some(seed_keyword) = sqlx::query_scalar::<_, String>(
                r#"
                SELECT seed_keyword
                FROM site.keyword_clusters
                WHERE cluster_key = $1
                LIMIT 1
                "#,
            )
            .bind(&page.keyword_cluster_key)
            .fetch_optional(pool)
            .await
            .map_err(classify_sqlx)?
            {
                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'serp_query', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(seo_domain::artifact_identity::seo_artifact_key(
                    "rebuild_dependency",
                    &[&page.page_node_key, "serp_query", &seed_keyword],
                ))
                .bind(&page.page_node_key)
                .bind(&seed_keyword)
                .bind(Json(json!({
                    "keyword_cluster_key": page.keyword_cluster_key,
                    "page_type_key": page.page_type_key,
                    "dominant_intent": page.dominant_intent,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
            }
        }

        let navigation_state_ref = format!(
            "{}|{}|{}",
            page.menu_group, page.breadcrumb_policy, page.canonical_url_family
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_rebuild_dependencies
                (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
            VALUES ($1, $2, 'navigation_state', $3, $4, 'active')
            ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
            SET reason_package = EXCLUDED.reason_package,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(seo_domain::artifact_identity::seo_artifact_key(
            "rebuild_dependency",
            &[&page.page_node_key, "navigation_state", &navigation_state_ref],
        ))
        .bind(&page.page_node_key)
        .bind(&navigation_state_ref)
        .bind(Json(json!({
            "menu_group": page.menu_group,
            "breadcrumb_policy": page.breadcrumb_policy,
            "canonical_url_family": page.canonical_url_family,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event_with_revision(
            "page_node",
            &page.page_node_key,
            &page.scope_signature,
            &format!(
                "{}|{}|{}|{}|{}|{}",
                page.canonical_url_path,
                page.page_type_key,
                page.dominant_intent,
                page.lifecycle_state,
                page.keyword_cluster_key,
                page.blueprint_key
            ),
        ));
        projection_events.push(seo_qdrant_projection_event(
            &input.run_id,
            "editorial_topics_4",
            "page_node",
            &page.page_node_key,
            &page.scope_signature,
            &format!(
                "{} {} {} {}",
                page.canonical_url_path,
                page.page_type_key,
                page.dominant_intent,
                page.lifecycle_state
            ),
            HashMap::from([
                (
                    "canonical_url_path".to_string(),
                    page.canonical_url_path.clone(),
                ),
                ("page_type_key".to_string(), page.page_type_key.clone()),
                ("dominant_intent".to_string(), page.dominant_intent.clone()),
                ("lifecycle_state".to_string(), page.lifecycle_state.clone()),
                ("menu_group".to_string(), page.menu_group.clone()),
                (
                    "canonical_url_family".to_string(),
                    page.canonical_url_family.clone(),
                ),
            ]),
        ));
    }

    for conflict in &output.cannibalization_conflicts {
        non_empty(&conflict.conflict_key, "conflict_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.cannibalization_conflicts
                (conflict_key, scope_signature, page_key_a, page_key_b,
                 conflict_reason, severity, detector_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'seo_cannibalization@1', $7)
            ON CONFLICT (conflict_key) DO UPDATE
            SET conflict_reason = EXCLUDED.conflict_reason,
                severity        = EXCLUDED.severity,
                status          = EXCLUDED.status,
                updated_at      = now()
            "#,
        )
        .bind(&conflict.conflict_key)
        .bind(&conflict.scope_signature)
        .bind(&conflict.page_key_a)
        .bind(&conflict.page_key_b)
        .bind(&conflict.conflict_reason)
        .bind(&conflict.severity)
        .bind(&conflict.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event_with_revision(
            "cannibalization_conflict",
            &conflict.conflict_key,
            &conflict.scope_signature,
            &format!(
                "{}|{}|{}",
                conflict.conflict_reason, conflict.severity, conflict.status
            ),
        ));
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

/// Retire only invalid, unpublished legacy plans within the reconciled scope.
/// Keep their identity for diagnosis; never delete pages, evidence or CMS revisions.
pub async fn retire_opaque_unpublished_plans(
    pool: &PgPool, run_id: &str, scope: &str, current_keys: &[String],
) -> Result<(), DomainError> {
    if scope.trim().is_empty() || current_keys.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT page_node_key, canonical_slug, canonical_url_path FROM site.page_nodes p
         WHERE scope_signature=$1 AND NOT(page_node_key=ANY($2))
           AND lifecycle_state IN ('candidate','planned','blocked','stale','needs_rebuild')
           AND NOT EXISTS (SELECT 1 FROM site.cms_pages c WHERE c.page_node_key=p.page_node_key)",
    ).bind(scope).bind(current_keys).fetch_all(pool).await.map_err(classify_sqlx)?;
    let mut events = Vec::new();
    for row in rows {
        let key: String = row.get("page_node_key");
        let slug: String = row.get("canonical_slug");
        let path: String = row.get("canonical_url_path");
        if !seo_domain::page_readiness::is_opaque_topic_identifier(&slug)
            && !path.split('/').any(seo_domain::page_readiness::is_opaque_topic_identifier) {
            continue;
        }
        let result = sqlx::query(
            "UPDATE site.page_nodes p SET lifecycle_state='deprecated', updated_at=now()
             WHERE page_node_key=$1 AND scope_signature=$2
               AND lifecycle_state IN ('candidate','planned','blocked','stale','needs_rebuild')
               AND NOT EXISTS (SELECT 1 FROM site.cms_pages c WHERE c.page_node_key=p.page_node_key)",
        ).bind(&key).bind(scope).execute(pool).await.map_err(classify_sqlx)?;
        if result.rows_affected() != 0 {
            events.push(seo_graph_projection_event_with_revision(
                "page_node", &key, scope, "deprecated:opaque_legacy_plan",
            ));
        }
    }
    emit_projection_events_for_run(pool, run_id, events).await?;
    Ok(())
}

pub async fn persist_link_recommend_output(
    pool: &PgPool,
    input: &LinkRecommendInputPayload,
    output: &LinkRecommendOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    for link in &output.link_recommendations {
        non_empty(&link.link_recommendation_key, "link_recommendation_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.link_recommendations
                (link_recommendation_key, scope_signature, source_page_key, target_page_key,
                 link_role, anchor_strategy, required_flag, score, scoring_version, status,
                 reason_payload, reason_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8::numeric, 'seo_link_score@1', $9, $10, 'graph_planning@1')
            ON CONFLICT (link_recommendation_key) DO UPDATE
            SET required_flag   = EXCLUDED.required_flag,
                score           = EXCLUDED.score,
                reason_payload  = EXCLUDED.reason_payload,
                reason_version  = EXCLUDED.reason_version,
                status          = EXCLUDED.status,
                updated_at      = now()
            "#,
        )
        .bind(&link.link_recommendation_key)
        .bind(&link.scope_signature)
        .bind(&link.source_page_key)
        .bind(&link.target_page_key)
        .bind(&link.link_role)
        .bind(&link.anchor_strategy)
        .bind(link.required_flag)
        .bind(link.score)
        .bind(&link.status)
        .bind(Json(graph_reason_payload(
            &link.reason_code,
            &link.topic_keys,
            &link.triple_refs,
            &link.support_refs,
            link.graph_confidence,
        )))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        if link.required_flag {
            sqlx::query(
                r#"
                INSERT INTO monitoring.seo_rebuild_dependencies
                    (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                VALUES ($1, $2, 'required_link', $3, $4, 'active')
                ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                SET reason_package = EXCLUDED.reason_package,
                    status = EXCLUDED.status,
                    updated_at = now()
                "#,
            )
            .bind(seo_domain::artifact_identity::seo_artifact_key(
                "rebuild_dependency",
                &[&link.source_page_key, "required_link", &link.target_page_key],
            ))
            .bind(&link.source_page_key)
            .bind(&link.target_page_key)
            .bind(Json(json!({
                "link_role": link.link_role,
                "anchor_strategy": link.anchor_strategy,
            })))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }

        projection_events.push(seo_graph_projection_event_with_revision(
            "link_recommendation",
            &link.link_recommendation_key,
            &link.scope_signature,
            &format!(
                "{}|{}|{}|{}|{}|{}|{}",
                link.source_page_key,
                link.target_page_key,
                link.link_role,
                link.anchor_strategy,
                link.required_flag,
                link.score,
                link.status
            ),
        ));
    }
    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}
