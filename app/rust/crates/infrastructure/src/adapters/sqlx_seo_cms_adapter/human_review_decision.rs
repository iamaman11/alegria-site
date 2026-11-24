pub async fn apply_human_review_decision(
    pool: &PgPool,
    request: &CmsReviewDecisionRequest,
) -> Result<CmsReviewDecisionOutcome, DomainError> {
    seo_domain::review_decision::validate_human_reviewer(
        &request.page_node_key,
        &request.revision_id,
        &request.actor_role,
    )?;
    let actor_role = request.actor_role.trim();
    let decision = request.decision.as_str();

    // The live pointer intentionally stays on the published revision while a
    // replacement waits for review. Always approve the explicit candidate;
    // deriving it from the live pointer would review the wrong revision.
    // Validate and lock the candidate in the same transaction that records
    // approval. A concurrent publish/reject must not invalidate this check.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let row = sqlx::query(
        r#"
        SELECT r.revision_id
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.page_node_key = p.page_node_key
        WHERE p.page_node_key = $1
          AND r.revision_id = $2
          AND r.revision_status = 'review_required'
        FOR UPDATE OF r
        "#,
    )
    .bind(&request.page_node_key)
    .bind(&request.revision_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure("exact review_required revision not found"))?;
    let revision_id: String = row.get("revision_id");
    // Reject reviews not associated with a real canonical Temporal workflow
    // before changing any CMS decision or HITL row.
    let workflow_id = resolve_seo_workflow_id(&mut *tx, &request.page_node_key, &revision_id)
        .await?
        .ok_or_else(|| {
            validation_failure("seo workflow run_id not found for current review revision")
        })?;
    let decision_key = seo_domain::artifact_identity::seo_artifact_key(
        "cms_approval_decision",
        &[
            &request.page_node_key,
            &revision_id,
            actor_role,
            decision,
        ],
    );
    sqlx::query(
        r#"
        INSERT INTO site.cms_approval_decisions
            (decision_key, page_node_key, revision_id, actor_role, decision, reason,
             decision_payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (decision_key) DO UPDATE
        SET decision = EXCLUDED.decision,
            reason = EXCLUDED.reason,
            decision_payload = EXCLUDED.decision_payload
        "#,
    )
    .bind(&decision_key)
    .bind(&request.page_node_key)
    .bind(&revision_id)
    .bind(actor_role)
    .bind(decision)
    .bind(&request.reason)
    .bind(Json(json!({
        "reason": request.reason.clone(),
        "source": request.source.clone(),
    })))
    .execute(&mut *tx)
    .await
    .map_err(classify_sqlx)?;

    let queue_state = request.decision.hitl_queue_state();
    sqlx::query(
        r#"
        UPDATE site.seo_hitl_tasks
        SET queue_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
          AND blocking_execution_key = $3
          AND queue_state IN ('open','in_review','reopened')
        "#,
    )
    .bind(queue_state)
    .bind(&request.page_node_key)
    .bind(&revision_id)
    .execute(&mut *tx)
    .await
    .map_err(classify_sqlx)?;


    tx.commit().await.map_err(classify_sqlx)?;

    Ok(CmsReviewDecisionOutcome {
        decision_key,
        revision_id,
        workflow_id,
    })
}

