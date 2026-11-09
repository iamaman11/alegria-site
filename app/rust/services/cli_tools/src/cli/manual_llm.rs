use infrastructure::adapters::manual_llm_queue::{self, ManualLlmSubmission};

async fn llm_pool(database_url: Option<String>) -> Result<sqlx::PgPool> {
    let url = database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .context("DATABASE_URL or --database-url is required")?;
    connect_pg(&url).await
}

async fn llm_next(database_url: Option<String>) -> Result<i32> {
    let pool = llm_pool(database_url).await?;
    match manual_llm_queue::next_editorial(&pool).await? {
        Some(job) => println!("{}", serde_json::to_string_pretty(&job)?),
        None => println!("NO_PENDING_LLM_JOBS"),
    }
    Ok(0)
}

async fn llm_submit(
    database_url: Option<String>,
    response_file: &str,
    model_key: &str,
    temporal_url: Option<String>,
    namespace: &str,
) -> Result<i32> {
    let raw = fs::read_to_string(response_file)
        .with_context(|| format!("read LLM submission from {response_file}"))?;
    let submission: ManualLlmSubmission = serde_json::from_str(&raw)
        .with_context(|| format!("decode typed LLM submission from {response_file}"))?;
    let pool = llm_pool(database_url).await?;
    let run_id = manual_llm_queue::submit_editorial(&pool, &submission, model_key).await?;
    // Write-first: if Temporal is unreachable, the completed job remains in
    // PostgreSQL. Retrying this exact submission is idempotent and re-signals.
    let temporal = TemporalSeoWorkflowControlAdapter::new(
        temporal_url
            .or_else(|| env::var("TEMPORAL_URL").ok())
            .unwrap_or_else(|| "http://localhost:7233".to_string()),
        "alegria-cli-manual-llm".to_string(),
        namespace.to_string(),
    );
    temporal
        .signal_llm_result_ready(&run_id)
        .await
        .context("manual LLM response saved but Temporal signal failed; retry llm-submit")?;
    println!(
        "LLM_SUBMIT: OK task_id={} run_id={} state=completed signaled=true",
        submission.task_id, run_id
    );
    Ok(0)
}
