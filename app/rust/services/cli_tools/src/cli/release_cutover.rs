fn seo_release_restore_gate(
    root: &Path,
    run_ci_verify: bool,
    run_temporal_gate: bool,
    run_restore_drill: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let required_paths = [
        "automation/ci_verify.sh",
        "automation/temporal_production_gate.sh",
        "infra/backups/restore_drill.sh",
        "automation/check_temporal_build_id_policy.py",
        "automation/check_seo_rollout_compat_contract.py",
        "automation/check_backup_restore_layout.py",
    ];
    let mut blocking_reasons = Vec::new();
    let mut missing_paths = Vec::new();
    for rel in required_paths {
        if !root.join(rel).exists() {
            missing_paths.push(rel.to_string());
            blocking_reasons.push(format!("missing required gate path: {rel}"));
        }
    }

    let build_id_policy = run_gate_command(
        root,
        "check_temporal_build_id_policy",
        "python3",
        &["automation/check_temporal_build_id_policy.py"],
        true,
    )?;
    evaluate_gate_status(&build_id_policy, &mut blocking_reasons);

    let rollout_compat = run_gate_command(
        root,
        "check_seo_rollout_compat_contract",
        "python3",
        &["automation/check_seo_rollout_compat_contract.py"],
        true,
    )?;
    evaluate_gate_status(&rollout_compat, &mut blocking_reasons);

    let backup_restore_layout = run_gate_command(
        root,
        "check_backup_restore_layout",
        "python3",
        &["automation/check_backup_restore_layout.py"],
        true,
    )?;
    evaluate_gate_status(&backup_restore_layout, &mut blocking_reasons);

    let ci_verify = run_gate_command(
        root,
        "ci_verify",
        "bash",
        &["automation/ci_verify.sh"],
        run_ci_verify,
    )?;
    evaluate_gate_status(&ci_verify, &mut blocking_reasons);

    let temporal_gate = run_gate_command(
        root,
        "temporal_production_gate",
        "bash",
        &["automation/temporal_production_gate.sh"],
        run_temporal_gate,
    )?;
    evaluate_gate_status(&temporal_gate, &mut blocking_reasons);

    let restore_drill = run_gate_command(
        root,
        "restore_drill",
        "bash",
        &["infra/backups/restore_drill.sh"],
        run_restore_drill,
    )?;
    evaluate_gate_status(&restore_drill, &mut blocking_reasons);

    let status = if blocking_reasons.is_empty() {
        "ok"
    } else {
        "blocked"
    };
    let payload = json!({
        "status": status,
        "release_gate": "release_and_restore_gate",
        "root": root.display().to_string(),
        "executed": {
            "run_ci_verify": run_ci_verify,
            "run_temporal_gate": run_temporal_gate,
            "run_restore_drill": run_restore_drill,
        },
        "required_paths": required_paths,
        "missing_paths": missing_paths,
        "blocking_reasons": blocking_reasons,
        "checks": {
            "build_id_policy": build_id_policy,
            "rollout_compat": rollout_compat,
            "backup_restore_layout": backup_restore_layout,
            "ci_verify": ci_verify,
            "temporal_production_gate": temporal_gate,
            "restore_drill": restore_drill,
        }
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(root, report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if status == "ok" { 0 } else { 2 })
}

async fn seo_cutover_shadow_verify(
    database_url: Option<String>,
    legacy_run_id: &str,
    cutover_run_id: &str,
    strict: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;

    let legacy_crawl = load_latest_step_value(&pool, legacy_run_id, "crawl_sources", "output")
        .await?
        .context("legacy run missing crawl_sources output")?;
    let legacy_raw_ingestion =
        load_latest_step_value(&pool, legacy_run_id, "raw_knowledge_ingestion", "output")
            .await?
            .context("legacy run missing raw_knowledge_ingestion output")?;
    let legacy_raw_ingestion_input =
        load_latest_step_value(&pool, legacy_run_id, "raw_knowledge_ingestion", "input")
            .await?
            .context("legacy run missing raw_knowledge_ingestion input")?;

    let cutover_raw_evidence =
        load_latest_step_value(&pool, cutover_run_id, "raw_evidence_register", "output")
            .await?
            .context("cutover run missing raw_evidence_register output")?;
    let cutover_candidate_validation =
        load_latest_step_value(&pool, cutover_run_id, "candidate_validation", "output")
            .await?
            .context("cutover run missing candidate_validation output")?;
    let cutover_truth_adjudication =
        load_latest_step_value(&pool, cutover_run_id, "truth_adjudication", "output")
            .await?
            .context("cutover run missing truth_adjudication output")?;
    let cutover_verified_write =
        load_latest_step_value(&pool, cutover_run_id, "verified_truth_write", "output")
            .await?
            .context("cutover run missing verified_truth_write output")?;
    let cutover_contradiction_gate =
        load_latest_step_value(&pool, cutover_run_id, "contradiction_gate", "output")
            .await?
            .context("cutover run missing contradiction_gate output")?;
    let cutover_projection_barrier = load_latest_step_value(
        &pool,
        cutover_run_id,
        "projection_barrier(semantic_projection)",
        "output",
    )
    .await?
    .context("cutover run missing projection_barrier(semantic_projection) output")?;
    let cutover_candidate_validation_input =
        load_latest_step_value(&pool, cutover_run_id, "candidate_validation", "input")
            .await?
            .context("cutover run missing candidate_validation input")?;

    let legacy_context_key = value_as_str(&legacy_raw_ingestion_input, "context_key")
        .map(ToOwned::to_owned)
        .context("legacy raw_knowledge_ingestion input missing context_key")?;
    let cutover_context_key = value_as_str(&cutover_candidate_validation_input, "context_key")
        .map(ToOwned::to_owned)
        .context("cutover candidate_validation input missing context_key")?;
    let legacy_raw_page_ids = value_as_i64_vec(&legacy_crawl, "raw_page_ids");
    let cutover_raw_page_ids =
        value_as_i64_vec(&cutover_candidate_validation_input, "raw_page_ids");

    let legacy_candidate_counts =
        load_candidate_status_counts(&pool, &legacy_context_key, &legacy_raw_page_ids).await?;
    let cutover_projection_status = projection_status_value(&pool, cutover_run_id).await?;
    let legacy_projection_status = projection_status_value(&pool, legacy_run_id).await?;
    let legacy_publish_events = cms_publish_event_summary(&pool, legacy_run_id).await?;
    let cutover_publish_events = cms_publish_event_summary(&pool, cutover_run_id).await?;
    let cutover_step_statuses = list_run_step_statuses(&pool, cutover_run_id).await?;
    let legacy_step_statuses = list_run_step_statuses(&pool, legacy_run_id).await?;
    let cutover_step_counts = step_execution_counts(&pool, cutover_run_id).await?;
    let legacy_step_counts = step_execution_counts(&pool, legacy_run_id).await?;

    let mut findings = Vec::new();
    if legacy_context_key != cutover_context_key {
        findings.push(error_finding(
            "SHADOW_CONTEXT_MISMATCH",
            format!(
                "legacy context_key `{legacy_context_key}` does not match cutover context_key `{cutover_context_key}`"
            ),
        ));
    }
    if legacy_raw_page_ids != cutover_raw_page_ids {
        findings.push(warning_finding(
            "SHADOW_RAW_PAGE_SCOPE_DIFF",
            format!(
                "legacy raw_page_ids ({}) and cutover raw_page_ids ({}) differ",
                legacy_raw_page_ids.len(),
                cutover_raw_page_ids.len()
            ),
        ));
    }

    let missing_cutover_steps = CUTOVER_PHASE_M1_LEDGER_STEPS
        .iter()
        .filter(|step_name| cutover_step_statuses.get(**step_name) != Some(&"done".to_string()))
        .map(|step_name| (*step_name).to_string())
        .collect::<Vec<_>>();
    if !missing_cutover_steps.is_empty() {
        findings.push(error_finding(
            "SHADOW_CUTOVER_STEP_COVERAGE",
            format!(
                "cutover run is missing completed ledger steps: {}",
                missing_cutover_steps.join(", ")
            ),
        ));
    }

    let legacy_verified = value_as_u64(&legacy_raw_ingestion, "verified_rule_count").unwrap_or(0);
    let legacy_changed_truth_keys =
        value_as_string_vec(&legacy_raw_ingestion, "changed_truth_keys").len() as u64;
    let legacy_needs_hitl = value_as_i64(&legacy_candidate_counts, "needs_hitl_count")
        .unwrap_or_default()
        .max(0) as u64;

    let cutover_verified =
        value_as_u64(&cutover_verified_write, "verified_rule_count").unwrap_or(0);
    let cutover_changed_truth_keys =
        value_as_string_vec(&cutover_verified_write, "changed_truth_keys").len() as u64;
    let cutover_needs_hitl =
        value_as_u64(&cutover_candidate_validation, "needs_hitl_count").unwrap_or(0);
    let cutover_contradictions =
        value_as_u64(&cutover_contradiction_gate, "conflict_count").unwrap_or(0);

    if cutover_verified > legacy_verified {
        findings.push(error_finding(
            "SHADOW_VERIFIED_INCREASE_UNJUSTIFIED",
            format!(
                "cutover verified_rule_count {} exceeds legacy verified_rule_count {}",
                cutover_verified, legacy_verified
            ),
        ));
    } else if cutover_verified < legacy_verified {
        findings.push(warning_finding(
            "SHADOW_VERIFIED_COUNT_LOWER",
            format!(
                "cutover verified_rule_count {} is lower than legacy verified_rule_count {}",
                cutover_verified, legacy_verified
            ),
        ));
    }

    if cutover_needs_hitl < legacy_needs_hitl {
        findings.push(warning_finding(
            "SHADOW_NEEDS_HITL_LOWER",
            format!(
                "cutover needs_hitl_count {} is lower than legacy needs_hitl_count {}; verify that no ambiguity was silently accepted",
                cutover_needs_hitl, legacy_needs_hitl
            ),
        ));
    }

    let barrier_blocked_events =
        value_as_i64(&cutover_projection_barrier, "blocked_events").unwrap_or_default();
    let barrier_status = value_as_str(&cutover_projection_barrier, "status")
        .unwrap_or("unknown")
        .to_string();
    if barrier_status != "clear" || barrier_blocked_events > 0 {
        findings.push(error_finding(
            "SHADOW_PROJECTION_BARRIER_BLOCKED",
            format!(
                "cutover semantic projection barrier status={} blocked_events={}",
                barrier_status, barrier_blocked_events
            ),
        ));
    }

    let cutover_truth_verified =
        value_as_u64(&cutover_truth_adjudication, "verified_count").unwrap_or(0);
    if cutover_truth_verified != cutover_verified {
        findings.push(warning_finding(
            "SHADOW_TRUTH_WRITE_DELTA",
            format!(
                "truth_adjudication verified_count {} differs from verified_truth_write verified_rule_count {}",
                cutover_truth_verified, cutover_verified
            ),
        ));
    }

    let cutover_m2_observed =
        value_as_i64(&cutover_publish_events, "review_requested_events").unwrap_or_default() > 0
            || CUTOVER_PHASE_M2_LEDGER_STEPS.iter().any(|step_name| {
                cutover_step_statuses.get(*step_name) == Some(&"done".to_string())
            });
    let legacy_m2_observed =
        value_as_i64(&legacy_publish_events, "review_requested_events").unwrap_or_default() > 0
            || LEGACY_PHASE_M2_LEDGER_STEPS
                .iter()
                .any(|step_name| legacy_step_statuses.get(*step_name) == Some(&"done".to_string()));

    let m1_raw_scope_required = !(legacy_m2_observed
        && cutover_m2_observed
        && legacy_raw_page_ids.is_empty()
        && cutover_raw_page_ids.is_empty());
    if legacy_raw_page_ids.is_empty() && m1_raw_scope_required {
        findings.push(error_finding(
            "SHADOW_LEGACY_RAW_PAGES_MISSING",
            "legacy crawl_sources output has no raw_page_ids",
        ));
    }
    if cutover_raw_page_ids.is_empty() && m1_raw_scope_required {
        findings.push(error_finding(
            "SHADOW_CUTOVER_RAW_PAGES_MISSING",
            "cutover candidate_validation input has no raw_page_ids",
        ));
    }

    let cutover_missing_m2_steps = if cutover_m2_observed {
        CUTOVER_PHASE_M2_LEDGER_STEPS
            .iter()
            .filter(|step_name| cutover_step_statuses.get(**step_name) != Some(&"done".to_string()))
            .map(|step_name| (*step_name).to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let legacy_missing_m2_steps = if legacy_m2_observed {
        LEGACY_PHASE_M2_LEDGER_STEPS
            .iter()
            .filter(|step_name| legacy_step_statuses.get(**step_name) != Some(&"done".to_string()))
            .map(|step_name| (*step_name).to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let legacy_review_requested_pages = *legacy_step_counts
        .get("load_cms_approval_decision")
        .unwrap_or(&0);
    let legacy_approved_pages = *legacy_step_counts.get("finalize_publish").unwrap_or(&0);
    let cutover_review_requested_pages =
        *cutover_step_counts.get("human_approval_wait").unwrap_or(&0);
    let cutover_approved_pages = *cutover_step_counts.get("finalize_publish").unwrap_or(&0);
    let legacy_blocked_pages =
        value_as_i64(&legacy_publish_events, "blocked_pages").unwrap_or_default();
    let cutover_blocked_pages =
        value_as_i64(&cutover_publish_events, "blocked_pages").unwrap_or_default();

    if cutover_m2_observed {
        if !cutover_missing_m2_steps.is_empty() {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_STEP_COVERAGE",
                format!(
                    "cutover run is missing completed Phase M2 steps: {}",
                    cutover_missing_m2_steps.join(", ")
                ),
            ));
        }
        if cutover_review_requested_pages == 0 {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_NO_REVIEW_REQUESTS",
                "cutover run emitted no seo_page_review_requested events",
            ));
        }
        if cutover_blocked_pages > 0 {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_BLOCKED_PAGES",
                format!(
                    "cutover run still has {} blocked publish pages",
                    cutover_blocked_pages
                ),
            ));
        }
        if cutover_approved_pages != cutover_review_requested_pages {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_APPROVAL_COUNT_MISMATCH",
                format!(
                    "cutover approved_pages {} differs from review_requested_pages {}",
                    cutover_approved_pages, cutover_review_requested_pages
                ),
            ));
        }
    }

    if legacy_m2_observed {
        if !legacy_missing_m2_steps.is_empty() {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_STEP_COVERAGE",
                format!(
                    "legacy run is missing completed Phase M2 steps: {}",
                    legacy_missing_m2_steps.join(", ")
                ),
            ));
        }
        if legacy_review_requested_pages == 0 {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_NO_REVIEW_REQUESTS",
                "legacy run emitted no seo_page_review_requested events",
            ));
        }
        if legacy_blocked_pages > 0 {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_BLOCKED_PAGES",
                format!(
                    "legacy run still has {} blocked publish pages",
                    legacy_blocked_pages
                ),
            ));
        }
        if legacy_approved_pages != legacy_review_requested_pages {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_APPROVAL_COUNT_MISMATCH",
                format!(
                    "legacy approved_pages {} differs from review_requested_pages {}",
                    legacy_approved_pages, legacy_review_requested_pages
                ),
            ));
        }
    }

    if legacy_m2_observed && cutover_m2_observed && legacy_approved_pages != cutover_approved_pages
    {
        findings.push(error_finding(
            "SHADOW_PHASE_M2_PUBLISHED_PAGE_COUNT_DIFF",
            format!(
                "legacy approved_pages {} differs from cutover approved_pages {}",
                legacy_approved_pages, cutover_approved_pages
            ),
        ));
    }

    let status = status_from_findings(&findings, strict);
    let payload = json!({
        "status": status,
        "strict": strict,
        "legacy_run_id": legacy_run_id,
        "cutover_run_id": cutover_run_id,
        "context_key": {
            "legacy": legacy_context_key,
            "cutover": cutover_context_key,
        },
        "phase_m1": {
            "raw_scope_required_for_verdict": m1_raw_scope_required,
            "expected_ledger_steps": CUTOVER_PHASE_M1_LEDGER_STEPS,
            "non_ledgered_activity_steps": CUTOVER_PHASE_M1_NON_LEDGERED_ACTIVITY_STEPS,
            "completed_ledger_steps": cutover_step_statuses
                .iter()
                .filter(|(_, status)| status.as_str() == "done")
                .map(|(step_name, _)| step_name.clone())
                .collect::<Vec<_>>(),
            "missing_ledger_steps": missing_cutover_steps,
        },
        "phase_m2": {
            "legacy": {
                "observed": legacy_m2_observed,
                "expected_ledger_steps": LEGACY_PHASE_M2_LEDGER_STEPS,
                "missing_ledger_steps": legacy_missing_m2_steps,
                "publish_events": legacy_publish_events,
                "step_counts": legacy_step_counts,
            },
            "cutover": {
                "observed": cutover_m2_observed,
                "expected_ledger_steps": CUTOVER_PHASE_M2_LEDGER_STEPS,
                "missing_ledger_steps": cutover_missing_m2_steps,
                "publish_events": cutover_publish_events,
                "step_counts": cutover_step_counts,
            }
        },
        "legacy_runtime": {
            "crawl_sources": legacy_crawl,
            "raw_knowledge_ingestion": legacy_raw_ingestion,
            "candidate_status_counts": legacy_candidate_counts,
            "projection_status": legacy_projection_status,
            "summary": {
                "verified_rule_count": legacy_verified,
                "needs_hitl_count": legacy_needs_hitl,
                "changed_truth_key_count": legacy_changed_truth_keys,
            }
        },
        "cutover_runtime": {
            "raw_evidence_register": cutover_raw_evidence,
            "candidate_validation": cutover_candidate_validation,
            "truth_adjudication": cutover_truth_adjudication,
            "verified_truth_write": cutover_verified_write,
            "contradiction_gate": cutover_contradiction_gate,
            "projection_barrier_semantic_projection": cutover_projection_barrier,
            "projection_status": cutover_projection_status,
            "support_refresh": {
                "expected": cutover_changed_truth_keys > 0,
                "observed_via_step_ledger": false,
                "note": "load_verified_support_bundle.* remains a non-ledgered activity surface"
            },
            "summary": {
                "verified_rule_count": cutover_verified,
                "needs_hitl_count": cutover_needs_hitl,
                "contradiction_conflict_count": cutover_contradictions,
                "changed_truth_key_count": cutover_changed_truth_keys,
            }
        },
        "findings": findings.iter().map(|finding| json!({
            "level": finding.level,
            "code": finding.code,
            "message": finding.message,
        })).collect::<Vec<_>>()
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(Path::new("."), report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }

    Ok(if status == "ok" || !strict { 0 } else { 2 })
}

