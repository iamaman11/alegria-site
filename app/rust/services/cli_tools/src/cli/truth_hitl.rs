async fn truth_hitl_reconcile(database_url: Option<String>, context_key: &str) -> Result<i32> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let count = sqlx_hitl_adapter::reconcile_truth_candidate_reviews(
        &pool,
        context_key,
        "operator_reconcile",
    )
    .await?;
    println!("TRUTH_HITL_RECONCILE inserted={count} context={context_key}");
    Ok(0)
}

async fn truth_hitl_list(
    database_url: Option<String>,
    context_key: &str,
    limit: i64,
) -> Result<i32> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let tasks = sqlx_hitl_adapter::list_truth_candidate_reviews(&pool, context_key, limit).await?;
    println!("{}", serde_json::to_string_pretty(&tasks)?);
    Ok(0)
}

async fn truth_hitl_show(database_url: Option<String>, task_key: &str) -> Result<i32> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let task = sqlx_hitl_adapter::show_truth_candidate_review(&pool, task_key).await?;
    println!("{}", serde_json::to_string_pretty(&task)?);
    Ok(0)
}

async fn truth_hitl_decide(
    database_url: Option<String>,
    task_key: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> Result<i32> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let state = sqlx_hitl_adapter::decide_truth_candidate_review(
        &pool,
        task_key,
        actor_role,
        decision,
        reason,
    )
    .await?;
    println!(
        "TRUTH_HITL_REVIEW state={state} task={task_key} independent_recheck=REQUIRED verified=NO"
    );
    Ok(0)
}
