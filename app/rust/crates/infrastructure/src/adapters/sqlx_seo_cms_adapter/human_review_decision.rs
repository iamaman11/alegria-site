pub async fn apply_human_review_decision(
    pool: &PgPool,
    request: &CmsReviewDecisionRequest,
) -> Result<CmsReviewDecisionOutcome, DomainError> {
    let actor_role = request.actor_role.trim();
    if actor_role.is_empty() || actor_role == "seo_system" {
        return Err(validation_failure("human actor_role is required"));
    }

    // The live pointer intentionally stays on the published revision while a
    // replacement waits for review. Never let a page-key-only decision select
    // that published revision or silently choose an older review candidate.
    let row = sqlx::query(
        r#"
        SELECT r.revision_id
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.page_node_key = p.page_node_key
        WHERE p.page_node_key = $1
          AND r.revision_id = COALESCE($2, p.current_revision_id)
          AND r.revision_status = 'review_required'
        "#,
    )
    .bind(&request.page_node_key)
    .bind(request.revision_id.as_deref())
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| validation_failure(
        "exact review_required revision not found; specify --revision-id for a published page"
    ))?;
    let revision_id: String = row.get("revision_id");
    // Reject reviews not associated with a real canonical Temporal workflow
    // before changing any CMS decision or HITL row.
    let workflow_id = resolve_seo_workflow_id(pool, &request.page_node_key, &revision_id)
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
            &request.decision,
        ],
    );
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
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
    .bind(&request.decision)
    .bind(&request.reason)
    .bind(Json(json!({
        "reason": request.reason.clone(),
        "source": request.source.clone(),
    })))
    .execute(&mut *tx)
    .await
    .map_err(classify_sqlx)?;

    let queue_state = match request.decision.as_str() {
        "approved" => "resolved",
        "blocked" => "reopened",
        "reopened" => "reopened",
        other => return Err(validation_failure(format!("unsupported decision: {other}"))),
    };
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

