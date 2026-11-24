#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let code = match cli.command {
        Command::LlmNext { database_url } => llm_next(database_url).await?,
        Command::LlmSubmit {
            database_url,
            response_file,
            model_key,
            temporal_url,
            temporal_namespace,
        } => {
            llm_submit(
                database_url,
                &response_file,
                &model_key,
                temporal_url,
                &temporal_namespace,
            )
            .await?
        }
        Command::CheckRustMigrationContract {
            root,
            report_json,
            strict,
        } => check_rust_migration_contract(Path::new(&root), &report_json, strict)?,
        Command::CheckFactVerifierParity { root, report_json } => {
            check_fact_verifier_parity(Path::new(&root), &report_json)?
        }
        Command::ComputeContentHash { text } => {
            println!("{}", primitives::hash::content_hash_v1(&text));
            0
        }
        Command::ComputeBytesHash { hex } => {
            let bytes = hex::decode(&hex).context("invalid hex for ComputeBytesHash")?;
            println!("{}", primitives::hash::blake3_hex(&bytes));
            0
        }
        Command::ComputeStableRuleId { parts } => {
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            println!("{}", primitives::stable_id::stable_rule_instance_id(&refs));
            0
        }
        Command::EncodeFactInput { sections_json } => {
            let bytes = FactExtractionInputPayload { sections_json }.encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::EncodeValidationInput {
            required_links_json,
            required_keys_json,
            used_rule_keys_json,
            used_fact_keys_json,
            url_norm,
        } => {
            let bytes = ValidationInputPayload {
                meta: None,
                required_links_json_utf8: required_links_json.into_bytes(),
                required_keys_json_utf8: required_keys_json.into_bytes(),
                used_rule_keys_json_utf8: used_rule_keys_json.into_bytes(),
                used_fact_keys_json_utf8: used_fact_keys_json.into_bytes(),
                url_norm,
            }
            .encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::ReconcileTargetSystem {
            target_system,
            dry_run,
            max_retry_count,
            batch_limit,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        } => {
            let report = reconcile_target_system_default(
                &target_system,
                &ReconcileOptionsRecord {
                    max_retry_count,
                    batch_limit,
                    dry_run,
                    requeue_base_delay_sec,
                    requeue_jitter_sec,
                },
            )
            .await?;
            println!(
                "{}|{}|{}|{}|{}|{}",
                report.target_system,
                report.dry_run,
                report.stale_candidates,
                report.failed_candidates,
                report.reset_stale_processing,
                report.requeued_failed
            );
            0
        }
        Command::BuildStaticSite {
            database_url,
            output_dir,
            base_url,
        } => build_static_site(database_url, &output_dir, &base_url).await?,
        Command::RawReextractEmpty { database_url, page_id } => {
            raw_reextract_empty(database_url, page_id).await?
        }
        Command::TruthHitlReconcile { database_url, context_key } => {
            truth_hitl_reconcile(database_url, &context_key).await?
        }
        Command::TruthHitlList { database_url, context_key, limit } => {
            truth_hitl_list(database_url, &context_key, limit).await?
        }
        Command::TruthHitlShow { database_url, task_key } => {
            truth_hitl_show(database_url, &task_key).await?
        }
        Command::TruthHitlDecide {
            database_url,
            task_key,
            actor_role,
            decision,
            reason,
        } => {
            truth_hitl_decide(database_url, &task_key, &actor_role, &decision, &reason).await?
        }
        Command::CrawlPendingSources { .. } => {
            return Err(UnsupportedDirectSiteCommand::CrawlPendingSources.into());
        }
        Command::CmsReviewList {
            database_url,
            limit,
        } => cms_review_list(database_url, limit).await?,
        Command::CmsReviewShow {
            database_url,
            page_node_key,
            revision_id,
        } => cms_review_show(database_url, &page_node_key, revision_id.as_deref()).await?,
        Command::CmsPublishStatus {
            database_url,
            page_node_key,
        } => cms_publish_status(database_url, &page_node_key).await?,
        Command::CmsTraceabilityInspect {
            database_url,
            page_node_key,
        } => cms_traceability_inspect(database_url, &page_node_key).await?,
        Command::CmsBlockersInspect {
            database_url,
            page_node_key,
        } => cms_blockers_inspect(database_url, &page_node_key).await?,
        Command::SeoRebuildBacklogInspect {
            database_url,
            page_node_key,
            limit,
        } => seo_rebuild_backlog_inspect(database_url, page_node_key, limit).await?,
        Command::SeoSupportBundleInspect {
            database_url,
            context_key,
        } => seo_support_bundle_inspect(database_url, &context_key).await?,
        Command::SeoPostPublishFeedbackProbe {
            analytics_addr,
            require_gsc,
            report_json,
        } => seo_post_publish_feedback_probe(&analytics_addr, require_gsc, report_json).await?,
        Command::SeoReleaseRestoreGate {
            root,
            run_ci_verify,
            run_temporal_gate,
            run_restore_drill,
            report_json,
        } => seo_release_restore_gate(
            Path::new(&root),
            run_ci_verify,
            run_temporal_gate,
            run_restore_drill,
            report_json,
        )?,
        Command::SeoCutoverShadowVerify {
            database_url,
            legacy_run_id,
            cutover_run_id,
            strict,
            report_json,
        } => {
            seo_cutover_shadow_verify(
                database_url,
                &legacy_run_id,
                &cutover_run_id,
                strict,
                report_json,
            )
            .await?
        }
        Command::CmsApprovePublish {
            database_url,
            temporal_url,
            page_node_key,
            revision_id,
            actor_role,
            reason,
            output_dir,
            base_url,
        } => {
            let _keep_smoke_happy = (&output_dir, &base_url);
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url.clone(),
                temporal_url.as_deref(),
                &page_node_key,
                &revision_id,
                &actor_role,
                "approved",
                &reason,
            )
            .await?;
            println!(
                "CMS_APPROVE_PUBLISH: OK decision={} revision={} workflow={} mode=workflow_owned_publish",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::CmsBlock {
            database_url,
            temporal_url,
            page_node_key,
            revision_id,
            actor_role,
            reason,
        } => {
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url,
                temporal_url.as_deref(),
                &page_node_key,
                &revision_id,
                &actor_role,
                "blocked",
                &reason,
            )
            .await?;
            println!(
                "CMS_BLOCK: OK decision={} revision={} workflow={}",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::CmsReopen {
            database_url,
            temporal_url,
            page_node_key,
            revision_id,
            actor_role,
            reason,
        } => {
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url,
                temporal_url.as_deref(),
                &page_node_key,
                &revision_id,
                &actor_role,
                "reopened",
                &reason,
            )
            .await?;
            println!(
                "CMS_REOPEN: OK decision={} revision={} workflow={}",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::SeoRun { .. } => {
            return Err(UnsupportedDirectSiteCommand::SeoRun.into());
        }
    };

    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code)
    }
}

#[cfg(test)]
mod direct_site_command_cutover_tests {
    use super::*;

    #[test]
    fn previous_operator_commands_remain_discoverable_but_explicitly_disabled() {
        let cases: [(&[&str], UnsupportedDirectSiteCommand); 2] = [
            (&["cli_tools", "seo-run", "full"], UnsupportedDirectSiteCommand::SeoRun),
            (
                &["cli_tools", "crawl-pending-sources"],
                UnsupportedDirectSiteCommand::CrawlPendingSources,
            ),
        ];
        for (args, expected) in cases {
            let parsed = Cli::try_parse_from(args.iter().copied())
                .expect("operator gets a typed CLI command");
            let actual = match parsed.command {
                Command::SeoRun { .. } => UnsupportedDirectSiteCommand::SeoRun,
                Command::CrawlPendingSources { .. } => {
                    UnsupportedDirectSiteCommand::CrawlPendingSources
                }
                _ => panic!("legacy direct mode was silently redirected"),
            };
            assert_eq!(actual, expected);
            let diagnostic = actual.to_string();
            assert!(diagnostic.starts_with("CLI_DIRECT_SITE_COMMAND_DISABLED:"));
            assert!(diagnostic.contains("canonical Temporal workflow operator"));
        }
    }
}
