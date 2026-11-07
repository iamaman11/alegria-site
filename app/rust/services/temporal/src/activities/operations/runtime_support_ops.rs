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

/// Keep the canonical production contract explicit: an absent flag is not an
/// implicit permission to skip graph/retrieval gates.
fn missing_required_contract_flags(flags: &[(&'static str, bool)]) -> Vec<&'static str> {
    flags
        .iter()
        .filter_map(|(name, enabled)| (!enabled).then_some(*name))
        .collect()
}

/// A zero-point result is admissible for these two optional outputs only
/// after the exact run has durably completed their producing activities.
fn optional_projection_valid_empty(
    name: &str,
    source_count: i64,
    provider_count: i64,
    completed_steps: &[String],
) -> bool {
    if source_count != 0 || provider_count != 0 {
        return false;
    }
    let completed = |step: &str| completed_steps.iter().any(|name| name == step);
    match name {
        "content_gap" => completed("opportunity_build") && completed("global_site_reconcile"),
        "link_recommendation" => {
            completed("link_recommend") && completed("global_site_reconcile")
        }
        _ => false,
    }
}

/// The last graph projection is generated during draft_assemble, not IA.
fn graph_projection_due_at_gate(name: &str, require_page_brief_projection: bool) -> bool {
    name != "page_brief" || require_page_brief_projection
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
    // The initial gate probes capabilities only. Materialization gates also
    // read actual Qdrant points for THIS scope and run, including per-page
    // gates; skip_provider_probes skips costly Voyage/Neo4j probes, not truth.
    let (qdrant_ready, qdrant_collection_presence, scoped_point_counts) =
        if input.skip_provider_probes && !input.require_materialized_projections {
            (false, Default::default(), Default::default())
        } else {
            probe_qdrant_required_collections(
                &required_collections,
                &scope.scope_signature,
                &input.run_id,
                input.require_materialized_projections,
                !input.skip_provider_probes,
            )
            .await
        };
    let collection_statuses =
        sqlx_seo_adapter::read_qdrant_collection_statuses(&acts.pool, &required_collections)
            .await?;
    let required_collection_statuses = collection_statuses
        .into_iter()
        .map(|status| {
            // Scope-dependent SEO collections are certified by the Qdrant
            // provider, NOT by the global kb.qdrant_points SQL ledger.
            let current_run_count = scoped_point_counts.get(&status.collection_name).copied();
            let point_count = effective_collection_point_count(status.point_count, current_run_count);
            let current_run_verified = current_run_count.is_some();
            SeoPreflightCollectionStatus {
                exists: point_count > 0,
                fresh: if current_run_verified {
                    // These points carry this run's identity in Qdrant.
                    point_count > 0
                } else {
                    status
                        .lag_seconds
                        .map(|lag| lag * 1000 <= input.projection_max_lag_ms.max(0))
                        .unwrap_or(false)
                },
                projection_complete: point_count > 0,
                collection_name: status.collection_name,
                point_count,
                // A Qdrant count does not expose a materialization timestamp.
                last_materialized_at: if current_run_verified {
                    None
                } else {
                    status.last_materialized_at
                },
                last_source_change_at: None,
                lag_seconds: if current_run_verified {
                    None
                } else {
                    status.lag_seconds
                },
            }
        })
        .collect::<Vec<_>>();
    let qdrant_collection_contract_ready = required_collection_statuses
        .iter()
        .all(|status| status.exists && status.projection_complete);
    let required_graph_projection_names = required_graph_projections();
    let graph_readback_names = required_graph_projection_names
        .iter()
        .copied()
        .filter(|name| {
            graph_projection_due_at_gate(name, input.require_page_brief_projection)
                && (!input.require_page_brief_projection || *name == "page_brief")
        })
        .collect::<Vec<_>>();
    // The post-IA barrier validates all due graph projections. Each later
    // page barrier only rechecks PageBrief; the prior full barrier already ran.
    let actual_graph_counts = if input.require_materialized_projections {
        read_neo4j_scope_projection_counts(&scope.scope_signature, &graph_readback_names).await?
    } else {
        Default::default()
    };
    let completed_producers = if input.require_materialized_projections {
        sqlx_seo_adapter::read_completed_seo_producers(&acts.pool, &input.run_id).await?
    } else {
        Vec::new()
    };
    let graph_projection_statuses = sqlx_seo_adapter::read_graph_projection_statuses_for_scope(
        &acts.pool,
        &required_graph_projection_names,
        &scope.scope_signature,
    )
    .await?;
    let required_graph_projection_statuses = graph_projection_statuses
        .into_iter()
        .map(|status| {
            let source_count = status.point_count;
            let actual_count = actual_graph_counts
                .get(&status.artifact_name)
                .copied()
                .unwrap_or(source_count);
            let valid_empty = input.require_materialized_projections
                && optional_projection_valid_empty(
                    &status.artifact_name,
                    source_count,
                    actual_count,
                    &completed_producers,
                );
            let complete = source_count > 0 && actual_count >= source_count;
            let materialization_outcome = if valid_empty {
                "valid_empty"
            } else if source_count <= 0 || actual_count <= 0 {
                "missing"
            } else if !complete {
                "incomplete"
            } else {
                "materialized"
            };
            SeoPreflightGraphProjectionStatus {
                projection_name: status.artifact_name,
                materialization_outcome: materialization_outcome.to_string(),
                exists: source_count > 0 && actual_count > 0,
                fresh: valid_empty
                    || status
                        .lag_seconds
                        .map(|lag| lag * 1000 <= input.projection_max_lag_ms.max(0))
                        .unwrap_or(false),
                projection_complete: valid_empty || complete,
                point_count: actual_count,
                last_materialized_at: status.last_materialized_at,
                last_source_change_at: None,
                lag_seconds: status.lag_seconds,
            }
        })
        .collect::<Vec<_>>();
    // A page brief is produced by draft_assemble, *after* IA planning.
    // Do not demand it at the post-planning gate; require it at the
    // per-page post-draft gate before editorial/publish.
    let applicable_graph_projections = required_graph_projection_statuses
        .iter()
        .filter(|status| {
            graph_projection_due_at_gate(&status.projection_name, input.require_page_brief_projection)
        });
    let graph_projection_contract_ready = applicable_graph_projections
        .clone()
        .all(|status| {
            status.projection_complete
                && (status.exists || status.materialization_outcome == "valid_empty")
        });

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
    let capability_probe = if input.skip_provider_probes {
        VoyageCapabilityProbe::default()
    } else {
        probe_voyage_capabilities().await
    };
    let neo4j_probe = if input.skip_provider_probes {
        Neo4jCapabilityProbe::default()
    } else {
        probe_neo4j_capabilities().await
    };
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

    if input.enforce_required_contract {
        let missing = missing_required_contract_flags(&[
            ("RETRIEVAL_CAPABILITY_REQUIRED", retrieval_capability_required),
            ("CANONICAL_VECTOR_RETRIEVAL_REQUIRED", canonical_vector_retrieval_required),
            ("CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED", contextual_raw_chunk_retrieval_required),
            ("VOYAGE_RERANK_REQUIRED", voyage_rerank_required),
            ("GRAPH_CAPABILITY_REQUIRED", graph_capability_required),
            ("NEO4J_SYNC_REQUIRED", neo4j_sync_required),
            ("GRAPH_QUERY_REQUIRED", graph_query_required),
            ("GRAPH_GDS_REQUIRED", graph_gds_required),
        ]);
        if !missing.is_empty() {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "canonical production preflight requires enabled flags: {}",
                    missing.join(", ")
                ),
            });
        }
    }

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
    let missing_graph_projections = applicable_graph_projections
        .clone()
        .filter(|status| !status.exists && status.materialization_outcome != "valid_empty")
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let stale_graph_projections = applicable_graph_projections
        .clone()
        .filter(|status| status.exists && !status.fresh)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();
    let incomplete_graph_projections = applicable_graph_projections
        .filter(|status| status.exists && !status.projection_complete)
        .map(|status| status.projection_name.clone())
        .collect::<Vec<_>>();

    let retrieval_contract_status = if retrieval_capability_required {
        if !input.skip_provider_probes
            && (!voyage_embeddings_ready
                || (contextual_raw_chunk_retrieval_required && !voyage_contextualized_ready)
                || (voyage_rerank_required && !voyage_rerank_ready)
                || (canonical_vector_retrieval_required && !voyage_embeddings_ready))
        {
            "blocked_provider_capability".to_string()
        } else if !input.skip_provider_probes && !qdrant_ready {
            "blocked_retrieval_contract".to_string()
        } else if input.require_materialized_projections
            && ((!input.skip_provider_probes && !missing_qdrant_collections.is_empty())
                || !missing_collections.is_empty())
        {
            "blocked_missing_collection".to_string()
        } else if input.require_materialized_projections && !stale_collections.is_empty() {
            "blocked_stale_collection".to_string()
        } else if input.require_materialized_projections
            && (projection_blocked || !incomplete_collections.is_empty())
        {
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
        if !input.skip_provider_probes
            && ((neo4j_sync_required && !neo4j_ready)
                || (graph_query_required && !graph_query_ready)
                || (graph_gds_required && !graph_gds_ready))
        {
            "blocked_provider_capability".to_string()
        } else if input.require_materialized_projections && !missing_graph_projections.is_empty() {
            "blocked_missing_projection".to_string()
        } else if input.require_materialized_projections && !stale_graph_projections.is_empty() {
            "blocked_stale_projection".to_string()
        } else if input.require_materialized_projections
            && (graph_projection_blocked || !incomplete_graph_projections.is_empty())
        {
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
        provider_probes_skipped: input.skip_provider_probes,
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

#[cfg(test)]
mod canonical_production_contract_tests {
    use super::{
        graph_projection_due_at_gate, missing_required_contract_flags,
        optional_projection_valid_empty,
    };

    #[test]
    fn valid_empty_requires_current_run_producer_evidence() {
        let completed = vec![
            "opportunity_build".to_string(),
            "link_recommend".to_string(),
            "global_site_reconcile".to_string(),
        ];
        assert!(optional_projection_valid_empty("content_gap", 0, 0, &completed));
        assert!(optional_projection_valid_empty(
            "link_recommendation",
            0,
            0,
            &completed
        ));
        assert!(!optional_projection_valid_empty("page_node", 0, 0, &completed));
        assert!(!optional_projection_valid_empty("content_gap", 1, 0, &completed));
        assert!(!optional_projection_valid_empty(
            "content_gap",
            0,
            1,
            &completed
        ));
        assert!(!optional_projection_valid_empty(
            "link_recommendation",
            0,
            0,
            &["link_recommend".to_string()]
        ));
        assert!(!optional_projection_valid_empty(
            "content_gap",
            0,
            0,
            &["global_site_reconcile".to_string()]
        ));
    }

    #[test]
    fn required_capabilities_must_be_explicit() {
        let missing = missing_required_contract_flags(&[
            ("GRAPH_CAPABILITY_REQUIRED", true),
            ("RETRIEVAL_CAPABILITY_REQUIRED", false),
            ("GRAPH_GDS_REQUIRED", false),
        ]);
        assert_eq!(
            missing,
            vec!["RETRIEVAL_CAPABILITY_REQUIRED", "GRAPH_GDS_REQUIRED"]
        );
        assert!(missing_required_contract_flags(&[
            ("GRAPH_CAPABILITY_REQUIRED", true),
            ("RETRIEVAL_CAPABILITY_REQUIRED", true),
        ])
        .is_empty());
    }

    #[test]
    fn future_brief_is_not_due_at_ia_gate_but_is_due_after_draft_assembly() {
        assert!(graph_projection_due_at_gate("page_node", false));
        assert!(graph_projection_due_at_gate("link_recommendation", false));
        assert!(!graph_projection_due_at_gate("page_brief", false));
        assert!(graph_projection_due_at_gate("page_brief", true));
    }
}
