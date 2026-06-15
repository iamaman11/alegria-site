async fn cms_review_list(database_url: Option<String>, limit: i64) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT p.page_node_key, p.canonical_url_path, p.current_status,
               r.revision_id, r.title, r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.current_status IN ('review_required','blocked','approved')
        ORDER BY r.updated_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "page_node_key": row.get::<String, _>("page_node_key"),
                "canonical_url_path": row.get::<String, _>("canonical_url_path"),
                "current_status": row.get::<String, _>("current_status"),
                "revision_id": row.get::<String, _>("revision_id"),
                "title": row.get::<String, _>("title"),
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn cms_review_show(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.scope_signature, p.canonical_url_path, p.current_status,
               r.revision_id, r.title, r.meta_description, r.h1, r.body_payload,
               r.schema_markup_payload, r.required_link_payload, r.traceability_manifest,
               r.revision_status, r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    let body: Json<Value> = row.get("body_payload");
    let schema: Json<Value> = row.get("schema_markup_payload");
    let links: Json<Value> = row.get("required_link_payload");
    let traceability: Json<Value> = row.get("traceability_manifest");
    let payload = json!({
        "page_node_key": row.get::<String, _>("page_node_key"),
        "scope_signature": row.get::<String, _>("scope_signature"),
        "canonical_url_path": row.get::<String, _>("canonical_url_path"),
        "current_status": row.get::<String, _>("current_status"),
        "revision_id": row.get::<String, _>("revision_id"),
        "revision_status": row.get::<String, _>("revision_status"),
        "title": row.get::<String, _>("title"),
        "meta_description": row.get::<String, _>("meta_description"),
        "h1": row.get::<String, _>("h1"),
        "body_payload": body.0,
        "schema_markup_payload": schema.0,
        "required_link_payload": links.0,
        "traceability_manifest": traceability.0,
        "updated_at": row.get::<String, _>("updated_at"),
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn cms_publish_status(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.current_status, p.published_at::text AS published_at,
               p.current_revision_id, r.revision_status,
               a.status AS artifact_status, a.artifact_uri, a.updated_at::text AS artifact_updated_at
        FROM site.cms_pages p
        LEFT JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        LEFT JOIN site.publish_artifacts a
          ON a.revision_id = p.current_revision_id
         AND a.page_node_key = p.page_node_key
        WHERE p.page_node_key = $1
        ORDER BY a.updated_at DESC NULLS LAST
        LIMIT 1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "page_node_key": row.get::<String, _>("page_node_key"),
            "current_status": row.get::<String, _>("current_status"),
            "published_at": row.get::<Option<String>, _>("published_at"),
            "current_revision_id": row.get::<Option<String>, _>("current_revision_id"),
            "revision_status": row.get::<Option<String>, _>("revision_status"),
            "artifact_status": row.get::<Option<String>, _>("artifact_status"),
            "artifact_uri": row.get::<Option<String>, _>("artifact_uri"),
            "artifact_updated_at": row.get::<Option<String>, _>("artifact_updated_at"),
        }))?
    );
    Ok(0)
}

async fn cms_traceability_inspect(
    database_url: Option<String>,
    page_node_key: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT r.revision_id, r.traceability_manifest
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    let traceability: Json<Value> = row.get("traceability_manifest");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "page_node_key": page_node_key,
            "revision_id": row.get::<String, _>("revision_id"),
            "traceability_manifest": traceability.0,
        }))?
    );
    Ok(0)
}

