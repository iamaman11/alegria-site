pub async fn apply_human_review_decision(
    pool: &PgPool,
    request: &CmsReviewDecisionRequest,
) -> Result<CmsReviewDecisionOutcome, DomainError> {
    let actor_role = request.actor_role.trim();
    if actor_role.is_empty() || actor_role == "seo_system" {
        return Err(validation_failure("human actor_role is required"));
    }

    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.current_revision_id AS revision_id
        FROM site.cms_pages p
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(&request.page_node_key)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)?;
    let revision_id: String = row.get("revision_id");
    let decision_key = primitives::seo::seo_artifact_key(
        "cms_approval_decision",
        &[
            &request.page_node_key,
            &revision_id,
            actor_role,
            &request.decision,
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
    .bind(&request.decision)
    .bind(&request.reason)
    .bind(Json(json!({
        "reason": request.reason.clone(),
        "source": request.source.clone(),
    })))
    .execute(pool)
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
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let workflow_id = resolve_seo_workflow_id(pool, &request.page_node_key, &revision_id)
        .await?
        .ok_or_else(|| {
            validation_failure("seo workflow run_id not found for current review revision")
        })?;

    Ok(CmsReviewDecisionOutcome {
        decision_key,
        revision_id,
        workflow_id,
    })
}

