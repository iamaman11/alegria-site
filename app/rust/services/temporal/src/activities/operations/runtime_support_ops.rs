pub(crate) fn test_step_prepare_impl(workflow_id: &str) -> String {
    format!("prepared:{workflow_id}")
}

pub(crate) fn test_step_finalize_impl(prepared_token: &str) -> String {
    format!("completed:{prepared_token}")
}

pub(crate) async fn check_data_freshness_impl(
    acts: &AlegriaActivities,
    threshold_input: &str,
) -> Result<String, DomainError> {
    let threshold_hours: i64 = threshold_input
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(24);

    let snapshot = load_freshness_snapshot(&acts.pool, threshold_hours)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let report = FreshnessReport {
        meta: Some(StepContractMeta {
            run_id: "operational:freshness".to_string(),
            step_name: "check_data_freshness".to_string(),
            schema_version: 1,
            input_hash: content_hash_v1(threshold_input),
            output_hash: String::new(),
            idempotency_key: content_hash_v1(&format!("operational:freshness|{}", threshold_input)),
            requires_hitl: false,
            prompt_version: String::new(),
            model_version: String::new(),
            registry_version: String::new(),
            error_class: String::new(),
            retry_class: "transient".to_string(),
            executor_version: AlegriaActivities::current_build_id(),
            derivation_version: "check_data_freshness@1".to_string(),
            scope_signature: String::new(),
            max_retries: 3,
        }),
        threshold_hours,
        stale_count: snapshot.stale_count,
        max_lag_hours: snapshot.max_lag_hours,
        status: if snapshot.stale_count > 0 {
            "stale"
        } else {
            "ok"
        }
        .to_string(),
    };

    serde_json::to_string(&report).map_err(AlegriaActivities::classify_error)
}

