async fn cms_review_decision(
    database_url: Option<String>,
    page_node_key: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> Result<(String, String, String)> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let workflow = TemporalSeoWorkflowControlAdapter::new(
        default_temporal_url(),
        format!("alegria-cli-tools@{}", std::process::id()),
        default_temporal_namespace(),
    );
    let result = apply_human_review_decision(
        &repo,
        &workflow,
        &ApplyHumanReviewDecisionInput {
            page_node_key: page_node_key.to_string(),
            actor_role: actor_role.to_string(),
            decision: decision.to_string(),
            reason: reason.to_string(),
            source: "cli_tools_headless_cms@1".to_string(),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok((result.decision_key, result.revision_id, result.workflow_id))
}

