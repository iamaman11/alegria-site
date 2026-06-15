async fn run_rebuild_dispatch(
    client: &infrastructure::adapters::temporalio_sdk_adapter::Client,
    task_queue: &str,
    database_url: Option<String>,
    limit: i64,
    dry_run: bool,
    workflow: RebuildDispatchWorkflowKind,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let rows = sqlx::query(
        r#"
        SELECT
            b.rebuild_request_key,
            b.page_node_key,
            b.trigger_type,
            b.priority,
            b.reason,
            b.created_at::text AS created_at,
            p.scope_signature,
            k.seed_keyword,
            s.market,
            s.locale,
            s.country_code,
            s.visa_type,
            s.applicant_profile,
            s.context_key,
            ctx.visa_subtype,
            ctx.citizenship_code
        FROM monitoring.seo_rebuild_backlog b
        LEFT JOIN site.page_nodes p ON p.page_node_key = b.page_node_key
        LEFT JOIN site.keyword_clusters k ON k.cluster_key = p.keyword_cluster_key
        LEFT JOIN site.site_scopes s ON s.scope_signature = p.scope_signature
        LEFT JOIN kb.visa_contexts ctx ON ctx.context_key = s.context_key
        WHERE b.status = 'queued'
        ORDER BY b.priority ASC, b.created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    let mut dispatched = Vec::new();
    let mut blocked = Vec::new();

    for row in rows {
        let rebuild_request_key: String = row.get("rebuild_request_key");
        let page_node_key = row.get::<Option<String>, _>("page_node_key");
        let trigger_type: String = row.get("trigger_type");
        let priority: i32 = row.get("priority");
        let created_at: String = row.get("created_at");
        let reason_text: String = row.get("reason");
        let reason_value = parse_reason_json(&reason_text);

        let scope_signature = row.get::<Option<String>, _>("scope_signature");
        let seed_keyword = row.get::<Option<String>, _>("seed_keyword");
        let market = row.get::<Option<String>, _>("market");
        let locale = row.get::<Option<String>, _>("locale");
        let country_code = row.get::<Option<String>, _>("country_code");
        let visa_type = row.get::<Option<String>, _>("visa_type");
        let applicant_profile = row.get::<Option<String>, _>("applicant_profile");
        let context_key = row.get::<Option<String>, _>("context_key");
        let visa_subtype = row.get::<Option<String>, _>("visa_subtype");
        let citizenship_code = row.get::<Option<String>, _>("citizenship_code");

        let mut missing = Vec::new();
        if page_node_key.as_deref().unwrap_or_default().is_empty() {
            missing.push("page_node_key");
        }
        if scope_signature.as_deref().unwrap_or_default().is_empty() {
            missing.push("scope_signature");
        }
        if seed_keyword.as_deref().unwrap_or_default().is_empty() {
            missing.push("seed_keyword");
        }
        if market.as_deref().unwrap_or_default().is_empty() {
            missing.push("market");
        }
        if locale.as_deref().unwrap_or_default().is_empty() {
            missing.push("locale");
        }
        if country_code.as_deref().unwrap_or_default().is_empty() {
            missing.push("country_code");
        }
        if visa_type.as_deref().unwrap_or_default().is_empty() {
            missing.push("visa_type");
        }
        if applicant_profile.as_deref().unwrap_or_default().is_empty() {
            missing.push("applicant_profile");
        }
        if context_key.as_deref().unwrap_or_default().is_empty() {
            missing.push("context_key");
        }
        if citizenship_code.as_deref().unwrap_or_default().is_empty() {
            missing.push("citizenship_code");
        }

        if !missing.is_empty() {
            let blocked_reason = json!({
                "status": "blocked",
                "failure_class": "rebuild_dispatch_missing_scope_data",
                "missing_fields": missing,
                "workflow_type": workflow.workflow_type(),
                "reason_package": reason_value,
            });
            if !dry_run {
                sqlx::query(
                    r#"
                    UPDATE monitoring.seo_rebuild_backlog
                    SET status = 'blocked',
                        reason = $2,
                        updated_at = now()
                    WHERE rebuild_request_key = $1
                    "#,
                )
                .bind(&rebuild_request_key)
                .bind(blocked_reason.to_string())
                .execute(&pool)
                .await?;
                if let Some(page_node_key) = page_node_key.as_deref() {
                    sqlx::query(
                        r#"
                        UPDATE site.global_rebuild_plan
                        SET status = 'blocked',
                            reason_payload = $3,
                            updated_at = now()
                        WHERE affected_page_node_key = $1
                          AND trigger_type = $2
                          AND status = 'queued'
                        "#,
                    )
                    .bind(page_node_key)
                    .bind(&trigger_type)
                    .bind(Json(blocked_reason.clone()))
                    .execute(&pool)
                    .await?;
                }
            }
            blocked.push(json!({
                "rebuild_request_key": rebuild_request_key,
                "page_node_key": page_node_key,
                "trigger_type": trigger_type,
                "priority": priority,
                "created_at": created_at,
                "status": if dry_run { "would_block" } else { "blocked" },
                "missing_fields": missing,
            }));
            continue;
        }

        let page_node_key = page_node_key.unwrap_or_default();
        let run_id = Uuid::new_v4().to_string();
        let query_batch_key = format!("rebuild:{}:{}", page_node_key, run_id);
        let dispatch_payload = json!({
            "status": if dry_run { "planned" } else { "running" },
            "workflow_type": workflow.workflow_type(),
            "workflow_id": run_id,
            "run_mode": workflow.run_mode(),
            "seed_keyword": seed_keyword.as_deref().unwrap_or_default(),
            "query_batch_key": query_batch_key,
            "reason_package": reason_value,
        });

        if !dry_run {
            register_site_build_input(
                &repo,
                &SeoSiteBuildRegistrationRequest {
                    run_id: run_id.clone(),
                    context_key: context_key.clone(),
                    market: market.clone().unwrap_or_default(),
                    locale: locale.clone().unwrap_or_default(),
                    country_code: country_code.clone().unwrap_or_default(),
                    visa_type: visa_type.clone().unwrap_or_default(),
                    visa_subtype: visa_subtype.clone().filter(|v| !v.trim().is_empty()),
                    applicant_profile: applicant_profile.clone().unwrap_or_default(),
                    citizenship_code: citizenship_code.clone().unwrap_or_default(),
                    bootstrap_context: false,
                    queries: vec![seed_keyword.clone().unwrap_or_default()],
                    query_batch_key: Some(query_batch_key.clone()),
                    run_mode: Some(workflow.run_mode().to_string()),
                },
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;

            let options = WorkflowStartOptions::new(task_queue.to_string(), run_id.clone()).build();
            client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    empty_payload(),
                    options,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to start {} for rebuild_request_key={}",
                        workflow.workflow_type(),
                        rebuild_request_key
                    )
                })?;

            sqlx::query(
                r#"
                UPDATE monitoring.seo_rebuild_backlog
                SET status = 'running',
                    reason = $2,
                    updated_at = now()
                WHERE rebuild_request_key = $1
                "#,
            )
            .bind(&rebuild_request_key)
            .bind(dispatch_payload.to_string())
            .execute(&pool)
            .await?;

            sqlx::query(
                r#"
                UPDATE site.global_rebuild_plan
                SET status = 'running',
                    reason_payload = $3,
                    updated_at = now()
                WHERE affected_page_node_key = $1
                  AND trigger_type = $2
                  AND status = 'queued'
                "#,
            )
            .bind(&page_node_key)
            .bind(&trigger_type)
            .bind(Json(dispatch_payload.clone()))
            .execute(&pool)
            .await?;
        }

        dispatched.push(json!({
            "rebuild_request_key": rebuild_request_key,
            "page_node_key": page_node_key,
            "trigger_type": trigger_type,
            "priority": priority,
            "created_at": created_at,
            "workflow_type": workflow.workflow_type(),
            "workflow_id": run_id,
            "run_mode": workflow.run_mode(),
            "query_batch_key": query_batch_key,
            "seed_keyword": seed_keyword,
            "scope_signature": scope_signature,
            "status": if dry_run { "planned" } else { "running" },
        }));
    }

    let payload = json!({
        "status": "ok",
        "dry_run": dry_run,
        "workflow_type": workflow.workflow_type(),
        "dispatched_count": dispatched.len(),
        "blocked_count": blocked.len(),
        "dispatched": dispatched,
        "blocked": blocked,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(0)
}

