pub async fn persist_finalize_publish_output(
    pool: &PgPool,
    input: &FinalizePublishInputPayload,
    planned: &FinalizePublishOutputPayload,
) -> Result<FinalizePublishOutputPayload, DomainError> {
    persist_finalize_publish_output_with_delivery(
        pool, input, planned, &CloudflareStaticDelivery,
    ).await
}

pub async fn persist_finalize_publish_output_with_delivery(
    pool: &PgPool,
    input: &FinalizePublishInputPayload,
    planned: &FinalizePublishOutputPayload,
    delivery: &dyn PublicReleaseDelivery,
) -> Result<FinalizePublishOutputPayload, DomainError> {
    let artifact = input.publish_artifact.as_ref().cloned().unwrap_or_default();
    let render_validation = input
        .render_validation
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let mut blocking_reasons = planned.blocking_reasons.clone();
    blocking_reasons.extend(render_validation.blocking_reasons.clone());
    let readiness = seo_domain::publishing::ReleaseReadiness::from_render_validation(
        &render_validation.verdict,
        &blocking_reasons,
    );
    let should_activate = readiness.can_activate(&planned.verdict);
    // Lock all publication effects for this public site, across workers and
    // processes. The transaction owns the lock until the outbox commits.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    if should_activate {
        let public_dir = std::env::var("SEO_STATIC_OUTPUT_DIR")
            .unwrap_or_else(|_| "app/rust/dist/static-site".to_string());
        sqlx::query("SELECT pg_advisory_xact_lock(21917, hashtext($1))")
            .bind(&public_dir)
            .execute(&mut *tx)
            .await
            .map_err(classify_sqlx)?;

        // A retry after an ambiguous COMMIT is not a new publish attempt.
        // Preserve the committed revision and its one logical Outbox event,
        // but never claim success without re-reading the exact public bytes.
        let committed: Option<(String, Json<serde_json::Value>)> = sqlx::query_as(
            r#"
            SELECT a.artifact_uri, a.manifest_json
            FROM site.cms_pages p
            JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
            JOIN site.publish_artifacts a ON a.revision_id = r.revision_id
            WHERE p.page_node_key = $1 AND p.current_revision_id = $2
              AND p.current_status = 'published' AND r.revision_status = 'published'
              AND a.artifact_key = $3 AND a.status = 'ready'
            FOR UPDATE OF p, r, a
            "#,
        )
        .bind(&input.page_node_key)
        .bind(&input.revision_id)
        .bind(&artifact.artifact_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        if let Some((uri, Json(persisted_meta))) = committed {
            let supplied: serde_json::Value = serde_json::from_str(&artifact.manifest_json)
                .map_err(|err| validation_failure(format!("retry artifact JSON: {err}")))?;
            if uri != artifact.artifact_uri || persisted_meta != supplied
                || supplied.get("run_id").and_then(|v| v.as_str()) != Some(input.run_id.as_str())
            {
                return Err(validation_failure("replayed publication artifact/run identity changed"));
            }
            verify_committed_release(&artifact, &input.page_node_key, &input.revision_id, delivery)
                .await.map_err(validation_failure)?;
            tx.commit().await.map_err(classify_sqlx)?;
            return Ok(FinalizePublishOutputPayload {
                verdict: "published".into(),
                current_status: "published".into(),
                blocking_reasons: Vec::new(),
                publish_artifact: Some(PublishArtifact { status: "ready".into(), ..artifact }),
            });
        }
    }

    let mut activation: Option<PublicationActivation> = None;
    if should_activate {
        // Render readiness is NOT publication. Activate an immutable,
        // approved release and prove real HTTPS bytes before DB commit.
        let allow_first_publication: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS (SELECT 1 FROM site.cms_pages WHERE published_at IS NOT NULL)",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        match promote_and_verify_public_release(
            &artifact,
            &input.page_node_key,
            &input.revision_id,
            allow_first_publication,
            &mut tx,
            delivery,
        )
        .await
        {
            Ok(receipt) => activation = Some(receipt),
            Err(reason) => {
                tracing::warn!(
                    page_node_key = %input.page_node_key,
                    revision_id = %input.revision_id,
                    reason = %reason,
                    "public release promotion/readback failed; refusing published state"
                );
                blocking_reasons.push("public_deployment_unverified".to_string());
            }
        }
    }
    blocking_reasons.sort();
    blocking_reasons.dedup();
    let outcome = seo_domain::publishing::PublicReleaseOutcome::after_readback(
        readiness,
        activation.is_some(),
    );
    let is_published = outcome.is_published();
    let final_verdict = outcome.verdict();

    // Database status, artifact, event, HITL, and outbox commit together.
    // On any SQL error the activation guard restores the prior public release.
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
    .bind(final_verdict)
    .bind(&input.revision_id)
    .execute(&mut *tx)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(
        r#"
        UPDATE site.cms_pages
        SET current_status = CASE
                WHEN $1 = 'blocked' AND published_at IS NOT NULL THEN 'published'
                ELSE $1 END,
            current_revision_id = CASE
                WHEN $1 = 'published' THEN $3
                ELSE current_revision_id END,
            rollback_revision_id = CASE
                WHEN $1 = 'published' AND current_status = 'published'
                     AND current_revision_id IS DISTINCT FROM $3
                THEN current_revision_id ELSE rollback_revision_id END,
            published_at = CASE WHEN $1 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END,
            updated_at = now()
        WHERE page_node_key = $2
        "#,
    )
    .bind(final_verdict)
    .bind(&input.page_node_key)
    .bind(&input.revision_id)
    .execute(&mut *tx)
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
    .bind(final_verdict)
    .bind(&input.page_node_key)
    .execute(&mut *tx)
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
        .bind(outcome.artifact_status())
        .bind(&artifact.artifact_key)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }

    let event_type = outcome.event_type();
    let event_key = seo_domain::artifact_identity::seo_artifact_key(
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
    .execute(&mut *tx)
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
    if is_published {
        // Resolve only the exact technical blocker now proven by the normal
        // finalizer. Historical draft/other-revision review tasks are unchanged.
        sqlx::query(
            r#"
            UPDATE site.seo_hitl_tasks
            SET queue_state = 'resolved', updated_at = now(),
                audit_log_payload = audit_log_payload || $1::jsonb
            WHERE page_node_key = $2 AND blocking_execution_key = $3
              AND task_type = 'publish_gate_blocker'
              AND blocking_step_name = 'publish_materialize'
              AND decision_payload->>'blocker' = 'public_deployment_unverified'
              AND queue_state IN ('open','in_review','reopened')
            "#,
        )
        .bind(Json(json!([{ "event": "public_deployment_verified", "revision_id": input.revision_id }])))
        .bind(&input.page_node_key)
        .bind(&input.revision_id)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }
    if !is_published {
        for blocker_reason in &blocking_reasons {
            let task_key = seo_domain::artifact_identity::seo_artifact_key(
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
            .execute(&mut *tx)
            .await
            .map_err(classify_sqlx)?;
        }
    }

    let _ = outbox_emit_many_in_tx(
        &mut tx,
        &[cms_event_outbox(&cms_event, &input.run_id)],
    )
    .await?;
    // The COMMIT acknowledgement can be lost even when PostgreSQL committed.
    // Do not roll back a publicly verified release based on an ambiguous
    // connection error or cancellation. Recovery reads the durable journal
    // and the authoritative DB state on the next worker startup.
    if let Some(receipt) = activation.as_mut() {
        receipt.defer_to_recovery();
    }
    tx.commit().await.map_err(classify_sqlx)?;
    if let Some(receipt) = activation.as_mut() {
        receipt.confirm();
    }

    Ok(FinalizePublishOutputPayload {
        verdict: final_verdict.to_string(),
        current_status: final_verdict.to_string(),
        blocking_reasons,
        publish_artifact: Some(PublishArtifact {
            status: outcome.artifact_status().to_string(),
            ..artifact
        }),
    })
}
