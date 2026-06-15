async fn run_seo_preflight(
    database_url: Option<String>,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    strict_projections: bool,
    projection_max_lag_ms: i64,
    output_dir: String,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let db_connect_timeout_s = env::var("SEO_PREFLIGHT_DB_CONNECT_TIMEOUT_S")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    let pool = tokio::time::timeout(
        Duration::from_secs(db_connect_timeout_s.max(5)),
        connect_pg(&database_url),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "database_connect timed out after {}s",
            db_connect_timeout_s.max(5)
        )
    })?
    .context("database_connect failed")?;
    sqlx_seo_adapter::ensure_seo_runtime_registries(&pool)
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let scope = identity::derive_scope(
        &market,
        &locale,
        &country_code,
        &visa_type,
        &applicant_profile,
    )
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    let normalized_profile =
        sqlx_seo_adapter::validate_applicant_profile_reference(&pool, &scope.applicant_profile)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
    println!("OK database_connect");
    println!("OK scope market={market} locale={locale} country={country_code} visa_type={visa_type} applicant_profile={normalized_profile}");

    let truth_identity = identity::derive_truth_identity(
        &country_code,
        &visa_type,
        visa_subtype.as_deref(),
        &citizenship_code,
    )
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    let resolved_context_key = if bootstrap_context {
        let report = sqlx_seo_adapter::bootstrap_seo_scope(
            &pool,
            context_key.as_deref(),
            &truth_identity.country_code,
            &truth_identity.visa_family,
            if truth_identity.visa_subtype.is_empty() {
                None
            } else {
                Some(truth_identity.visa_subtype.as_str())
            },
            &truth_identity.citizenship_code,
        )
        .await
        .context("bootstrap_seo_scope failed")?;
        println!(
            "OK seo_scope context_key={} created_context={} seeded_registries={}",
            report.context_key, report.created_context, report.seeded_registry_count
        );
        report.context_key
    } else {
        context_key.unwrap_or_else(|| truth_identity.context_key.clone())
    };

    let context_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM kb.visa_contexts WHERE context_key = $1 AND status = 'active'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("active context lookup failed")?;
    let context_count: i64 = sqlx::Row::get(&context_row, "count");
    if context_count == 0 {
        println!("FAIL active_context context_key={resolved_context_key}");
        return Ok(2);
    }
    println!("OK active_context context_key={resolved_context_key}");

    let page_type_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM site.registry_page_types WHERE status = 'active'",
    )
    .fetch_one(&pool)
    .await
    .context("page type registry count failed")?;
    let page_type_count: i64 = sqlx::Row::get(&page_type_row, "count");
    if page_type_count == 0 {
        println!("FAIL registry_page_types active_count=0");
        return Ok(2);
    }
    println!("OK registry_page_types active_count={page_type_count}");

    let page_node_row = sqlx::query("SELECT count(*)::bigint AS count FROM site.page_nodes")
        .fetch_one(&pool)
        .await
        .context("page node count failed")?;
    let page_node_count: i64 = sqlx::Row::get(&page_node_row, "count");
    let navigation_item_row =
        sqlx::query("SELECT count(*)::bigint AS count FROM site.navigation_items")
            .fetch_one(&pool)
            .await
            .context("navigation item count failed")?;
    let navigation_item_count: i64 = sqlx::Row::get(&navigation_item_row, "count");
    println!(
        "OK global_site_model page_nodes={} navigation_items={}",
        page_node_count, navigation_item_count
    );

    let verified_rule_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM verified.rule_instances WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("verified rule count failed")?;
    let verified_rule_count: i64 = sqlx::Row::get(&verified_rule_row, "count");
    let pending_rule_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM verified.rule_instances WHERE context_key = $1 AND status = 'pending'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("pending rule count failed")?;
    let pending_rule_count: i64 = sqlx::Row::get(&pending_rule_row, "count");
    println!(
        "OK knowledge_state verified_rules={} pending_review_rules={}",
        verified_rule_count, pending_rule_count
    );

    let qdrant_point_row = sqlx::query("SELECT count(*)::bigint AS count FROM kb.qdrant_points")
        .fetch_one(&pool)
        .await
        .context("qdrant point ledger count failed")?;
    let qdrant_point_count: i64 = sqlx::Row::get(&qdrant_point_row, "count");
    println!("OK qdrant_point_ledger points={qdrant_point_count}");

    let required_collections = required_retrieval_collections();
    let collection_status_rows =
        sqlx_seo_adapter::read_qdrant_collection_statuses(&pool, &required_collections)
            .await
            .context("qdrant collection status read failed")?;
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let qdrant_probe = tokio::time::timeout(
        Duration::from_secs(3),
        probe_required_qdrant_collections(&qdrant_url, &required_collections),
    )
    .await;
    let (qdrant_ready, qdrant_collection_presence) = match qdrant_probe {
        Ok(Ok(statuses)) => (true, statuses),
        Ok(Err(err)) => {
            println!("WARN qdrant_connect url={} error={}", qdrant_url, err);
            (false, BTreeMap::new())
        }
        Err(_) => {
            println!("WARN qdrant_connect url={} error=timeout", qdrant_url);
            (false, BTreeMap::new())
        }
    };
    let required_collection_statuses = collection_status_rows
        .into_iter()
        .map(|status| {
            let qdrant_collection_exists = qdrant_collection_presence
                .get(&status.collection_name)
                .copied()
                .unwrap_or(false);
            SeoPreflightCollectionReport {
                collection_name: status.collection_name,
                exists: status.point_count > 0,
                fresh: status
                    .lag_seconds
                    .map(|lag| lag * 1000 <= projection_max_lag_ms.max(0))
                    .unwrap_or(false),
                projection_complete: status.point_count > 0,
                point_count: status.point_count,
                last_materialized_at: status.last_materialized_at,
                last_source_change_at: None,
                lag_seconds: status.lag_seconds,
                qdrant_collection_exists,
            }
        })
        .collect::<Vec<_>>();
    let qdrant_collection_contract_ready = required_collection_statuses.iter().all(|status| {
        status.exists && status.projection_complete && status.qdrant_collection_exists
    });
    let required_graph_projection_names = required_graph_projections();
    let graph_projection_status_rows =
        sqlx_seo_adapter::read_graph_projection_statuses(&pool, &required_graph_projection_names)
            .await
            .context("graph projection status read failed")?;
    let required_graph_projections = graph_projection_status_rows
        .into_iter()
        .map(|status| SeoPreflightGraphProjectionReport {
            projection_name: status.artifact_name,
            exists: status.point_count > 0,
            fresh: status
                .lag_seconds
                .map(|lag| lag * 1000 <= projection_max_lag_ms.max(0))
                .unwrap_or(false),
            projection_complete: status.point_count > 0,
            point_count: status.point_count,
            last_materialized_at: status.last_materialized_at,
            last_source_change_at: None,
            lag_seconds: status.lag_seconds,
        })
        .collect::<Vec<_>>();
    let graph_projection_contract_ready = required_graph_projections
        .iter()
        .all(|status| status.exists && status.projection_complete);

    let mut projection_blocked = false;
    let mut graph_projection_blocked = false;
    let projection_statuses = sqlx_seo_adapter::read_projection_sync_status(&pool)
        .await
        .context("projection sync status failed")?;
    for status in projection_statuses {
        let open_events = status.open_event_count();
        let blocked_events = status.blocking_event_count();
        if blocked_events > 0 {
            projection_blocked = true;
        }
        if status.target_system == "neo4j" && blocked_events > 0 {
            graph_projection_blocked = true;
        }
        let state = if status.failed_events > 0 {
            "FAIL"
        } else if open_events > 0 && status.max_open_lag_ms > projection_max_lag_ms {
            "WARN"
        } else {
            "OK"
        };
        if status.target_system == "neo4j"
            && open_events > 0
            && status.max_open_lag_ms > projection_max_lag_ms
        {
            graph_projection_blocked = true;
        }
        println!(
            "{} projection_status target={} pending={} processing={} failed={} done={} max_open_lag_ms={}",
            state,
            status.target_system,
            status.pending_events,
            status.processing_events,
            status.failed_events,
            status.done_events,
            status.max_open_lag_ms
        );
        if let Some(aggregate_key) = status.oldest_open_aggregate_key.as_deref() {
            println!(
                "INFO projection_oldest_open target={} event_id={} aggregate_key={} event_type={}",
                status.target_system,
                status.oldest_open_event_id.as_deref().unwrap_or(""),
                aggregate_key,
                status.oldest_open_event_type.as_deref().unwrap_or("")
            );
        }
        if let Some(aggregate_key) = status.latest_failed_aggregate_key.as_deref() {
            println!(
                "INFO projection_latest_failed target={} aggregate_key={} event_type={} error={}",
                status.target_system,
                aggregate_key,
                status.latest_failed_event_type.as_deref().unwrap_or(""),
                status.latest_failed_error.as_deref().unwrap_or("")
            );
        }
    }
    if strict_projections && projection_blocked {
        println!("FAIL projection_barrier strict=true blocked_events_present=true");
        return Ok(2);
    }
    if projection_blocked {
        println!("WARN projection_barrier strict=false blocked_events_present=true");
    } else {
        println!("OK projection_barrier all_targets_drained=true");
    }

    let capability_probe = probe_voyage_capabilities().await;
    let voyage_embeddings_ready = capability_probe.embeddings_ready;
    let voyage_contextualized_ready = capability_probe.contextualized_ready;
    let voyage_rerank_ready = capability_probe.rerank_ready;
    let neo4j_probe = probe_neo4j_capabilities().await;
    let neo4j_ready = neo4j_probe.neo4j_ready;
    let graph_query_ready = neo4j_probe.graph_query_ready;
    let graph_gds_ready = neo4j_probe.graph_gds_ready;
    let retrieval_capability_required = env_flag("RETRIEVAL_CAPABILITY_REQUIRED");
    let canonical_vector_retrieval_required = env_flag("CANONICAL_VECTOR_RETRIEVAL_REQUIRED");
    let contextual_raw_chunk_retrieval_required =
        env_flag("CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED");
    let voyage_rerank_required = env_flag("VOYAGE_RERANK_REQUIRED");
    let graph_capability_required = env_flag("GRAPH_CAPABILITY_REQUIRED");
    let neo4j_sync_required = env_flag("NEO4J_SYNC_REQUIRED");
    let graph_query_required = env_flag("GRAPH_QUERY_REQUIRED");
    let graph_gds_required = env_flag("GRAPH_GDS_REQUIRED");

    let missing_collections = required_collection_statuses
        .iter()
        .filter(|status| !status.exists || !status.qdrant_collection_exists)
        .map(|status| status.collection_name.clone())
        .collect::<Vec<_>>();
    let stale_collections = required_collection_statuses
        .iter()
        .filter(|status| status.exists && !status.fresh)
        .map(|status| status.collection_name.clone())
        .collect::<Vec<_>>();
    let incomplete_collections = required_collection_statuses
        .iter()
        .filter(|status| status.exists && !status.projection_complete)
        .map(|status| status.collection_name.clone())
        .collect::<Vec<_>>();
    let missing_graph_projections = required_graph_projections
        .iter()
        .filter(|status| !status.exists)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let stale_graph_projections = required_graph_projections
        .iter()
        .filter(|status| status.exists && !status.fresh)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let incomplete_graph_projections = required_graph_projections
        .iter()
        .filter(|status| status.exists && !status.projection_complete)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let retrieval_status = if retrieval_capability_required {
        if !voyage_embeddings_ready
            || (contextual_raw_chunk_retrieval_required && !voyage_contextualized_ready)
            || (voyage_rerank_required && !voyage_rerank_ready)
            || (canonical_vector_retrieval_required && !voyage_embeddings_ready)
        {
            "blocked_provider_capability".to_string()
        } else if !qdrant_ready {
            "blocked_retrieval_contract".to_string()
        } else if !missing_collections.is_empty() {
            "blocked_missing_collection".to_string()
        } else if !stale_collections.is_empty() {
            "blocked_stale_collection".to_string()
        } else if projection_blocked || !incomplete_collections.is_empty() {
            "blocked_projection_incomplete".to_string()
        } else {
            "pass".to_string()
        }
    } else if projection_blocked {
        "warn".to_string()
    } else {
        "pass".to_string()
    };
    let retrieval_block_reason = match retrieval_status.as_str() {
        "blocked_provider_capability" => Some(
            format!(
                "retrieval contract requires Voyage embeddings/contextualized/rerank capabilities, probe failed: {}",
                if capability_probe.errors.is_empty() {
                    "unknown capability probe failure".to_string()
                } else {
                    capability_probe.errors.join("; ")
                }
            ),
        ),
        "blocked_retrieval_contract" => Some(
            "retrieval contract requires reachable Qdrant and required retrieval surfaces".to_string(),
        ),
        "blocked_missing_collection" => Some(format!(
            "missing required retrieval collections: {}",
            missing_collections.join(",")
        )),
        "blocked_stale_collection" => Some(format!(
            "stale required retrieval collections: {}",
            stale_collections.join(",")
        )),
        "blocked_projection_incomplete" => Some(format!(
            "projection backlog or incomplete retrieval collections block the required retrieval contract: projection_blocked={} incomplete=[{}]",
            projection_blocked,
            incomplete_collections.join(",")
        )),
        _ => None,
    };
    let graph_contract_status = if graph_capability_required {
        if (neo4j_sync_required && !neo4j_ready)
            || (graph_query_required && !graph_query_ready)
            || (graph_gds_required && !graph_gds_ready)
        {
            "blocked_provider_capability".to_string()
        } else if !missing_graph_projections.is_empty() {
            "blocked_missing_projection".to_string()
        } else if !stale_graph_projections.is_empty() {
            "blocked_stale_projection".to_string()
        } else if graph_projection_blocked || !incomplete_graph_projections.is_empty() {
            "blocked_projection_incomplete".to_string()
        } else {
            "pass".to_string()
        }
    } else if graph_projection_blocked {
        "warn".to_string()
    } else {
        "pass".to_string()
    };
    let graph_block_reason = match graph_contract_status.as_str() {
        "blocked_provider_capability" => Some(format!(
            "graph contract requires neo4j/gds/query capabilities, probe failed: {}",
            if neo4j_probe.errors.is_empty() {
                "unknown neo4j probe failure".to_string()
            } else {
                neo4j_probe.errors.join("; ")
            }
        )),
        "blocked_missing_projection" => Some(format!(
            "missing required graph projections: {}",
            missing_graph_projections.join(",")
        )),
        "blocked_stale_projection" => Some(format!(
            "stale required graph projections: {}",
            stale_graph_projections.join(",")
        )),
        "blocked_projection_incomplete" => Some(format!(
            "graph projection backlog or incomplete projections block contract: graph_projection_blocked={} incomplete=[{}]",
            graph_projection_blocked,
            incomplete_graph_projections.join(",")
        )),
        _ => None,
    };

    if env_set("DATAFORSEO_LOGIN") && env_set("DATAFORSEO_PASSWORD") {
        println!("OK dataforseo_credentials");
    } else {
        println!(
            "WARN dataforseo_credentials missing; live SERP discovery will not populate crawl queue"
        );
    }
    if voyage_embeddings_ready || voyage_contextualized_ready || voyage_rerank_ready {
        println!("OK voyage_capability_probe");
        println!(
            "OK voyage_models embedding={} contextualized={} rerank={} probe={{embeddings:{},contextualized:{},rerank:{}}}",
            env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string()),
            env::var("VOYAGE_CONTEXT_MODEL").unwrap_or_else(|_| "voyage-context-3".to_string()),
            env::var("VOYAGE_RERANK_MODEL").unwrap_or_else(|_| "rerank-2.5".to_string()),
            voyage_embeddings_ready,
            voyage_contextualized_ready,
            voyage_rerank_ready
        );
    } else {
        println!(
            "WARN voyage_capability_probe failed; semantic retrieval contract may block runtime"
        );
    }
    if qdrant_ready {
        println!(
            "OK qdrant_connect url={} required_collection_contract={}",
            qdrant_url, qdrant_collection_contract_ready
        );
    }
    if neo4j_ready {
        println!("OK neo4j_connect");
    } else {
        println!(
            "WARN neo4j_capability_probe failed: {}",
            if neo4j_probe.errors.is_empty() {
                "unknown".to_string()
            } else {
                neo4j_probe.errors.join("; ")
            }
        );
    }
    if llm_configured() {
        println!("OK llm_provider");
    } else {
        println!("WARN llm_provider missing; deterministic fallback cannot produce production-quality pages");
    }

    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("static output dir is not writable: {output_dir}"))?;
    println!("OK static_output_dir path={output_dir}");

    let report = SeoPreflightReport {
        artifact_id: "seo_preflight".to_string(),
        status: if retrieval_status != "pass" {
            retrieval_status.clone()
        } else {
            graph_contract_status.clone()
        },
        context_key: resolved_context_key.clone(),
        normalized_profile,
        retrieval_capability_required,
        canonical_vector_retrieval_required,
        contextual_raw_chunk_retrieval_required,
        voyage_rerank_required,
        voyage_embeddings_ready,
        voyage_contextualized_ready,
        voyage_rerank_ready,
        qdrant_ready,
        qdrant_collection_contract_ready,
        neo4j_ready,
        graph_query_ready,
        graph_gds_ready,
        graph_projection_contract_ready,
        graph_capability_required,
        neo4j_sync_required,
        graph_query_required,
        graph_gds_required,
        projection_blocked,
        retrieval_block_reason: retrieval_block_reason.clone(),
        graph_contract_status: graph_contract_status.clone(),
        graph_block_reason: graph_block_reason.clone(),
        required_collections: required_collection_statuses,
        required_graph_projections,
    };
    if let Some(path) = report_json.as_deref() {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create seo preflight report parent: {}", parent.display())
            })?;
        }
        fs::write(
            &path,
            serde_json::to_vec_pretty(&report).context("serialize seo preflight report")?,
        )
        .with_context(|| format!("write seo preflight report: {}", path.display()))?;
        println!("INFO seo_preflight_report path={}", path.display());
    }

    if retrieval_capability_required && retrieval_status != "pass" {
        println!(
            "FAIL retrieval_contract status={} reason={}",
            retrieval_status,
            retrieval_block_reason.as_deref().unwrap_or("")
        );
        return Ok(2);
    }
    if graph_capability_required && graph_contract_status != "pass" {
        println!(
            "FAIL graph_contract status={} reason={}",
            graph_contract_status,
            graph_block_reason.as_deref().unwrap_or("")
        );
        return Ok(2);
    }

    Ok(0)
}

