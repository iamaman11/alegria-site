fn default_database_url() -> String {
    env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres_password@localhost:5433/alegria".to_string()
    })
}

fn default_temporal_url() -> String {
    env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".to_string())
}

fn default_temporal_namespace() -> String {
    env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| "default".to_string())
}

fn evaluate_gate_status(report: &Value, blocking_reasons: &mut Vec<String>) {
    let label = report
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("unknown_gate");
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    if status == "error" {
        let exit_code = report
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        blocking_reasons.push(format!("{label} failed (exit_code={exit_code})"));
    }
}

async fn build_static_site(
    database_url: Option<String>,
    output_dir: &str,
    base_url: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    // One canonical renderer for CLI public export and workflow candidate builds.
    let build = infrastructure::adapters::static_site_builder_adapter::build_static_site(
        &pool,
        Path::new(output_dir),
        base_url,
    )
    .await?;
    println!(
        "STATIC_SITE_BUILD: OK pages={} artifacts={} output={}",
        build.previews.len(),
        build.artifacts.len(),
        output_dir
    );
    Ok(0)
}

async fn crawl_pending_sources(
    database_url: Option<String>,
    run_id: String,
    query_batch_key: String,
    limit: i64,
    emit_qdrant: bool,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let output = run_crawl_sources(
        &repo,
        &contracts::generated::alegria::temporal::v1::CrawlSourcesInputPayload {
            run_id,
            query_batch_key,
            limit: limit as u32,
            emit_qdrant,
        },
    )
    .await?;
    println!(
        "CRAWL_SUMMARY claimed={} crawled={} failed={} raw_pages={} status={}",
        output.claimed_count,
        output.crawled_count,
        output.failed_count,
        output.raw_page_count,
        output.status
    );
    for url in &output.failed_urls {
        eprintln!("CRAWL_FAILED url={url}");
    }
    Ok(if output.failed_count == 0 { 0 } else { 1 })
}
