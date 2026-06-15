    async fn register_synthetic_site_input(
        repo: &SqlxSeoRuntimeRepository<'_>,
        run_id: String,
        query_batch_key: String,
    ) -> contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload {
        register_site_build_input(
            repo,
            &SeoSiteBuildRegistrationRequest {
                run_id,
                context_key: None,
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec!["spain tourist visa belarus".to_string()],
                query_batch_key: Some(query_batch_key),
                run_mode: Some(
                    run_mode_for_scenario(SeoScenarioKind::Full, false, false, true).to_string(),
                ),
            },
        )
        .await
        .unwrap()
    }

    async fn seed_synthetic_truth(pool: &sqlx::PgPool, context_key: &str, source_base_url: &str) {
        sqlx::query(
            r#"
            INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
            VALUES ('document.passport', 'document', 'Passport', 'active')
            ON CONFLICT (concept_key) DO UPDATE
            SET label_ru = EXCLUDED.label_ru,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO kb.sources (source_key, source_type, source_label, base_url, trust_level, status)
            VALUES ('source.official.synthetic', 'government', 'Synthetic official source', $1, 5, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(format!("{source_base_url}/official"))
        .execute(pool)
        .await
        .unwrap();
        seed_admissible_verified_rule(
            pool,
            context_key,
            "rule.synthetic.passport",
            "document_required",
            "document.passport",
            "document_required",
            serde_json::json!({
                "subtype": "travel_document",
            }),
            "source.official.synthetic",
            &format!("{source_base_url}/official/passport"),
            "Passport is required for the visa application.",
        )
        .await;
    }

    async fn seed_admissible_verified_rule(
        pool: &sqlx::PgPool,
        context_key: &str,
        rule_instance_id: &str,
        rule_type_key: &str,
        concept_key: &str,
        role_type: &str,
        params: serde_json::Value,
        source_key: &str,
        source_url: &str,
        fragment_text: &str,
    ) {
        let source_base_url = source_url.split('/').take(3).collect::<Vec<_>>().join("/");
        sqlx::query(
            r#"
            INSERT INTO kb.sources (source_key, source_type, source_label, base_url, trust_level, status)
            VALUES ($1, 'government', 'Synthetic support source', $2, 5, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(source_key)
        .bind(source_base_url)
        .execute(pool)
        .await
        .unwrap();
        let raw_html = format!("<html><body><main><p>{fragment_text}</p></main></body></html>");
        let snapshot_hash = format!("seed:{}:{}", rule_instance_id, raw_html.len());
        let page_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.pages
                (url, domain, dtype, status_code, final_url, title, raw_html, raw_html_bytes, content_hash, content, source_observation, processed)
            VALUES
                ($1, $2, 'government', 200, $1, 'Synthetic support source', $3, $4, $5, $6, $7, true)
            RETURNING id
            "#,
        )
        .bind(source_url)
        .bind(
            source_url
                .split('/')
                .nth(2)
                .unwrap_or("example.com")
                .to_string(),
        )
        .bind(&raw_html)
        .bind(raw_html.len() as i32)
        .bind(&snapshot_hash)
        .bind(Json(serde_json::json!({
            "markdown": fragment_text,
            "headings": [],
            "links": [],
        })))
        .bind(Json(serde_json::json!({
            "seeded_by": "integration_harness",
            "source_key": source_key,
        })))
        .fetch_one(pool)
        .await
        .unwrap();
        let section_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.sections
                (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
            VALUES
                ($1, 'Requirements', 1, 0, 'paragraph', $2, $3)
            RETURNING id
            "#,
        )
        .bind(page_id)
        .bind(fragment_text)
        .bind(format!("seed:{}:section:{}", rule_instance_id, fragment_text.len()))
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO verified.rule_instances
                (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status,
                 source_key, confidence, evidence_section_id, evidence_quote, span_start, span_end,
                 source_snapshot_hash, verification_method, adjudication_reason, publish_admissibility,
                 freshness_class, completeness_class, registry_version, prompt_version, model_version,
                 pipeline_version, effective_from)
            VALUES
                ($1, $2, $3, $4, $5, $6, 'verified',
                 $7, 0.99, $8, $9, 0, $10,
                 $11, 'synthetic_seed', 'integration_harness_seed', 'admissible',
                 'fresh', 'complete', 'registry@1', 'seed@1', 'none', 'integration_harness@1', current_date)
            ON CONFLICT (rule_instance_id) DO UPDATE
            SET params = EXCLUDED.params,
                status = EXCLUDED.status,
                source_key = EXCLUDED.source_key,
                confidence = EXCLUDED.confidence,
                evidence_section_id = EXCLUDED.evidence_section_id,
                evidence_quote = EXCLUDED.evidence_quote,
                span_start = EXCLUDED.span_start,
                span_end = EXCLUDED.span_end,
                source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                verification_method = EXCLUDED.verification_method,
                adjudication_reason = EXCLUDED.adjudication_reason,
                publish_admissibility = EXCLUDED.publish_admissibility,
                freshness_class = EXCLUDED.freshness_class,
                completeness_class = EXCLUDED.completeness_class,
                registry_version = EXCLUDED.registry_version,
                prompt_version = EXCLUDED.prompt_version,
                model_version = EXCLUDED.model_version,
                pipeline_version = EXCLUDED.pipeline_version,
                updated_at = now()
            "#,
        )
        .bind(rule_instance_id)
        .bind(context_key)
        .bind(rule_type_key)
        .bind(concept_key)
        .bind(role_type)
        .bind(Json(params))
        .bind(source_key)
        .bind(section_id)
        .bind(fragment_text)
        .bind(fragment_text.len() as i32)
        .bind(snapshot_hash)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_truth_context_and_concept(
        pool: &sqlx::PgPool,
        context_key: &str,
        concept_key: &str,
        concept_type: &str,
        label_ru: &str,
    ) {
        sqlx::query(
            r#"
            INSERT INTO kb.visa_contexts
                (context_key, country_code, visa_family, visa_subtype, citizenship_code, status)
            VALUES
                ($1, 'ES', 'tourist', NULL, 'BY', 'active')
            ON CONFLICT (context_key) DO UPDATE
            SET status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(context_key)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
            VALUES ($1, $2, $3, 'active')
            ON CONFLICT (concept_key) DO UPDATE
            SET concept_type = EXCLUDED.concept_type,
                label_ru = EXCLUDED.label_ru,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(concept_key)
        .bind(concept_type)
        .bind(label_ru)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_raw_page_with_section(
        pool: &sqlx::PgPool,
        url: &str,
        dtype: &str,
        heading_path: &str,
        content_md: &str,
    ) -> (i64, i64) {
        let raw_html = format!("<html><body><main><p>{content_md}</p></main></body></html>");
        let page_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.pages
                (url, domain, dtype, status_code, final_url, title, raw_html, raw_html_bytes, content_hash, content, source_observation, processed)
            VALUES
                ($1, $2, $3, 200, $1, 'Synthetic extraction source', $4, $5, $6, $7, $8, true)
            RETURNING id
            "#,
        )
        .bind(url)
        .bind(url.split('/').nth(2).unwrap_or("example.com").to_string())
        .bind(dtype)
        .bind(&raw_html)
        .bind(raw_html.len() as i32)
        .bind(format!("seed:raw-page:{}:{}", url, raw_html.len()))
        .bind(Json(serde_json::json!({
            "markdown": content_md,
            "headings": [heading_path],
            "links": [],
        })))
        .bind(Json(serde_json::json!({
            "seeded_by": "integration_harness_truth_runtime",
            "source_url": url,
        })))
        .fetch_one(pool)
        .await
        .unwrap();

        let section_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.sections
                (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
            VALUES
                ($1, $2, 1, 0, 'paragraph', $3, $4)
            RETURNING id
            "#,
        )
        .bind(page_id)
        .bind(heading_path)
        .bind(content_md)
        .bind(format!("seed:raw-section:{}:{}", url, content_md.len()))
        .fetch_one(pool)
        .await
        .unwrap();

        (page_id, section_id)
    }

    fn local_truth_response(payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-truth-test",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": payload.to_string()
                }
            }]
        })
    }

    fn local_truth_candidate_payload(
        raw_text: &str,
        role: &str,
        concept_canonical_key: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "candidates": [{
                "role": role,
                "concept_canonical_key": concept_canonical_key,
                "raw_mention": raw_text,
                "params": params,
                "scope": {},
                "severity": "mandatory",
                "applies_to_profiles": [],
                "exceptions_raw": "",
                "conditions_raw": "",
                "alternatives": [],
                "modality_raw": "",
                "derivation_type": "direct",
                "is_numeric": true,
                "is_range": false,
                "is_incomplete": false,
                "confidence": 0.94,
                "evidence_section_id": 0,
                "evidence_quote": raw_text,
                "span_start": 0,
                "span_end": raw_text.len(),
                "uncertainty_flags": []
            }]
        })
    }

    fn local_truth_env(endpoint: &str) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "SEO_TRUTH_LLM_LOCAL_ENDPOINT".to_string(),
                endpoint.to_string(),
            ),
            (
                "SEO_TRUTH_LLM_LOCAL_MODEL".to_string(),
                "stub-truth-extraction-model".to_string(),
            ),
        ])
    }

    async fn capture_semantic_snapshot(
        pool: &sqlx::PgPool,
        scope_signature: &str,
    ) -> SemanticSnapshot {
        let page_nodes = sqlx::query(
            r#"
            SELECT canonical_url_path, page_type_key, dominant_intent, lifecycle_state
            FROM site.page_nodes
            WHERE scope_signature = $1
            ORDER BY canonical_url_path
            "#,
        )
        .bind(scope_signature)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| SemanticPageNode {
            canonical_url_path: row.get("canonical_url_path"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            lifecycle_state: row.get("lifecycle_state"),
        })
        .collect::<Vec<_>>();

        let page_drafts = sqlx::query(
            r#"
            SELECT
                n.canonical_url_path,
                d.qa_verdict,
                d.body_markdown,
                d.truth_snapshot_ref,
                COALESCE(count(b.page_support_binding_key), 0)::bigint AS support_ref_count
            FROM site.page_drafts d
            JOIN site.page_nodes n ON n.page_node_key = d.page_node_key
            LEFT JOIN site.page_support_bindings b ON b.page_draft_key = d.page_draft_key
            WHERE n.scope_signature = $1
            GROUP BY n.canonical_url_path, d.qa_verdict, d.body_markdown, d.truth_snapshot_ref, d.updated_at
            ORDER BY n.canonical_url_path, d.updated_at
            "#,
        )
        .bind(scope_signature)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
            .map(|row| SemanticPageDraft {
                canonical_url_path: row.get("canonical_url_path"),
                qa_verdict: row.get("qa_verdict"),
                body_markdown: row.get("body_markdown"),
                has_truth_snapshot_ref: !row
                    .get::<String, _>("truth_snapshot_ref")
                    .trim()
                    .is_empty(),
                support_ref_count: row.get("support_ref_count"),
            })
        .collect::<Vec<_>>();

        SemanticSnapshot {
            page_nodes,
            page_drafts,
        }
    }

    fn normalized_cli_phase_multiset(
        result: &seo_application::scenario::SeoScenarioResult,
    ) -> BTreeMap<String, usize> {
        let allowed = [
            "serp_ingest",
            "crawl_sources",
            "raw_knowledge_ingestion",
            "serp_normalize",
            "opportunity_build",
            "ia_build",
            "link_recommend",
            "global_site_reconcile",
            "draft_assemble",
            "editorial_draft_generate",
            "draft_normalize",
            "content_contract_validate",
            "draft_qa",
            "rebuild_detect",
        ];
        let mut counts = BTreeMap::new();
        for report in &result.phase_reports {
            if allowed.contains(&report.phase.as_str()) {
                *counts.entry(report.phase.clone()).or_insert(0) += 1;
            }
        }
        for phase in ["raw_knowledge_ingestion", "global_site_reconcile"] {
            if let Some(count) = counts.get_mut(phase) {
                *count = 1;
            }
        }
        counts
    }