pub(crate) async fn neo4j_backwrite_impl(
    input: &Neo4jBackwriteInput,
) -> Result<Neo4jBackwriteOutput, DomainError> {
    let mut opts = sqlx_reconcile_adapter::load_default_reconcile_options();
    opts.dry_run = input.dry_run;
    if let Some(v) = input.max_retry_count {
        opts.max_retry_count = v;
    }
    if let Some(v) = input.batch_limit {
        opts.batch_limit = v;
    }
    if let Some(v) = input.requeue_base_delay_sec {
        opts.requeue_base_delay_sec = v;
    }
    if let Some(v) = input.requeue_jitter_sec {
        opts.requeue_jitter_sec = v;
    }

    let target = if input.target_system.trim().is_empty() {
        "neo4j"
    } else {
        input.target_system.as_str()
    };
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(target, &opts)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    Ok(Neo4jBackwriteOutput {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_reconcile_impl(
    target_system: &str,
    dry_run: bool,
    max_retry_count: i32,
    batch_limit: i64,
    requeue_base_delay_sec: i64,
    requeue_jitter_sec: i64,
) -> Result<ReconcileTargetReportRecord, DomainError> {
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(
        target_system,
        &sqlx_reconcile_adapter::ReconcileOptionsRecord {
            max_retry_count,
            batch_limit,
            dry_run,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        },
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(ReconcileTargetReportRecord {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_sync_impl(
    acts: &AlegriaActivities,
    input: &ProjectionSyncInput,
) -> Result<ProjectionSyncOutput, DomainError> {
    let worker_id = format!("projection-sync:{}:{}", input.step_name, input.run_id);
    let batch = sqlx_outbox_adapter::claim_outbox_batch_for_run_target(
        &acts.pool,
        &worker_id,
        &input.run_id,
        &input.target_system,
        input.batch_limit.max(1),
        input.lease_seconds.max(30),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    let mut processed_events = 0_i64;
    let retried_events = 0_i64;
    let failed_events = 0_i64;

    for event in batch {
        match projection_materialize_adapter::dispatch_event(
            &event.target_system,
            &event.event_type,
            &event.aggregate_key,
            &event.payload_type,
            &event.payload_bytes,
        )
        .await
        {
            Ok(_) => {
                sqlx_outbox_adapter::mark_done(&acts.pool, event.event_id)
                    .await
                    .map_err(AlegriaActivities::classify_error)?;
                processed_events += 1;
            }
            Err(err) => {
                let message = err.to_string();
                if event.retry_count + 1 >= 10 {
                    sqlx_outbox_adapter::mark_failed(&acts.pool, event.event_id, &message)
                        .await
                        .map_err(AlegriaActivities::classify_error)?;
                } else {
                    sqlx_outbox_adapter::mark_retry(&acts.pool, event.event_id, &message, 30)
                        .await
                        .map_err(AlegriaActivities::classify_error)?;
                }
                return Err(DomainError::InfraUnavailable { message });
            }
        }
    }

    let backlog = sqlx_seo_adapter::projection_backlog_counts(
        &acts.pool,
        &input.run_id,
        &input.target_system,
    )
    .await?;
    let remaining_pending = backlog.pending_events;
    let remaining_failed = backlog.failed_events;

    Ok(ProjectionSyncOutput {
        run_id: input.run_id.clone(),
        target_system: input.target_system.clone(),
        processed_events,
        retried_events,
        failed_events,
        remaining_pending,
        remaining_failed,
        status: if remaining_pending == 0 && remaining_failed == 0 && retried_events == 0 {
            "done".to_string()
        } else if failed_events > 0 || retried_events > 0 {
            "blocked".to_string()
        } else {
            "partial".to_string()
        },
    })
}

pub(crate) async fn load_semantic_section_sample_impl(
    acts: &AlegriaActivities,
    input: &SemanticSectionSampleInput,
) -> Result<SemanticSectionSampleOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let section = sections
        .into_iter()
        .find(|section| !section.content_md.trim().is_empty())
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "no non-empty raw section available for semantic slice".to_string(),
        })?;

    Ok(SemanticSectionSampleOutput {
        section_id: section.id.to_string(),
        page_id: section.page_id,
        source_url: section.source_url,
        source_domain: section.source_domain,
        heading_path: section.heading_path,
        section_type: section.section_type,
        raw_text: section.content_md,
    })
}

pub(crate) async fn seo_preflight_impl(
    acts: &AlegriaActivities,
    input: &SeoPreflightInput,
) -> Result<SeoPreflightOutput, DomainError> {
    sqlx_seo_adapter::ensure_seo_runtime_registries(&acts.pool).await?;
    let scope = seo_domain::identity::derive_scope_from_payload(&input.scope)?;
    let normalized_profile = sqlx_seo_adapter::validate_applicant_profile_reference(
        &acts.pool,
        &scope.applicant_profile,
    )
    .await?;

    let preflight_counts =
        sqlx_seo_adapter::load_seo_preflight_store_counts(&acts.pool, &input.context_key).await?;
    if preflight_counts.context_count == 0 {
        return Err(DomainError::ValidationFailure {
            message: format!("active context is missing for `{}`", input.context_key),
        });
    }
    let required_collections = required_retrieval_collections();
    let collection_statuses =
        sqlx_seo_adapter::read_qdrant_collection_statuses(&acts.pool, &required_collections)
            .await?;
    let required_collection_statuses = collection_statuses
        .into_iter()
        .map(|status| SeoPreflightCollectionStatus {
            exists: status.point_count > 0,
            fresh: status
                .lag_seconds
                .map(|lag| lag * 1000 <= input.projection_max_lag_ms.max(0))
                .unwrap_or(false),
            projection_complete: status.point_count > 0,
            collection_name: status.collection_name,
            point_count: status.point_count,
            last_materialized_at: status.last_materialized_at,
            last_source_change_at: None,
            lag_seconds: status.lag_seconds,
        })
        .collect::<Vec<_>>();
    let qdrant_collection_contract_ready = required_collection_statuses
        .iter()
        .all(|status| status.exists && status.projection_complete);
    let (qdrant_ready, qdrant_collection_presence) =
        probe_qdrant_required_collections(&required_collections).await;
    let required_graph_projection_names = required_graph_projections();
    let graph_projection_statuses = sqlx_seo_adapter::read_graph_projection_statuses(
        &acts.pool,
        &required_graph_projection_names,
    )
    .await?;
    let required_graph_projection_statuses = graph_projection_statuses
        .into_iter()
        .map(|status| SeoPreflightGraphProjectionStatus {
            projection_name: status.artifact_name,
            exists: status.point_count > 0,
            fresh: status
                .lag_seconds
                .map(|lag| lag * 1000 <= input.projection_max_lag_ms.max(0))
                .unwrap_or(false),
            projection_complete: status.point_count > 0,
            point_count: status.point_count,
            last_materialized_at: status.last_materialized_at,
            last_source_change_at: None,
            lag_seconds: status.lag_seconds,
        })
        .collect::<Vec<_>>();
    let graph_projection_contract_ready = required_graph_projection_statuses
        .iter()
        .all(|status| status.exists && status.projection_complete);

    let projection_statuses = sqlx_seo_adapter::read_projection_sync_status(&acts.pool).await?;
    let projection_blocked = projection_statuses.iter().any(|status| {
        status.failed_events > 0
            || (status.open_event_count() > 0
                && status.max_open_lag_ms > input.projection_max_lag_ms)
    });
    let graph_projection_blocked = projection_statuses.iter().any(|status| {
        status.target_system == "neo4j"
            && (status.failed_events > 0
                || (status.open_event_count() > 0
                    && status.max_open_lag_ms > input.projection_max_lag_ms))
    });
    let capability_probe = probe_voyage_capabilities().await;
    let neo4j_probe = probe_neo4j_capabilities().await;
    let voyage_embeddings_ready = capability_probe.embeddings_ready;
    let voyage_contextualized_ready = capability_probe.contextualized_ready;
    let voyage_rerank_ready = capability_probe.rerank_ready;
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
        .filter(|status| !status.exists)
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
    let missing_qdrant_collections = required_collections
        .iter()
        .filter(|collection| {
            !qdrant_collection_presence
                .get(**collection)
                .copied()
                .unwrap_or(false)
        })
        .map(|collection| (*collection).to_string())
        .collect::<Vec<_>>();
    let missing_graph_projections = required_graph_projection_statuses
        .iter()
        .filter(|status| !status.exists)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let stale_graph_projections = required_graph_projection_statuses
        .iter()
        .filter(|status| status.exists && !status.fresh)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let incomplete_graph_projections = required_graph_projection_statuses
        .iter()
        .filter(|status| status.exists && !status.projection_complete)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();

    let retrieval_contract_status = if retrieval_capability_required {
        if !voyage_embeddings_ready
            || (contextual_raw_chunk_retrieval_required && !voyage_contextualized_ready)
            || (voyage_rerank_required && !voyage_rerank_ready)
            || (canonical_vector_retrieval_required && !voyage_embeddings_ready)
        {
            "blocked_provider_capability".to_string()
        } else if !qdrant_ready {
            "blocked_retrieval_contract".to_string()
        } else if !missing_qdrant_collections.is_empty() || !missing_collections.is_empty() {
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
    let retrieval_block_reason = match retrieval_contract_status.as_str() {
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
            "missing required retrieval collections: ledger_missing=[{}] qdrant_missing=[{}]",
            missing_collections.join(","),
            missing_qdrant_collections.join(",")
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

    if retrieval_capability_required && retrieval_contract_status != "pass" {
        return Err(DomainError::ValidationFailure {
            message: retrieval_block_reason
                .clone()
                .unwrap_or_else(|| "retrieval contract blocked canonical runtime".to_string()),
        });
    }
    if graph_capability_required && graph_contract_status != "pass" {
        return Err(DomainError::ValidationFailure {
            message: graph_block_reason
                .clone()
                .unwrap_or_else(|| "graph contract blocked canonical runtime".to_string()),
        });
    }

    Ok(SeoPreflightOutput {
        context_key: input.context_key.clone(),
        normalized_profile,
        page_type_count: preflight_counts.page_type_count,
        page_node_count: preflight_counts.page_node_count,
        navigation_item_count: preflight_counts.navigation_item_count,
        verified_rule_count: preflight_counts.verified_rule_count,
        pending_rule_count: preflight_counts.pending_rule_count,
        qdrant_point_count: preflight_counts.qdrant_point_count,
        voyage_embeddings_ready,
        voyage_contextualized_ready,
        voyage_rerank_ready,
        qdrant_ready,
        qdrant_collection_contract_ready,
        neo4j_ready,
        graph_query_ready,
        graph_gds_ready,
        graph_projection_contract_ready,
        retrieval_capability_required,
        canonical_vector_retrieval_required,
        contextual_raw_chunk_retrieval_required,
        voyage_rerank_required,
        graph_capability_required,
        neo4j_sync_required,
        graph_query_required,
        graph_gds_required,
        retrieval_contract_status: retrieval_contract_status.clone(),
        retrieval_block_reason,
        graph_contract_status: graph_contract_status.clone(),
        graph_block_reason,
        required_collection_statuses,
        required_graph_projection_statuses,
        projection_blocked,
        status: if retrieval_contract_status == "warn" || graph_contract_status == "warn" {
            "warn".to_string()
        } else if retrieval_contract_status == "pass" && graph_contract_status == "pass" {
            if projection_blocked {
                "warn".to_string()
            } else {
                "ok".to_string()
            }
        } else {
            if retrieval_contract_status != "pass" {
                retrieval_contract_status
            } else {
                graph_contract_status
            }
        },
    })
}
