pub async fn persist_cms_publish_output(
    pool: &PgPool,
    input: &CmsPublishInputPayload,
    planned: &CmsPublishOutputPayload,
) -> Result<CmsPublishOutputPayload, DomainError> {
    if planned.page_draft_key.trim().is_empty() {
        return Err(validation_failure(
            "cms_publish persistence requires page_draft_key",
        ));
    }
    let Some(row) = load_publish_draft(pool, &planned.page_draft_key).await? else {
        return Err(validation_failure(format!(
            "page_draft not found for cms_publish: {}",
            planned.page_draft_key
        )));
    };

    let mut blockers = planned.blocking_reasons.clone();
    if row.qa_verdict != "publish_ready" {
        blockers.push("failing_draft_qa_verdict".to_string());
    }
    if row.truth_snapshot_ref.trim().is_empty() {
        blockers.push("missing_traceability_manifest".to_string());
    }
    if row.locale_code.trim().is_empty() || row.locale_code == "und" {
        blockers.push("missing_scope_locale".to_string());
    }
    if count_active_cannibalization(pool, &row.page_node_key).await? > 0 {
        blockers.push("active_cannibalization_conflict".to_string());
    }
    let resolved_links = resolve_required_internal_links(pool, &row.page_node_key).await?;
    // Require a structural link plan, but do not block a root/hub revision
    // merely because its first child pages have not been published yet.
    if count_required_link_recommendations(pool, &row.page_node_key).await? == 0 {
        blockers.push("required_link_obligations_unsatisfied".to_string());
    }
    if count_unpublished_mandatory_links(pool, &row.page_node_key).await? > 0 {
        blockers.push("required_internal_target_not_published".to_string());
    }
    if count_canonical_conflicts(pool, &row).await? > 0 {
        blockers.push("canonical_url_conflict".to_string());
    }

    let revision_id = if planned.revision_id.trim().is_empty() {
        seo_domain::artifact_identity::seo_artifact_key(
            "cms_revision",
            &[
                &row.page_node_key,
                &row.page_draft_key,
                &row.draft_revision.to_string(),
                "seo_cms_publish@1",
            ],
        )
    } else {
        planned.revision_id.clone()
    };
    let cms_document_id = if planned.cms_document_id.trim().is_empty() {
        seo_domain::artifact_identity::seo_artifact_key("cms_document", &[&row.page_node_key])
    } else {
        planned.cms_document_id.clone()
    };
    let valid_human_approval = if input.publish_mode == "approved_publish" {
        let persisted =
            load_latest_approval_decision(pool, &row.page_node_key, &revision_id).await?;
        input
            .approval_decision
            .as_ref()
            .zip(persisted.as_ref())
            .is_some_and(|(supplied, saved)| {
                supplied.decision_key == saved.decision_key
                    && supplied.actor_role == saved.actor_role
                    && approval_decision_valid(supplied, &row, &revision_id)
                    && approval_decision_valid(saved, &row, &revision_id)
            })
    } else {
        false
    };
    // CMS persists the link plan; the static builder is the single owner of
    // displaying links using the current release's page titles and routes.
    let content_blocks = row
        .traceability_manifest
        .get("content_blocks")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let required_link_payload = resolved_links
        .iter()
        .map(|link| json!({
            "target_page_key": link.target_page_key,
            "canonical_url_path": link.canonical_url_path,
            "label": link.title,
            "link_role": link.link_role,
        }))
        .collect::<Vec<_>>();
    if input.publish_mode == "approved_publish" && !valid_human_approval {
        blockers.push("missing_persisted_human_approval".to_string());
    }
    blockers.sort();
    blockers.dedup();
    let (current_status, page_lifecycle_state, revision_status, event_type, verdict) =
        if blockers.is_empty() {
            match input.publish_mode.as_str() {
                "approved_publish" => (
                    "approved",
                    "approved",
                    "approved",
                    "seo_page_approved",
                    "approved",
                ),
                _ => (
                    "review_required",
                    "review_required",
                    "review_required",
                    "seo_page_review_requested",
                    "review_requested",
                ),
            }
        } else {
            (
                "blocked",
                "blocked",
                "blocked",
                "seo_page_publish_blocked",
                "blocked",
            )
        };

    let revision_write = sqlx::query(
        r#"
        INSERT INTO site.cms_page_revisions
            (revision_id, page_node_key, page_draft_key, page_brief_key, title,
             meta_description, h1, body_payload, faq_payload, schema_markup_payload,
             required_link_payload, traceability_manifest, blueprint_key, qa_report_key,
             freshness_class, review_owner_role, revision_reason, revision_status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '[]'::jsonb, $9, $10, $11,
                $12, $13, 'fresh', 'seo_editor', 'seo_publish_gate', $14)
        ON CONFLICT (revision_id) DO UPDATE
        SET revision_status = CASE
                WHEN site.cms_page_revisions.revision_status = 'published'
                THEN site.cms_page_revisions.revision_status
                ELSE EXCLUDED.revision_status END,
            updated_at = CASE
                WHEN site.cms_page_revisions.revision_status = 'published'
                THEN site.cms_page_revisions.updated_at
                ELSE now() END
        WHERE site.cms_page_revisions.page_draft_key = EXCLUDED.page_draft_key
          AND site.cms_page_revisions.page_node_key = EXCLUDED.page_node_key
          AND site.cms_page_revisions.page_brief_key = EXCLUDED.page_brief_key
          AND site.cms_page_revisions.title = EXCLUDED.title
          AND site.cms_page_revisions.meta_description = EXCLUDED.meta_description
          AND site.cms_page_revisions.h1 = EXCLUDED.h1
          AND site.cms_page_revisions.body_payload = EXCLUDED.body_payload
          AND site.cms_page_revisions.schema_markup_payload = EXCLUDED.schema_markup_payload
          AND site.cms_page_revisions.required_link_payload = EXCLUDED.required_link_payload
          AND site.cms_page_revisions.traceability_manifest = EXCLUDED.traceability_manifest
        "#,
    )
    .bind(&revision_id)
    .bind(&row.page_node_key)
    .bind(&row.page_draft_key)
    .bind(&row.page_brief_key)
    .bind(&row.title)
    .bind(&row.meta_description)
    .bind(&row.title)
    .bind(Json(json!({
        "markdown": row.body_markdown.clone(),
        "content_blocks": content_blocks,
        "claim_ledger": row
            .traceability_manifest
            .get("claim_ledger")
            .cloned()
            .unwrap_or_else(|| json!([])),
    })))
    .bind(Json(json!({
        "@type": "Article",
        "headline": row.title.clone(),
        "url": row.canonical_url_path.clone(),
    })))
    .bind(Json(required_link_payload))
    .bind(Json(json!({
        "truth_snapshot_ref": row.truth_snapshot_ref.clone(),
        "page_draft_key": row.page_draft_key.clone(),
        "traceability_manifest": row.traceability_manifest.clone(),
    })))
    .bind(&row.blueprint_key)
    .bind(format!("draft_qa:{}", row.page_draft_key))
    .bind(revision_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if revision_write.rows_affected() != 1 {
        return Err(validation_failure("immutable CMS revision content changed"));
    }

    sqlx::query(
        r#"
        INSERT INTO site.cms_pages
            (page_node_key, scope_signature, canonical_url_path, locale_code,
             page_type_key, dominant_intent_key, cms_document_id,
             current_revision_id, current_status, published_at)
        VALUES ($1, $2, $3, $9, $4, $5, $6, $7, $8,
                CASE WHEN $8 = 'published' THEN now() ELSE NULL END)
        ON CONFLICT (page_node_key) DO UPDATE
        SET canonical_url_path = CASE
                WHEN site.cms_pages.current_status = 'published'
                THEN site.cms_pages.canonical_url_path
                ELSE EXCLUDED.canonical_url_path END,
            locale_code = CASE
                WHEN site.cms_pages.current_status = 'published'
                THEN site.cms_pages.locale_code
                ELSE EXCLUDED.locale_code END,
            -- The newly approved revision is a candidate, NOT yet the live one.
            current_revision_id = CASE
                WHEN site.cms_pages.current_status = 'published'
                THEN site.cms_pages.current_revision_id
                ELSE EXCLUDED.current_revision_id END,
            current_status = CASE
                WHEN site.cms_pages.current_status = 'published'
                THEN 'published' ELSE EXCLUDED.current_status END,
            published_at = CASE
                WHEN EXCLUDED.current_status = 'published'
                THEN COALESCE(site.cms_pages.published_at, now())
                ELSE site.cms_pages.published_at
            END,
            updated_at = now()
        "#,
    )
    .bind(&row.page_node_key)
    .bind(&row.scope_signature)
    .bind(&row.canonical_url_path)
    .bind(&row.page_type_key)
    .bind(&row.dominant_intent)
    .bind(&cms_document_id)
    .bind(&revision_id)
    .bind(current_status)
    .bind(&row.locale_code)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = CASE
                WHEN lifecycle_state = 'published' THEN 'published'
                ELSE $1 END,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(page_lifecycle_state)
    .bind(&row.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let event_key = seo_domain::artifact_identity::seo_artifact_key(
        "cms_event",
        &[
            &row.page_node_key,
            &revision_id,
            event_type,
            "seo_cms_event@1",
        ],
    );
    let event_payload = json!({
        "verdict": verdict,
        "blocking_reasons": blockers.clone(),
        "canonical_url_path": row.canonical_url_path.clone(),
        "canonical_slug": row.canonical_slug.clone(),
        "run_id": input.run_id.clone(),
    });
    sqlx::query(
        r#"
        INSERT INTO site.cms_publish_events
            (event_key, event_type, page_node_key, scope_signature, revision_id,
             actor_role, causal_step, payload_version, event_payload)
        VALUES ($1, $2, $3, $4, $5, $6, 'cms_publish', 'seo_cms_event@1', $7)
        ON CONFLICT (event_key) DO UPDATE
        SET event_payload = EXCLUDED.event_payload
        "#,
    )
    .bind(&event_key)
    .bind(event_type)
    .bind(&row.page_node_key)
    .bind(&row.scope_signature)
    .bind(&revision_id)
    .bind(if input.actor_role.trim().is_empty() {
        "seo_system"
    } else {
        input.actor_role.as_str()
    })
    .bind(Json(event_payload.clone()))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if !blockers.is_empty() {
        for blocker in &blockers {
            let task_key = seo_domain::artifact_identity::seo_artifact_key(
                "seo_hitl_task",
                &[&row.page_node_key, &revision_id, blocker, "seo_hitl@1"],
            );
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     page_node_key, scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                VALUES ($1, $2, 'open', 'seo_editor', 'seo_editor', $3, $4,
                        'cms_publish', $5, 'blocking', $6, $7)
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    decision_payload = EXCLUDED.decision_payload,
                    updated_at = now()
                "#,
            )
            .bind(&task_key)
            .bind(hitl_task_type(blocker))
            .bind(&row.page_node_key)
            .bind(&row.scope_signature)
            .bind(&revision_id)
            .bind(Json(
                json!({ "blocker": blocker, "revision_id": revision_id }),
            ))
            .bind(Json(json!([{
                "event": "publish_blocked",
                "blocker": blocker,
            }])))
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
    }

    let cms_event = SeoCmsEventPayload {
        event_key,
        event_type: event_type.to_string(),
        page_node_key: row.page_node_key.clone(),
        scope_signature: row.scope_signature.clone(),
        revision_id: revision_id.clone(),
        actor_role: if input.actor_role.trim().is_empty() {
            "seo_system".to_string()
        } else {
            input.actor_role.clone()
        },
        causal_step: "cms_publish".to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        payload_version: "seo_cms_event@1".to_string(),
        metadata: HashMap::from([
            ("verdict".to_string(), verdict.to_string()),
            (
                "canonical_url_path".to_string(),
                row.canonical_url_path.clone(),
            ),
            ("cms_document_id".to_string(), cms_document_id.clone()),
        ]),
    };
    let emitted = outbox_emit_many(pool, &[cms_event_outbox(&cms_event, &input.run_id)]).await?;
    let publish_artifact = if verdict == "approved" {
        let manifest = json!({
            "page_node_key": row.page_node_key.clone(),
            "revision_id": revision_id.clone(),
            "canonical_url_path": row.canonical_url_path.clone(),
            "cms_document_id": cms_document_id.clone(),
            "content_contract": "headless_cms_blocks@1",
        });
        let artifact = PublishArtifact {
            artifact_key: seo_domain::artifact_identity::seo_artifact_key(
                "publish_artifact",
                &[&row.page_node_key, &revision_id, "headless_snapshot"],
            ),
            page_node_key: row.page_node_key.clone(),
            revision_id: revision_id.clone(),
            artifact_type: "headless_snapshot".to_string(),
            artifact_uri: format!(
                "cms://pages/{}/revisions/{}",
                row.page_node_key, revision_id
            ),
            manifest_json: manifest.to_string(),
            status: "pending_materialization".to_string(),
        };
        sqlx::query(
            r#"
            INSERT INTO site.publish_artifacts
                (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                 manifest_json, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (artifact_key) DO UPDATE
            SET artifact_uri = EXCLUDED.artifact_uri,
                manifest_json = EXCLUDED.manifest_json,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(&artifact.artifact_key)
        .bind(&artifact.page_node_key)
        .bind(&artifact.revision_id)
        .bind(&artifact.artifact_type)
        .bind(&artifact.artifact_uri)
        .bind(Json(manifest))
        .bind(&artifact.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        Some(artifact)
    } else {
        None
    };

    Ok(CmsPublishOutputPayload {
        page_node_key: row.page_node_key,
        page_draft_key: row.page_draft_key,
        revision_id,
        cms_document_id,
        verdict: verdict.to_string(),
        blocking_reasons: blockers,
        outbox_emitted: emitted as u32,
        publish_artifact,
    })
}