async fn cms_blockers_inspect(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT task_key, task_type, queue_state, blocking_step_name,
               severity, decision_payload, audit_log_payload,
               updated_at::text AS updated_at
        FROM site.seo_hitl_tasks
        WHERE page_node_key = $1
          AND queue_state IN ('open','reopened','in_review')
        ORDER BY updated_at DESC
        "#,
    )
    .bind(page_node_key)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "task_key": row.get::<String, _>("task_key"),
                "task_type": row.get::<String, _>("task_type"),
                "queue_state": row.get::<String, _>("queue_state"),
                "blocking_step_name": row.get::<String, _>("blocking_step_name"),
                "severity": row.get::<String, _>("severity"),
                "decision_payload": row.get::<Json<Value>, _>("decision_payload").0,
                "audit_log_payload": row.get::<Json<Value>, _>("audit_log_payload").0,
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_rebuild_backlog_inspect(
    database_url: Option<String>,
    page_node_key: Option<String>,
    limit: i64,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = if let Some(page_node_key) = page_node_key.as_ref() {
        sqlx::query(
            r#"
            SELECT rebuild_request_key, page_node_key, trigger_type, priority, status, reason,
                   updated_at::text AS updated_at
            FROM monitoring.seo_rebuild_backlog
            WHERE page_node_key = $1
            ORDER BY updated_at DESC
            LIMIT $2
            "#,
        )
        .bind(page_node_key)
        .bind(limit)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT rebuild_request_key, page_node_key, trigger_type, priority, status, reason,
                   updated_at::text AS updated_at
            FROM monitoring.seo_rebuild_backlog
            ORDER BY updated_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&pool)
        .await?
    };
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "rebuild_request_key": row.get::<String, _>("rebuild_request_key"),
                "page_node_key": row.get::<Option<String>, _>("page_node_key"),
                "trigger_type": row.get::<String, _>("trigger_type"),
                "priority": row.get::<i32, _>("priority"),
                "status": row.get::<String, _>("status"),
                "reason": row.get::<String, _>("reason"),
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_support_bundle_inspect(
    database_url: Option<String>,
    context_key: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id,
            r.rule_type_key,
            r.role_type,
            r.status,
            r.effective_from::text AS effective_from,
            r.effective_to::text AS effective_to,
            COALESCE(c.label_ru, c.concept_key, r.concept_key) AS concept_label,
            COALESCE(s.source_label, '') AS source_label,
            COALESCE(s.source_type, 'editorial') AS source_type,
            COALESCE(s.trust_level, 3) AS trust_level
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1
          AND r.status = 'verified'
        ORDER BY r.role_type, r.rule_instance_id
        "#,
    )
    .bind(context_key)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "rule_instance_id": row.get::<String, _>("rule_instance_id"),
                "rule_type_key": row.get::<String, _>("rule_type_key"),
                "role_type": row.get::<String, _>("role_type"),
                "concept_label": row.get::<String, _>("concept_label"),
                "source_label": row.get::<String, _>("source_label"),
                "source_type": row.get::<String, _>("source_type"),
                "trust_level": row.get::<i32, _>("trust_level"),
                "effective_from": row.get::<Option<String>, _>("effective_from"),
                "effective_to": row.get::<Option<String>, _>("effective_to"),
                "status": row.get::<String, _>("status"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_post_publish_feedback_probe(
    analytics_addr: &str,
    require_gsc: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let analytics = match AnalyticsClient::connect(analytics_addr).await {
        Ok(mut client) => {
            let ok = client.ping().await;
            json!({
                "status": if ok { "ok" } else { "error" },
                "addr": analytics_addr,
            })
        }
        Err(err) => json!({
            "status": "error",
            "addr": analytics_addr,
            "error": err.to_string(),
        }),
    };

    let gsc_access_token = env::var("GSC_ACCESS_TOKEN").ok();
    let gsc_site_url = env::var("GSC_SITE_URL").ok();
    let gsc = match (gsc_access_token, gsc_site_url) {
        (Some(token), Some(site_url))
            if !token.trim().is_empty() && !site_url.trim().is_empty() =>
        {
            let http = new_default_client(30)?;
            match http
                .get("https://www.googleapis.com/webmasters/v3/sites")
                .bearer_auth(token)
                .send()
                .await
            {
                Ok(resp) => json!({
                    "status": if resp.status().is_success() { "ok" } else { "error" },
                    "http_status": resp.status().as_u16(),
                    "site_url": site_url,
                }),
                Err(err) => json!({
                    "status": "error",
                    "site_url": site_url,
                    "error": err.to_string(),
                }),
            }
        }
        _ if require_gsc => json!({
            "status": "error",
            "error": "GSC_ACCESS_TOKEN and GSC_SITE_URL are required",
        }),
        _ => json!({
            "status": "not_configured",
        }),
    };

    let analytics_ok = analytics.get("status").and_then(Value::as_str) == Some("ok");
    let gsc_status = gsc.get("status").and_then(Value::as_str).unwrap_or("error");
    let gsc_ok = gsc_status == "ok" || (!require_gsc && gsc_status == "not_configured");
    let overall_status = if analytics_ok && gsc_ok {
        "ok"
    } else {
        "error"
    };

    let payload = json!({
        "status": overall_status,
        "analytics": analytics,
        "gsc": gsc,
        "truth_mutation": "forbidden",
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(Path::new("."), report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if overall_status == "ok" { 0 } else { 2 })
}

