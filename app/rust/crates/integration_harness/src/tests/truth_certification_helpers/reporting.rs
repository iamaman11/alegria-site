async fn collect_truth_certification_fixture_report(
    pool: &sqlx::PgPool,
    fixture: &TruthCertificationFixture,
    run_id: &str,
    context_key: &str,
    scope_signature: &str,
) -> TruthCertificationFixtureReport {
    let verified_rows = sqlx::query(
        r#"
        SELECT rule_type_key, concept_key, params
        FROM verified.rule_instances
        WHERE context_key = $1
          AND status = 'verified'
        ORDER BY rule_type_key, concept_key
        "#,
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .unwrap();
    let verified_rule_set = verified_rows
        .into_iter()
        .map(|row| {
            stable_rule_key(
                &row.get::<String, _>("rule_type_key"),
                &row.get::<String, _>("concept_key"),
                &row.get::<serde_json::Value, _>("params"),
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let truth_adjudication = latest_output_payload_json(pool, run_id, "truth_adjudication").await;
    let mut needs_hitl_set = BTreeSet::new();
    let mut rejected_set = BTreeSet::new();
    let mut truth_adjudication_decisions = BTreeSet::new();
    let mut truth_adjudication_source_keys = BTreeSet::new();
    for payload in &truth_adjudication {
        for decision in payload
            .get("decisions")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            let concept = decision
                .get("concept_canonical_key")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let params = decision
                .get("params")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let reason = decision
                .get("adjudication_reason")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let source_key = decision
                .get("source_key")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !source_key.trim().is_empty() {
                truth_adjudication_source_keys.insert(source_key.to_string());
            }
            truth_adjudication_decisions.insert(format!(
                "{}|{}|{}|{}|{}",
                concept,
                serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string()),
                source_key,
                decision
                    .get("decision")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                reason
            ));
            match decision.get("decision").and_then(|value| value.as_str()) {
                Some("needs_hitl") => {
                    needs_hitl_set.insert(stable_decision_key(concept, &params, reason));
                }
                Some("rejected") => {
                    rejected_set.insert(stable_decision_key(concept, &params, reason));
                }
                _ => {}
            }
        }
    }

    let contradiction = latest_output_payload_json(pool, run_id, "contradiction_gate").await;
    let contradiction_groups = contradiction
        .iter()
        .flat_map(|payload| {
            payload
                .get("sections")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .flat_map(|section| {
                    section
                        .get("output")
                        .and_then(|value| value.get("conflicts"))
                        .and_then(|value| value.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|conflict| {
                            Some(format!(
                                "{}|{}|{}|{}|{}",
                                conflict.get("subject_key")?.as_str()?,
                                conflict.get("predicate_key")?.as_str()?,
                                conflict.get("value_left")?.as_str()?,
                                conflict.get("value_right")?.as_str()?,
                                conflict.get("severity")?.as_str()?
                            ))
                        })
                })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let source_governance_snapshot = if truth_adjudication_source_keys.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            r#"
            SELECT source_key, source_type, authority_class, independence_group_key,
                   trust_level, freshness_ttl_days, override_eligible
            FROM kb.sources
            WHERE source_key = ANY($1)
            ORDER BY source_key
            "#,
        )
        .bind(
            truth_adjudication_source_keys
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                row.get::<String, _>("source_key"),
                row.get::<String, _>("source_type"),
                row.get::<String, _>("authority_class"),
                row.get::<String, _>("independence_group_key"),
                row.get::<i32, _>("trust_level"),
                row.get::<i32, _>("freshness_ttl_days"),
                row.get::<bool, _>("override_eligible"),
            )
        })
        .collect::<Vec<_>>()
    };

    let completeness = latest_output_payload_json(pool, run_id, "completeness_judge").await;
    let completeness_failures = completeness
        .iter()
        .flat_map(|payload| {
            payload
                .get("sections")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .flat_map(|section| {
                    let blocked = section
                        .get("blocked_by_gate")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    if blocked {
                        Vec::new()
                    } else {
                        section
                            .get("output")
                            .and_then(|value| value.get("missing_elements"))
                            .and_then(|value| value.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|missing| {
                                Some(format!(
                                    "{}|{}|{}",
                                    missing.get("loss_type")?.as_str()?,
                                    missing.get("raw_fragment")?.as_str()?,
                                    missing.get("action")?.as_str()?
                                ))
                            })
                            .collect::<Vec<_>>()
                    }
                })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let draft_qas = latest_proto_outputs::<DraftQaOutputPayload>(pool, run_id, "draft_qa").await;
    let draft_blocking_reasons = draft_qas
        .iter()
        .flat_map(|output| output.blocking_reasons.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let cms_approved =
        latest_proto_outputs::<CmsPublishOutputPayload>(pool, run_id, "cms_publish_approved")
            .await;
    let cms_review =
        latest_proto_outputs::<CmsPublishOutputPayload>(pool, run_id, "cms_request_review").await;
    let publish_verdict = if cms_approved
        .iter()
        .any(|output| output.verdict == "approved")
    {
        "allow".to_string()
    } else if !cms_review.is_empty() || fixture.run_mode.contains("publish") {
        "blocked".to_string()
    } else {
        "not_attempted".to_string()
    };

    let projection_barriers = latest_proto_outputs::<ProjectionBarrierAuditOutputPayload>(
        pool,
        run_id,
        "projection_barrier(semantic_projection)",
    )
    .await;
    let projection_verdict = if projection_barriers
        .last()
        .map(|output| output.status.as_str() == "clear")
        .unwrap_or(false)
    {
        "allow".to_string()
    } else {
        "blocked".to_string()
    };

    let verified_truth_write = latest_output_payload_json(pool, run_id, "verified_truth_write").await;
    let changed_truth_keys = verified_truth_write
        .iter()
        .flat_map(|payload| {
            payload
                .get("changed_truth_keys")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_string))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let rebuild_outputs =
        latest_proto_outputs::<RebuildDetectOutputPayload>(pool, run_id, "rebuild_detect").await;
    let rebuild_impacted_page_count = rebuild_outputs
        .iter()
        .map(|output| output.impacts.len())
        .sum::<usize>();
    let rebuild_impact_emitted = rebuild_outputs
        .iter()
        .any(|output| !output.impacted_page_node_keys.is_empty());
    let draft_normalize_outputs =
        latest_proto_outputs::<DraftNormalizeOutputPayload>(pool, run_id, "draft_normalize").await;
    let required_factual_blocks_without_support = draft_normalize_outputs
        .iter()
        .flat_map(|output| {
            output.draft.as_ref().into_iter().flat_map(|draft| {
                draft.content_blocks.iter().filter_map(|block| {
                    let factual = !matches!(
                        block.section_role.as_str(),
                        "related_pages" | "cta_disclaimer"
                    );
                    if block.required && factual && block.support_refs.is_empty() {
                        Some(block.section_role.clone())
                    } else {
                        None
                    }
                })
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let temporal_step_multiset = temporal_step_multiset(pool, run_id).await;
    let semantic_snapshot = capture_semantic_snapshot(pool, scope_signature).await;
    let semantic_page_draft_qa_verdicts = semantic_snapshot
        .page_drafts
        .iter()
        .map(|draft| draft.qa_verdict.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let draft_verdict = if draft_qas.is_empty()
        || !draft_blocking_reasons.is_empty()
        || !required_factual_blocks_without_support.is_empty()
        || semantic_page_draft_qa_verdicts
            .iter()
            .any(|verdict| verdict != "publish_ready")
    {
        "blocked".to_string()
    } else if draft_qas
        .iter()
        .all(|output| output.verdict == "publish_ready")
    {
        "allow".to_string()
    } else {
        "blocked".to_string()
    };

    let mut failure_reasons = Vec::new();
    if verified_rule_set != fixture.expected_outcomes.verified_rule_set {
        failure_reasons.push("verified_rule_set_mismatch".to_string());
    }
    if needs_hitl_set.iter().cloned().collect::<Vec<_>>() != fixture.expected_outcomes.needs_hitl_set {
        failure_reasons.push("needs_hitl_set_mismatch".to_string());
    }
    if rejected_set.iter().cloned().collect::<Vec<_>>() != fixture.expected_outcomes.rejected_set {
        failure_reasons.push("rejected_set_mismatch".to_string());
    }
    if contradiction_groups != fixture.expected_outcomes.contradiction_groups {
        failure_reasons.push("contradiction_groups_mismatch".to_string());
    }
    if completeness_failures != fixture.expected_outcomes.completeness_failures {
        failure_reasons.push("completeness_failures_mismatch".to_string());
    }
    if draft_verdict != fixture.expected_outcomes.draft_verdict {
        failure_reasons.push("draft_verdict_mismatch".to_string());
    }
    if publish_verdict != fixture.expected_outcomes.publish_verdict {
        failure_reasons.push("publish_verdict_mismatch".to_string());
    }
    if projection_verdict != fixture.expected_outcomes.projection_verdict {
        failure_reasons.push("projection_verdict_mismatch".to_string());
    }
    if rebuild_impact_emitted != fixture.expected_outcomes.rebuild_impact_emitted {
        failure_reasons.push("rebuild_impact_mismatch".to_string());
    }

    TruthCertificationFixtureReport {
        fixture_id: fixture.fixture_id.clone(),
        run_id: run_id.to_string(),
        context_key: context_key.to_string(),
        scope_signature: scope_signature.to_string(),
        verified_rule_set,
        needs_hitl_set: needs_hitl_set.into_iter().collect(),
        rejected_set: rejected_set.into_iter().collect(),
        contradiction_groups,
        completeness_failures,
        draft_verdict,
        draft_blocking_reasons,
        publish_verdict,
        projection_verdict,
        changed_truth_keys,
        rebuild_impact_emitted,
        rebuild_impacted_page_count,
        temporal_step_multiset,
        semantic_page_node_count: semantic_snapshot.page_nodes.len(),
        semantic_page_draft_count: semantic_snapshot.page_drafts.len(),
        semantic_page_draft_qa_verdicts,
        truth_adjudication_decisions: truth_adjudication_decisions.into_iter().collect(),
        source_governance_snapshot,
        required_factual_blocks_without_support,
        pass: failure_reasons.is_empty(),
        failure_reasons,
        workflow_failure_diagnostics: None,
    }
}

async fn certification_failure_diagnostics(pool: &sqlx::PgPool, run_id: &str) -> String {
    let rows = sqlx::query(
        r#"
        SELECT step_name, status, COALESCE(error_class, '') AS error_class, COALESCE(error_message, '') AS error_message
        FROM pipeline.step_executions
        WHERE run_id = $1
        ORDER BY updated_at DESC, step_execution_id DESC
        LIMIT 12
        "#,
    )
    .bind(Uuid::parse_str(run_id).unwrap())
    .fetch_all(pool)
    .await
    .unwrap();
    let summary = rows
        .into_iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}",
                row.get::<String, _>("step_name"),
                row.get::<String, _>("status"),
                row.get::<String, _>("error_class"),
                row.get::<String, _>("error_message")
            )
        })
        .collect::<Vec<_>>();
    summary.join("\n")
}

fn expected_workflow_failure_allowed(
    fixture: &TruthCertificationFixture,
    report: &TruthCertificationFixtureReport,
    diagnostics: &str,
) -> bool {
    fixture.expected_outcomes.draft_verdict == "blocked"
        && report.draft_verdict == "blocked"
        && report.publish_verdict == fixture.expected_outcomes.publish_verdict
        && diagnostics.contains("truth_admissibility_gate:failed:")
}
