// One output contract for a newly started or reattached canonical run.
fn report_workflow_completion(
    workflow_type: &str,
    workflow_id: &str,
    result: &str,
    canonical: bool,
) -> Result<()> {
    if canonical {
        if !result.starts_with("seo_site_build_canonical_cutover_ok ") {
            anyhow::bail!("canonical workflow did not produce a successful site: {result}");
        }
        let fields: std::collections::BTreeMap<_, _> = result
            .split_whitespace()
            .filter_map(|part| part.split_once('='))
            .collect();
        if fields.get("run_id").copied() != Some(workflow_id) {
            anyhow::bail!("canonical result workflow ID does not match requested run");
        }
        let published_pages = fields
            .get("published_pages")
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_default();
        let newly_verified = fields
            .get("truth_verified")
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_default();
        // Rework can reuse an already VERIFIED support bundle without
        // extracting any new rules. Both paths still require published pages
        // and nonempty verified truth; a new run with neither fails closed.
        let verified_support = fields
            .get("support_bundle")
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_default();
        if published_pages == 0 || (newly_verified == 0 && verified_support == 0) {
            anyhow::bail!("canonical workflow completed without published pages or verified support");
        }
    }
    println!(
        "WORKFLOW_RESULT workflow_type={} workflow_id={} result={}",
        workflow_type, workflow_id, result
    );
    Ok(())
}

/// A completed source-only V2 run is not a published site. Require its own
/// exact receipt; never apply the canonical page/truth publication gate to it,
/// and never interpret its result as permission to publish content.
fn validate_source_acquisition_receipt(workflow_id: &str, result: &str) -> Result<()> {
    if !result.starts_with("seo_source_acquisition_v2_ok ") {
        anyhow::bail!("V2 source-only workflow did not return a source receipt");
    }
    let fields: std::collections::BTreeMap<_, _> = result
        .split_whitespace()
        .filter_map(|part| part.split_once('='))
        .collect();
    if fields.get("run_id").copied() != Some(workflow_id) {
        anyhow::bail!("V2 source-only receipt run_id mismatch");
    }
    if !matches!(fields.get("source_status").copied(), Some("done" | "partial")) {
        anyhow::bail!("V2 source-only receipt lacks terminal source status");
    }
    if fields
        .get("crawled")
        .and_then(|count| count.parse::<u64>().ok())
        .is_none()
    {
        anyhow::bail!("V2 source-only receipt lacks a valid crawl count");
    }
    Ok(())
}

// Temporal owns workflow lifecycle. PostgreSQL stores a read model, never an
// independent guess about whether a job is still active. Fail closed on any
// inaccessible or unexpected Temporal execution.
async fn reconcile_terminal_site_runs(client: &Client, database_url: Option<String>) -> Result<()> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let rows = sqlx::query(
        "SELECT run_id, workflow_run_id FROM pipeline.execution_runs
         WHERE workflow_type = 'seo_site_build' AND status NOT IN ('done', 'failed')
         ORDER BY created_at, run_id",
    )
    .fetch_all(&pool)
    .await?;
    let mut active = 0usize;
    let mut reconciled = 0usize;
    for row in rows {
        let run_id: Uuid = row.get("run_id");
        let workflow_id: String = row.get("workflow_run_id");
        if run_id.to_string() != workflow_id {
            anyhow::bail!("run_id and Temporal workflow_id differ: {run_id}");
        }
        let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.clone());
        let description = handle
            .describe(WorkflowDescribeOptions::default())
            .await
            .with_context(|| format!("Temporal description unavailable for {workflow_id}"))?;
        let temporal_type = description.workflow_type();
        if description.id() != workflow_id
            || !matches!(
                temporal_type,
                "SeoSiteBuildCanonicalCutoverWorkflow" | "SeoSiteBuildComposableV2Workflow"
            )
        {
            anyhow::bail!("unexpected Temporal workflow identity for {workflow_id}");
        }
        let terminal_status = match description.status() {
            WorkflowClientStatus::Running | WorkflowClientStatus::Paused => None,
            WorkflowClientStatus::Completed => {
                // A canonical V1 completion needs verified pages and truth;
                // a V2 completion needs its distinct source-only receipt.
                let raw = handle
                    .get_result(WorkflowGetResultOptions::default())
                    .await
                    .with_context(|| format!("completed workflow result missing: {workflow_id}"))?;
                let result: String = serde_json::from_slice(
                    &raw.payloads.first().context("completed workflow missing payload")?.data,
                )?;
                let valid_receipt = if temporal_type == "SeoSiteBuildCanonicalCutoverWorkflow" {
                    // Preserve the unchanged canonical publish/truth validation.
                    report_workflow_completion(temporal_type, &workflow_id, &result, true)
                } else {
                    // The same registration table stores a run-mode-specific
                    // input. Verify that the immutable source-only Temporal
                    // type matches its persisted crawl-only scope before
                    // reconciling status; no publication is inferred.
                    let persisted = sqlx_seo_adapter::load_seo_site_build_input(
                        &pool,
                        &workflow_id,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("V2 persisted registration unavailable: {error}"))?;
                    if persisted.run_id != workflow_id || persisted.run_mode != "crawl_only" {
                        anyhow::bail!("V2 Temporal identity does not match persisted crawl_only registration");
                    }
                    validate_source_acquisition_receipt(&workflow_id, &result).and_then(|()| {
                        report_workflow_completion(temporal_type, &workflow_id, &result, false)
                    })
                };
                if valid_receipt.is_ok() {
                    Some("done")
                } else {
                    Some("failed")
                }
            }
            WorkflowClientStatus::Failed
            | WorkflowClientStatus::Canceled
            | WorkflowClientStatus::Terminated
            | WorkflowClientStatus::TimedOut => Some("failed"),
            other => anyhow::bail!("unsupported Temporal status {other:?} for {workflow_id}"),
        };
        if let Some(status) = terminal_status {
            let updated = sqlx_execution_runs_adapter::reconcile_terminal_execution_run(
                &pool, &workflow_id, status,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "RUN_RECONCILE workflow_id={} temporal={:?} status={} updated={}",
                workflow_id, description.status(), status, updated
            );
            reconciled += usize::from(updated);
        } else {
            active += 1;
        }
    }
    println!("RUN_RECONCILE complete active={active} reconciled={reconciled}");
    Ok(())
}

