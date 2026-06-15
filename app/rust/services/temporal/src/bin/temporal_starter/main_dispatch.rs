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
            query_batch_key,
            run_mode,
        } => {
            if workflow == WorkflowKind::ContentGeneration {
                anyhow::bail!(
                    "ContentGenerationWorkflow is legacy-only. Use SeoSiteBuildCanonicalCutoverWorkflow for production SEO generation."
                );
            }
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
                WorkflowKind::SeoSiteBuild
                    | WorkflowKind::SeoSiteBuildCanonicalCutover
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
                    query_batch_key,
                    run_mode,
                )
                .await?;
            }
            let options = WorkflowStartOptions::new(task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    empty_payload(),
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
        }
        Command::SeoPreflight { .. } => {
            unreachable!("SeoPreflight is handled before Temporal connect")
        }
        Command::RebuildDispatch {
            database_url,
            limit,
            dry_run,
            workflow,
            report_json,
        } => {
            let code = run_rebuild_dispatch(
                &client,
                &task_queue,
                database_url,
                limit,
                dry_run,
                workflow,
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
