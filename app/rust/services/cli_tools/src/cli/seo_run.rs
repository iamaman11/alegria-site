fn default_seo_run_id() -> String {
    Uuid::new_v4().to_string()
}

fn map_seo_run_kind(command: SeoRunCommand) -> SeoScenarioKind {
    match command {
        SeoRunCommand::Planning => SeoScenarioKind::PlanningOnly,
        SeoRunCommand::Drafting => SeoScenarioKind::DraftingOnly,
        SeoRunCommand::Publish => SeoScenarioKind::PublishOnly,
        SeoRunCommand::Full => SeoScenarioKind::Full,
        SeoRunCommand::Rebuild => SeoScenarioKind::RebuildOnly,
        SeoRunCommand::CrawlIngest => SeoScenarioKind::CrawlIngestOnly,
    }
}

async fn register_seo_run_input(
    database_url: Option<String>,
    scenario: SeoRunCommand,
    run_id: Option<String>,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    queries: Vec<String>,
    query_batch_key: Option<String>,
    publish: bool,
    require_preapproved_decision: bool,
    warn_only_projections: bool,
) -> Result<contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    register_site_build_input(
        &repo,
        &SeoSiteBuildRegistrationRequest {
            run_id: run_id.unwrap_or_else(default_seo_run_id),
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            queries,
            query_batch_key,
            run_mode: Some(
                run_mode_for_scenario(
                    map_seo_run_kind(scenario),
                    publish,
                    require_preapproved_decision,
                    warn_only_projections,
                )
                .to_string(),
            ),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))
}

async fn seo_run(
    scenario: SeoRunCommand,
    database_url: Option<String>,
    run_id: Option<String>,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    queries: Vec<String>,
    query_batch_key: Option<String>,
    output_dir: String,
    base_url: String,
    publish: bool,
    require_preapproved_decision: bool,
    warn_only_projections: bool,
) -> Result<i32> {
    let site_input = register_seo_run_input(
        database_url.clone(),
        scenario,
        run_id,
        context_key,
        market,
        locale,
        country_code,
        visa_type,
        visa_subtype,
        applicant_profile,
        citizenship_code,
        bootstrap_context,
        queries,
        query_batch_key,
        publish,
        require_preapproved_decision,
        warn_only_projections,
    )
    .await?;
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let result = execute_site_build_scenario(
        &repo,
        &SeoScenarioRequest {
            scenario: map_seo_run_kind(scenario),
            mode: SeoExecutionMode::SemiAutoOperator,
            policy: SeoRunPolicy::for_semi_auto_operator(
                publish,
                require_preapproved_decision,
                warn_only_projections,
            ),
            output_dir,
            base_url,
            site_input,
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;

    println!(
        "SEO_RUN_RESULT scenario={} mode={} status={} page_total={} published_pages={} changed_truth_keys={}",
        result.scenario,
        result.mode,
        result.status,
        result.page_total,
        result.published_pages,
        result.changed_truth_keys.len()
    );
    for report in result.phase_reports {
        println!(
            "SEO_RUN_PHASE phase={} status={} page_node_key={} detail={}",
            report.phase, report.status, report.page_node_key, report.detail
        );
    }
    Ok(if result.status.starts_with("blocked") {
        1
    } else {
        0
    })
}

