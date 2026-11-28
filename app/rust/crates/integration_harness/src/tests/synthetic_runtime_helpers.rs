    #[cfg(feature = "e2e")]
    use integration_data_fixtures::{seed_raw_page_with_section, seed_truth_context_and_concept};

    // The positive full-site fixture must cover the same required factual
    // roles as the production blueprints. These are disposable DB test facts,
    // not a fallback or a source of production truth.
    #[cfg(feature = "e2e")]
    async fn seed_synthetic_truth(pool: &sqlx::PgPool, context_key: &str, source_base_url: &str) {
        let facts = [
            ("passport", "document", "document_required", serde_json::json!({"subtype": "travel_document"}), "Passport is required for the visa application."),
            ("fee", "fee", "fee_item", serde_json::json!({"amount": 80, "currency": "EUR"}), "The visa fee is 80 EUR."),
            ("eligibility", "rule", "eligibility_rule", serde_json::json!({"profile": "standard_tourist_applicant"}), "Standard tourist applicants are eligible to apply."),
            ("submit", "process", "step", serde_json::json!({"action": "submit_application", "order": 1}), "Application step 1: submit the application."),
            ("timing", "timeline", "timeline_item", serde_json::json!({"days": 15}), "Processing takes 15 days."),
            ("location", "location", "where_to_apply", serde_json::json!({"channel": "visa_centre"}), "Applications are submitted at the visa centre."),
        ];
        for (key, concept_type, role, params, quote) in facts {
            let concept = format!("synthetic.{key}");
            seed_truth_context_and_concept(pool, context_key, &concept, concept_type, key).await;
            seed_admissible_verified_rule(
                pool,
                context_key,
                &format!("rule.synthetic.{key}"),
                role,
                &concept,
                role,
                params,
                "source.official.synthetic",
                &format!("{source_base_url}/official/spain-tourist-visa"),
                quote,
            )
            .await;
        }
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