#[cfg(test)]
mod source_only_reconcile_tests {
    use super::validate_source_acquisition_receipt;

    #[test]
    fn exact_source_receipt_is_not_a_published_site_success() {
        let id = "00000000-0000-4000-8000-000000000001";
        let done = format!(
            "seo_source_acquisition_v2_ok run_id={id} source_status=done crawled=2 verified_support=0"
        );
        let partial = format!(
            "seo_source_acquisition_v2_ok run_id={id} source_status=partial crawled=1 verified_support=1"
        );
        assert!(validate_source_acquisition_receipt(id, &done).is_ok());
        assert!(validate_source_acquisition_receipt(id, &partial).is_ok());
        for false_receipt in [
            done.replace(id, "unexpected-run"),
            done.replace("source_status=done", "source_status=pending_retry"),
            done.replace("crawled=2", "crawled=bogus"),
            done.replace("seo_source_acquisition_v2_ok", "seo_site_build_canonical_cutover_ok"),
        ] {
            assert!(validate_source_acquisition_receipt(id, &false_receipt).is_err());
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli {
        temporal_url,
        namespace,
        task_queue,
        command,
    } = Cli::parse();
    let command = match command {
        Command::SeoPreflight {
            database_url,
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            strict_projections,
            projection_max_lag_ms,
            output_dir,
            report_json,
        } => {
            let code = run_seo_preflight(
                database_url,
                context_key,
                market,
                locale,
                country_code,
                visa_type,
                visa_subtype,
                applicant_profile,
                citizenship_code,
                bootstrap_context,
                strict_projections,
                projection_max_lag_ms,
                output_dir,
                report_json,
            )
            .await?;
            std::process::exit(code);
        }
        other => other,
    };
    let command = match command {
        Command::OntologyBackfillPlan {
            database_url,
            concept_key,
            limit,
            apply_neo4j,
            apply_qdrant,
            report_json,
        } => {
            let code = run_ontology_backfill_plan(
                database_url,
                concept_key,
                limit,
                apply_neo4j,
                apply_qdrant,
                report_json,
            )
            .await?;
            std::process::exit(code);
        }
        other => other,
    };

    let identity = format!("alegria-temporal-starter@{}", std::process::id());
    let client = connect_client(&temporal_url, identity, &namespace)
        .await
        .context("failed to connect to Temporal")?;

    match command {
        Command::Ping => {
            println!(
                "ok temporal_url={} namespace={} task_queue={}",
                temporal_url, namespace, task_queue
            );
        }
        Command::Start {
            workflow,
            workflow_id,
            database_url,
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
            source_urls,
            query_batch_key,
            reuse_corpus_from,
            run_mode,
            v2_order,
            wait,
        } => {
            let durable_plan = if workflow == WorkflowKind::SeoSiteBuildComposableV2 {
                if run_mode.trim() != "crawl_only" {
                    anyhow::bail!(
                        "V2 currently supports only --run-mode crawl_only; no publication or content-building steps are enabled"
                    );
                }
                Some(v2_order.unwrap_or(V2Order::SupportFirst).durable_plan()?)
            } else {
                if v2_order.is_some() {
                    anyhow::bail!("--v2-order requires --workflow seo-site-build-composable-v2");
                }
                None
            };
            if matches!(
                workflow,
                WorkflowKind::ExpertDecomposedExtraction
                    | WorkflowKind::ExpertExtraction
                    | WorkflowKind::ExpertProjection
                    | WorkflowKind::ExpertSemanticSlice
            ) && !expert_migration_workflows_enabled()
            {
                anyhow::bail!(
                    "Expert migration workflows are diagnostic-only. Set ALLOW_EXPERT_MIGRATION_WORKFLOWS=true for explicit migration diagnostics, or use SeoSiteBuildCanonicalCutoverWorkflow for canonical execution."
                );
            }
            if reuse_corpus_from.is_some() && workflow != WorkflowKind::SeoSiteBuildCanonicalCutover {
                anyhow::bail!("retained corpus requires the canonical site workflow");
            }
            let wf_id = match workflow_id {
                Some(v) => v,
                None if workflow.requires_uuid_run_id() => {
                    // For fact/content workflows workflow_id is treated as run_id in DB layer.
                    // It must be a UUID string to avoid non-deterministic retry loops.
                    Uuid::new_v4().to_string()
                }
                None => format!("{}-{}", workflow.id_prefix(), Uuid::new_v4()),
            };
            if matches!(
                workflow,
                WorkflowKind::SeoSiteBuildCanonicalCutover
                    | WorkflowKind::SeoSiteBuildComposableV2
                    | WorkflowKind::ExpertDecomposedExtraction
                    | WorkflowKind::ExpertExtraction
                    | WorkflowKind::ExpertProjection
                    | WorkflowKind::ExpertSemanticSlice
            ) {
                persist_seo_site_build_input(
                    database_url,
                    &wf_id,
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
                    source_urls,
                    query_batch_key,
                    reuse_corpus_from,
                    run_mode,
                )
                .await?;
            }
            let options = WorkflowStartOptions::new(task_queue, wf_id.clone()).build();
            // The typed, versioned plan is serialized into Temporal's Workflow
            // Started event. Worker replay therefore repeats the same order.
            let start_payload = match durable_plan {
                Some(plan) => RawValue::from_value(&plan, &PayloadConverter::serde_json()),
                None => empty_payload(),
            };
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    start_payload,
                    options,
                )
                .await
                .with_context(|| format!("failed to start {}", workflow.workflow_type()))?;

            println!(
                "started workflow_type={} workflow_id={} run_id={:?}",
                workflow.workflow_type(),
                handle.info().workflow_id,
                handle.info().run_id
            );
            if wait {
                let result_raw = handle
                    .get_result(WorkflowGetResultOptions::default())
                    .await
                    .context("canonical workflow did not complete successfully")?;
                let first = result_raw
                    .payloads
                    .first()
                    .context("workflow completed without a result payload")?;
                let result: String = serde_json::from_slice(&first.data)
                    .context("workflow result was not a JSON string")?;
                if workflow == WorkflowKind::SeoSiteBuildComposableV2 {
                    // --wait is an operator-facing success boundary too:
                    // source-only V2 must never report arbitrary payloads as
                    // successful completions just because Temporal finished.
                    validate_source_acquisition_receipt(&wf_id, &result)?;
                }
                report_workflow_completion(
                    workflow.workflow_type(),
                    &wf_id,
                    &result,
                    workflow == WorkflowKind::SeoSiteBuildCanonicalCutover,
                )?;
            }
        }
        Command::SeoPreflight { .. } => {
            unreachable!("SeoPreflight is handled before Temporal connect")
        }
        Command::RebuildDispatch {
            database_url,
            limit,
            dry_run,
            report_json,
        } => {
            let code = run_rebuild_dispatch(
                &client,
                &task_queue,
                database_url,
                limit,
                dry_run,
                report_json,
            )
            .await?;
            std::process::exit(code);
        }
        Command::OntologyBackfillPlan { .. } => {
            unreachable!("OntologyBackfillPlan is handled before Temporal connect")
        }
        Command::DemoHitl {
            workflow_id,
            wait_before_resume_ms,
        } => {
            let wf_id = workflow_id.unwrap_or_else(|| format!("test-hitl-{}", Uuid::new_v4()));
            let options = WorkflowStartOptions::new(task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new("TestHitlWorkflow"),
                    empty_payload(),
                    options,
                )
                .await
                .context("failed to start TestHitlWorkflow")?;

            tokio::time::sleep(Duration::from_millis(wait_before_resume_ms)).await;
            handle
                .signal(
                    UntypedSignal::<UntypedWorkflow>::new("resume"),
                    empty_payload(),
                    WorkflowSignalOptions::default(),
                )
                .await
                .context("failed to send resume signal")?;

            let result_raw = handle
                .get_result(WorkflowGetResultOptions::default())
                .await
                .context("failed to get workflow result")?;
            println!(
                "demo_hitl_ok workflow_id={} run_id={:?} result_payloads={}",
                handle.info().workflow_id,
                handle.info().run_id,
                result_raw.payloads.len()
            );
        }
        Command::WorkflowWait { workflow_id } => {
            // A terminated operator terminal must never create another workflow.
            Uuid::parse_str(&workflow_id).context("existing canonical run_id must be a UUID")?;
            let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.clone());
            let result_raw = handle
                .get_result(WorkflowGetResultOptions::default())
                .await
                .context("failed to await existing canonical workflow")?;
            let first = result_raw
                .payloads
                .first()
                .context("existing workflow completed without a result payload")?;
            let result: String = serde_json::from_slice(&first.data)
                .context("existing workflow result was not a JSON string")?;
            report_workflow_completion(
                "SeoSiteBuildCanonicalCutoverWorkflow",
                &workflow_id,
                &result,
                true,
            )?;
        }
        Command::RunReconcile { database_url } => {
            reconcile_terminal_site_runs(&client, database_url).await?;
        }
        Command::WorkflowResume { workflow_id } => {
            let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.clone());
            handle
                .signal(
                    UntypedSignal::<UntypedWorkflow>::new("resume"),
                    empty_payload(),
                    WorkflowSignalOptions::default(),
                )
                .await
                .with_context(|| format!("failed to send resume signal to {workflow_id}"))?;
            println!("resume_sent workflow_id={workflow_id}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_or_empty_canonical_reconnection_result() {
        let id = "9c337079-ae0b-4cb4-b052-bb3a0ca623c1";
        let good = format!(
            "seo_site_build_canonical_cutover_ok run_id={id} ia_pages=1 published_pages=1 truth_verified=2"
        );
        assert!(report_workflow_completion(
            "SeoSiteBuildCanonicalCutoverWorkflow",
            id,
            &good,
            true
        )
        .is_ok());
        // A canonical rework run may reuse seven previously verified facts
        // while correctly reporting zero *new* fact verifications.
        let reused = format!(
            "seo_site_build_canonical_cutover_ok run_id={id} support_bundle=7 ia_pages=5 published_pages=5 truth_verified=0"
        );
        assert!(report_workflow_completion(
            "SeoSiteBuildCanonicalCutoverWorkflow",
            id,
            &reused,
            true
        )
        .is_ok());
        for result in [
            format!("seo_site_build_canonical_cutover_ok run_id=wrong ia_pages=1 published_pages=1 truth_verified=2"),
            format!("seo_site_build_canonical_cutover_ok run_id={id} ia_pages=1 published_pages=0 truth_verified=2"),
            format!("seo_site_build_canonical_cutover_ok run_id={id} ia_pages=1 published_pages=1 truth_verified=0"),
            format!("seo_site_build_canonical_cutover_ok run_id={id} support_bundle=0 published_pages=5 truth_verified=0"),
            format!("seo_site_build_canonical_cutover_ok run_id={id} support_bundle=7 published_pages=0 truth_verified=0"),
            "blocked:incomplete_verified_truth".to_string(),
        ] {
            assert!(report_workflow_completion(
                "SeoSiteBuildCanonicalCutoverWorkflow",
                id,
                &result,
                true
            )
            .is_err());
        }
    }

    #[test]
    fn normalizes_supported_applicant_profile() {
        let profile = identity::normalize_applicant_profile(" Standard ").unwrap();
        assert_eq!(profile, "standard");
    }

    #[test]
    fn rejects_unknown_applicant_profile() {
        let err = identity::normalize_applicant_profile("default-applicant")
            .expect_err("unknown profile must fail");
        let message = err.to_string();
        assert!(message.contains("unsupported applicant_profile"));
    }

    #[test]
    fn applicant_profile_does_not_change_truth_identity_without_explicit_subtype() {
        let standard = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let minor = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        assert_eq!(standard.context_key, minor.context_key);
    }

    #[test]
    fn explicit_regulatory_subtype_changes_truth_identity() {
        let base = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let subtype =
            identity::derive_truth_identity("ES", "tourist", Some("priority_track"), "BY").unwrap();
        assert_ne!(base.context_key, subtype.context_key);
    }

    #[test]
    fn locale_and_profile_change_scope_not_truth_identity() {
        let truth = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let scope_ru =
            identity::derive_scope("alegria-site", "ru-RU", "ES", "tourist", "standard").unwrap();
        let scope_en =
            identity::derive_scope("alegria-site", "en", "ES", "tourist", "minor").unwrap();
        assert_eq!(
            truth.context_key,
            identity::derive_truth_identity("ES", "tourist", None, "BY")
                .unwrap()
                .context_key
        );
        assert_ne!(scope_ru.scope_signature, scope_en.scope_signature);
    }
}
