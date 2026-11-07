pub async fn persist_finalize_publish_output(
    pool: &PgPool,
    input: &FinalizePublishInputPayload,
    planned: &FinalizePublishOutputPayload,
) -> Result<FinalizePublishOutputPayload, DomainError> {
    let artifact = input.publish_artifact.as_ref().cloned().unwrap_or_default();
    let render_validation = input
        .render_validation
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let mut blocking_reasons = planned.blocking_reasons.clone();
    blocking_reasons.extend(render_validation.blocking_reasons.clone());
    let mut is_published = planned.verdict == "published";
    if is_published {
        // Render readiness is NOT publication. Only promote an immutable,
        // human-approved release and verify its bytes over public HTTPS first.
        if let Err(reason) =
            promote_and_verify_public_release(&artifact, &input.page_node_key, &input.revision_id)
                .await
        {
            tracing::warn!(
                page_node_key = %input.page_node_key,
                revision_id = %input.revision_id,
                reason = %reason,
                "public release promotion/readback failed; refusing published state"
            );
            blocking_reasons.push("public_deployment_unverified".to_string());
            is_published = false;
        }
    }
    blocking_reasons.sort();
    blocking_reasons.dedup();
    let final_verdict = if is_published { "published" } else { "blocked" };

    sqlx::query(
        r#"
        UPDATE site.cms_page_revisions
        SET revision_status = CASE
                WHEN revision_status = 'published' AND $1 = 'blocked'
                THEN revision_status ELSE $1 END,
            updated_at = now()
        WHERE revision_id = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.revision_id)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.cms_pages
        SET current_status = CASE
                WHEN $1 = 'blocked' AND published_at IS NOT NULL THEN 'published'
                ELSE $1 END,
            published_at = CASE WHEN $1 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = CASE
                WHEN $1 = 'blocked' AND EXISTS (
                    SELECT 1 FROM site.cms_pages cp
                    WHERE cp.page_node_key = $2 AND cp.published_at IS NOT NULL
                ) THEN 'published'
                ELSE $1 END,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(if is_published { "published" } else { "blocked" })
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    if !artifact.artifact_key.trim().is_empty() {
        sqlx::query(
            r#"
            UPDATE site.publish_artifacts
            SET status = $1,
                updated_at = now()
            WHERE artifact_key = $2
            "#,
        )
        .bind(if is_published { "ready" } else { "blocked" })
        .bind(&artifact.artifact_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    let event_type = if is_published {
        "seo_page_published"
    } else {
        "seo_page_publish_blocked"
    };
    let event_key = primitives::seo::seo_artifact_key(
        "cms_event",
        &[
            &input.page_node_key,
            &input.revision_id,
            event_type,
            "publish_finalize@1",
        ],
    );
    sqlx::query(
        r#"
        INSERT INTO site.cms_publish_events
            (event_key, event_type, page_node_key, scope_signature, revision_id,
             actor_role, causal_step, payload_version, event_payload)
        SELECT $1, $2, page_node_key, scope_signature, $3, 'seo_system',
               'publish_materialize', 'seo_cms_event@1', $4
        FROM site.cms_pages
        WHERE page_node_key = $5
        ON CONFLICT (event_key) DO UPDATE
        SET event_payload = EXCLUDED.event_payload
        "#,
    )
    .bind(&event_key)
    .bind(event_type)
    .bind(&input.revision_id)
    .bind(Json(json!({
        "verdict": final_verdict,
        "blocking_reasons": blocking_reasons,
    })))
    .bind(&input.page_node_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let cms_event = SeoCmsEventPayload {
        event_key,
        event_type: event_type.to_string(),
        page_node_key: input.page_node_key.clone(),
        scope_signature: String::new(),
        revision_id: input.revision_id.clone(),
        actor_role: "seo_system".to_string(),
        causal_step: "publish_materialize".to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        payload_version: "seo_cms_event@1".to_string(),
        metadata: HashMap::from([("verdict".to_string(), final_verdict.to_string())]),
    };
    let _ = outbox_emit_many(pool, &[cms_event_outbox(&cms_event, &input.run_id)]).await?;

    if !is_published {
        for blocker_reason in &blocking_reasons {
            let task_key = primitives::seo::seo_artifact_key(
                "seo_hitl_task",
                &[
                    &input.page_node_key,
                    &input.revision_id,
                    blocker_reason,
                    "publish_finalize@1",
                ],
            );
            sqlx::query(
                r#"
                INSERT INTO site.seo_hitl_tasks
                    (task_key, task_type, queue_state, first_owner_role, current_owner_role,
                     page_node_key, scope_signature, blocking_step_name, blocking_execution_key,
                     severity, decision_payload, audit_log_payload)
                SELECT $1, $2, 'open', 'seo_editor', 'seo_editor', page_node_key, scope_signature,
                       'publish_materialize', $3, 'blocking', $4, $5
                FROM site.cms_pages
                WHERE page_node_key = $6
                ON CONFLICT (task_key) DO UPDATE
                SET queue_state = 'open',
                    updated_at = now()
                "#,
            )
            .bind(&task_key)
            .bind(hitl_task_type(blocker_reason))
            .bind(&input.revision_id)
            .bind(Json(json!({ "blocker": blocker_reason })))
            .bind(Json(
                json!([{ "event": "publish_blocked", "blocker": blocker_reason }]),
            ))
            .bind(&input.page_node_key)
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
    }

    Ok(FinalizePublishOutputPayload {
        verdict: final_verdict.to_string(),
        current_status: if is_published {
            "published".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons,
        publish_artifact: Some(PublishArtifact {
            status: if is_published {
                "ready".to_string()
            } else {
                "blocked".to_string()
            },
            ..artifact
        }),
    })
}
