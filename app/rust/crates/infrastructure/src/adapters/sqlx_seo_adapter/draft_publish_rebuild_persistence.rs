
pub async fn persist_draft_assemble_output(
    pool: &PgPool,
    run_id: &str,
    output: &DraftAssembleOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    non_empty(run_id, "run_id")?;
    if let Some(brief) = output.page_brief.as_ref() {
        non_empty(&brief.page_brief_key, "page_brief_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_briefs
                (page_brief_key, page_node_key, blueprint_key, title, meta_description,
                 required_sections, required_links, evidence_manifest, brief_version, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (page_brief_key) DO UPDATE
            SET title            = EXCLUDED.title,
                meta_description = EXCLUDED.meta_description,
                status           = EXCLUDED.status,
                updated_at       = now()
            "#,
        )
        .bind(&brief.page_brief_key)
        .bind(&brief.page_node_key)
        .bind(&brief.blueprint_key)
        .bind(&brief.title)
        .bind(&brief.meta_description)
        .bind(Json(json!(brief.required_sections)))
        .bind(Json(json!(brief.required_links)))
        .bind(Json(json!({
            "truth_snapshot_ref": brief.truth_snapshot_ref,
            "metadata_obligations": brief.metadata_obligations.clone(),
            "target_audience": brief.target_audience.clone(),
            "goal": brief.goal.clone(),
            "editorial_brief_key": output
                .editorial_brief
                .as_ref()
                .map(|brief| brief.editorial_brief_key.clone())
                .unwrap_or_default(),
            "llm_request_key": output
                .llm_request
                .as_ref()
                .map(|request| request.request_key.clone())
                .unwrap_or_default(),
        })))
        .bind(brief.brief_version as i32)
        .bind(&brief.status)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        projection_events.push(seo_graph_projection_event(
            "page_brief",
            &brief.page_brief_key,
            "",
        ));
    }

    if let Some(plan) = output.content_block_plan.as_ref() {
        upsert_runtime_blob(pool, run_id, "content_block_plan", plan).await?;
    }

    if let Some(draft) = output.draft.as_ref() {
        non_empty(&draft.page_draft_key, "page_draft_key")?;
        sqlx::query(
            r#"
            INSERT INTO site.page_drafts
                (page_draft_key, page_node_key, page_brief_key, draft_revision,
                 body_markdown, traceability_manifest, qa_verdict, qa_blockers,
                 truth_snapshot_ref, assembly_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'seo_draft_assemble@1')
            ON CONFLICT (page_draft_key) DO UPDATE
            SET body_markdown      = EXCLUDED.body_markdown,
                traceability_manifest = EXCLUDED.traceability_manifest,
                qa_verdict         = EXCLUDED.qa_verdict,
                qa_blockers        = EXCLUDED.qa_blockers,
                truth_snapshot_ref = EXCLUDED.truth_snapshot_ref,
                assembly_version   = EXCLUDED.assembly_version,
                updated_at         = now()
            "#,
        )
        .bind(&draft.page_draft_key)
        .bind(&draft.page_node_key)
        .bind(&draft.page_brief_key)
        .bind(draft.draft_revision as i32)
        .bind(&draft.body_markdown)
        .bind(Json(json!({
            "traceability_entries": draft.traceability_entries.clone(),
            "claim_ledger": draft.claim_ledger.clone(),
            "content_blocks": draft.content_blocks.clone(),
            "faq_json": draft.faq_json.clone(),
            "schema_markup_json": draft.schema_markup_json.clone(),
            "llm_provider_key": draft.llm_provider_key.clone(),
            "llm_model_key": draft.llm_model_key.clone(),
            "generation_request_key": draft.generation_request_key.clone(),
        })))
        .bind(&draft.qa_verdict)
        .bind(Json(json!([])))
        .bind(&draft.truth_snapshot_ref)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        set_page_lifecycle(pool, &draft.page_node_key, "draft_ready", "draft_assembled").await?;

        for claim in &draft.claim_ledger {
            for support_ref in &claim.support_refs {
                if support_ref.trim().is_empty() {
                    continue;
                }
                sqlx::query(
                    r#"
                    INSERT INTO site.page_support_bindings
                        (page_support_binding_key, page_node_key, page_draft_key, support_ref,
                         source_section_key, fragment_kind, traceability_label)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (page_node_key, page_draft_key, support_ref, source_section_key, fragment_kind) DO UPDATE
                    SET traceability_label = EXCLUDED.traceability_label,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "page_support_binding",
                    &[
                        &draft.page_node_key,
                        &draft.page_draft_key,
                        support_ref,
                        &claim.source_section_key,
                        &claim.claim_kind,
                    ],
                ))
                .bind(&draft.page_node_key)
                .bind(&draft.page_draft_key)
                .bind(support_ref)
                .bind(&claim.source_section_key)
                .bind(&claim.claim_kind)
                .bind(&claim.traceability_label)
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;

                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'truth_support', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "rebuild_dependency",
                    &[&draft.page_node_key, "truth_support", support_ref],
                ))
                .bind(&draft.page_node_key)
                .bind(support_ref)
                .bind(Json(json!({
                    "page_draft_key": draft.page_draft_key,
                    "source_section_key": claim.source_section_key,
                    "claim_kind": claim.claim_kind,
                    "traceability_label": claim.traceability_label,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;

                if let Some(source_key) = sqlx::query_scalar::<_, String>(
                    r#"
                    SELECT source_key
                    FROM verified.rule_instances
                    WHERE rule_instance_id = $1
                      AND source_key IS NOT NULL
                      AND source_key <> ''
                    LIMIT 1
                    "#,
                )
                .bind(support_ref)
                .fetch_optional(pool)
                .await
                .map_err(classify_sqlx)?
                {
                    sqlx::query(
                        r#"
                        INSERT INTO monitoring.seo_rebuild_dependencies
                            (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                        VALUES ($1, $2, 'source_provenance', $3, $4, 'active')
                        ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                        SET reason_package = EXCLUDED.reason_package,
                            status = EXCLUDED.status,
                            updated_at = now()
                        "#,
                    )
                    .bind(primitives::seo::seo_artifact_key(
                        "rebuild_dependency",
                        &[&draft.page_node_key, "source_provenance", &source_key],
                    ))
                    .bind(&draft.page_node_key)
                    .bind(&source_key)
                    .bind(Json(json!({
                        "page_draft_key": draft.page_draft_key,
                        "support_ref": support_ref,
                        "source_section_key": claim.source_section_key,
                    })))
                    .execute(pool)
                    .await
                    .map_err(classify_sqlx)?;
                }
            }
        }

        if let Some(plan) = output.content_block_plan.as_ref() {
            for item in &plan.items {
                if item.template_key.trim().is_empty() {
                    continue;
                }
                sqlx::query(
                    r#"
                    INSERT INTO monitoring.seo_rebuild_dependencies
                        (rebuild_dependency_key, page_node_key, dependency_type, dependency_ref, reason_package, status)
                    VALUES ($1, $2, 'section_template', $3, $4, 'active')
                    ON CONFLICT (page_node_key, dependency_type, dependency_ref) DO UPDATE
                    SET reason_package = EXCLUDED.reason_package,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(primitives::seo::seo_artifact_key(
                    "rebuild_dependency",
                    &[&draft.page_node_key, "section_template", &item.template_key],
                ))
                .bind(&draft.page_node_key)
                .bind(&item.template_key)
                .bind(Json(json!({
                    "page_draft_key": draft.page_draft_key,
                    "section_role": item.section_role,
                    "block_type": item.block_type,
                    "required": item.required,
                })))
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
            }
        }

        projection_events.push(seo_qdrant_projection_event(
            "editorial_topics_4",
            "page_draft",
            &draft.page_draft_key,
            "",
            &draft.body_markdown,
            HashMap::from([
                ("page_node_key".to_string(), draft.page_node_key.clone()),
                ("page_brief_key".to_string(), draft.page_brief_key.clone()),
                ("qa_verdict".to_string(), draft.qa_verdict.clone()),
                (
                    "truth_snapshot_ref".to_string(),
                    draft.truth_snapshot_ref.clone(),
                ),
            ]),
        ));
    }

    emit_projection_events_for_run(pool, run_id, projection_events).await?;
    Ok(())
}

pub async fn persist_draft_normalize_output(
    pool: &PgPool,
    input: &DraftNormalizeInputPayload,
    output: &DraftNormalizeOutputPayload,
) -> Result<(), DomainError> {
    upsert_runtime_blob(pool, &input.run_id, "draft_normalize_output", output).await?;
    let output_as_assemble = DraftAssembleOutputPayload {
        page_brief: None,
        draft: output.draft.clone(),
        editorial_brief: None,
        llm_request: None,
        content_block_plan: output.content_block_plan.clone(),
    };
    persist_draft_assemble_output(pool, &input.run_id, &output_as_assemble).await
}

pub async fn persist_content_contract_validate_output(
    pool: &PgPool,
    input: &ContentContractValidateInputPayload,
    output: &ContentContractValidateOutputPayload,
) -> Result<(), DomainError> {
    upsert_runtime_blob(pool, &input.run_id, "content_contract_validation", output).await?;
    if let Some(draft) = input.draft.as_ref() {
        non_empty(&draft.page_draft_key, "page_draft_key")?;
        sqlx::query(
            r#"
            UPDATE site.page_drafts
            SET qa_blockers = $1,
                updated_at = now()
            WHERE page_draft_key = $2
            "#,
        )
        .bind(Json(json!({
            "content_contract_verdict": output.verdict,
            "blocking_reasons": output.blocking_reasons,
            "blockers": output.blockers,
        })))
        .bind(&draft.page_draft_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(())
}

pub async fn persist_draft_qa_output(
    pool: &PgPool,
    input: &DraftQaInputPayload,
    output: &DraftQaOutputPayload,
) -> Result<(), DomainError> {
    let Some(draft) = input.draft.as_ref() else {
        return Err(validation_failure("draft_qa persistence requires draft"));
    };
    non_empty(&draft.page_draft_key, "page_draft_key")?;
    sqlx::query(
        r#"
        UPDATE site.page_drafts
        SET qa_verdict  = $1,
            qa_blockers = $2,
            updated_at  = now()
        WHERE page_draft_key = $3
        "#,
    )
    .bind(&output.verdict)
    .bind(Json(json!({
        "blocking_reasons": output.blocking_reasons,
        "blockers": output.blockers,
        "required_next_action": output.required_next_action,
    })))
    .bind(&draft.page_draft_key)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    let lifecycle_state = "review_required";
    set_page_lifecycle(pool, &draft.page_node_key, lifecycle_state, "draft_qa").await?;

    for reason in &output.blocking_reasons {
        let quality_failure_key = primitives::seo::seo_artifact_key(
            "seo_quality_failure",
            &[&draft.page_draft_key, reason, "draft_qa@1"],
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_quality_failures
                (quality_failure_key, page_draft_key, failure_type, severity, status, details)
            VALUES ($1, $2, $3, 'blocking', 'open', $4)
            ON CONFLICT (quality_failure_key) DO UPDATE
            SET status     = EXCLUDED.status,
                details    = EXCLUDED.details,
                updated_at = now()
            "#,
        )
        .bind(quality_failure_key)
        .bind(&draft.page_draft_key)
        .bind(reason)
        .bind(Json(json!({
            "verdict": output.verdict,
            "supported_claims": output.supported_claims,
            "unsupported_claims": output.unsupported_claims,
        })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    emit_projection_events_for_run(
        pool,
        &input.run_id,
        vec![seo_qdrant_projection_event(
            "editorial_topics_4",
            "page_draft",
            &draft.page_draft_key,
            "",
            &draft.body_markdown,
            HashMap::from([
                ("page_node_key".to_string(), draft.page_node_key.clone()),
                ("page_brief_key".to_string(), draft.page_brief_key.clone()),
                ("qa_verdict".to_string(), output.verdict.clone()),
                (
                    "blocking_reasons".to_string(),
                    output.blocking_reasons.join("|"),
                ),
            ]),
        )],
    )
    .await?;

    Ok(())
}

pub async fn persist_rebuild_detect_output(
    pool: &PgPool,
    input: &RebuildDetectInputPayload,
    output: &RebuildDetectOutputPayload,
) -> Result<(), DomainError> {
    if output.verdict != "rebuild_required" {
        return Ok(());
    }

    let mut projection_events = Vec::new();
    let impact_by_page = resolve_rebuild_impacts(pool, &input.changed_truth_keys).await?;
    let output_impact_by_page = output
        .impacts
        .iter()
        .map(|impact| {
            let reason_package = serde_json::from_str::<Value>(&impact.reason_package_json)
                .unwrap_or_else(|_| {
                    json!({
                        "fallback_reason": impact.reason_package_json,
                    })
                });
            (
                impact.page_node_key.clone(),
                (impact.trigger_type.clone(), reason_package, impact.priority),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for page_node_key in &output.impacted_page_node_keys {
        if page_node_key.trim().is_empty() {
            continue;
        }
        let changed_truth_key = input
            .changed_truth_keys
            .first()
            .map(String::as_str)
            .unwrap_or("truth_change");
        let (trigger_type, reason_package, priority) = output_impact_by_page
            .get(page_node_key)
            .cloned()
            .unwrap_or((
                rebuild::TRUTH_CHANGE.to_string(),
                rebuild::canonical_reason_package(rebuild::TRUTH_CHANGE, &input.changed_truth_keys),
                rebuild::trigger_priority(rebuild::TRUTH_CHANGE),
            ));
        let reason_package = if let Some(impact_reason) = impact_by_page.get(page_node_key) {
            let mut merged = reason_package;
            if let Some(matched) = impact_reason.get("matched_dependencies").cloned() {
                merged["matched_dependencies"] = matched;
            }
            if let Some(seed) = impact_reason.get("seed_reason_package").cloned() {
                merged["seed_reason_package"] = seed;
            }
            merged
        } else {
            reason_package
        };
        if let Some(lifecycle_state) = rebuild::lifecycle_state_for_trigger(&trigger_type) {
            set_page_lifecycle(pool, page_node_key, lifecycle_state, &trigger_type).await?;
        }
        projection_events.push(seo_graph_projection_event("page_node", page_node_key, ""));
        let rebuild_request_key = primitives::seo::seo_artifact_key(
            "seo_rebuild",
            &[
                page_node_key,
                &trigger_type,
                changed_truth_key,
                "rebuild_detect@1",
            ],
        );
        sqlx::query(
            r#"
            INSERT INTO monitoring.seo_rebuild_backlog
                (rebuild_request_key, page_node_key, trigger_type, priority, status, reason)
            VALUES ($1, $2, $3, $4, 'queued', $5)
            ON CONFLICT (rebuild_request_key) DO UPDATE
                SET status     = 'queued',
                trigger_type = EXCLUDED.trigger_type,
                priority   = EXCLUDED.priority,
                reason     = EXCLUDED.reason,
                updated_at = now()
            "#,
        )
        .bind(rebuild_request_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(reason_package.to_string())
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

        let global_rebuild_plan_key = primitives::seo::seo_artifact_key(
            "global_rebuild_plan",
            &[page_node_key, &trigger_type, changed_truth_key],
        );
        sqlx::query(
            r#"
            INSERT INTO site.global_rebuild_plan
                (rebuild_plan_key, affected_page_node_key, trigger_type, priority, reason_payload, status)
            VALUES ($1, $2, $3, $4, $5, 'queued')
            ON CONFLICT (rebuild_plan_key) DO UPDATE
            SET affected_page_node_key = EXCLUDED.affected_page_node_key,
                trigger_type = EXCLUDED.trigger_type,
                priority = EXCLUDED.priority,
                reason_payload = EXCLUDED.reason_payload,
                status = 'queued',
                updated_at = now()
            "#,
        )
        .bind(global_rebuild_plan_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(Json(reason_package.clone()))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    emit_projection_events_for_run(pool, &input.run_id, projection_events).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_applicant_profile;
    use seo_domain::applicability::{
        decide_support_resolution, ApplicabilityRuleCandidate, SupportResolutionDecision,
    };

    fn candidate() -> ApplicabilityRuleCandidate {
        ApplicabilityRuleCandidate {
            rule_instance_id: "rule:1".to_string(),
            has_profile_overrides: false,
            has_apply_profile: false,
            has_conditional_profile: false,
            has_exclude_profile: false,
            has_waive_exception: false,
            has_remove_exception: false,
            has_replace_exception: false,
            has_add_requirement_exception: false,
        }
    }

    #[test]
    fn supports_apply_without_profile_overrides() {
        let decision = decide_support_resolution(&candidate());
        assert_eq!(decision, SupportResolutionDecision::Include);
    }

    #[test]
    fn excludes_conditional_without_context() {
        let mut flags = candidate();
        flags.has_profile_overrides = true;
        flags.has_conditional_profile = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Unresolved("unresolved_conditional_applicability")
        );
    }

    #[test]
    fn excludes_replace_value_override_from_auto_support() {
        let mut flags = candidate();
        flags.has_replace_exception = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Unresolved("unresolved_exception_override")
        );
    }

    #[test]
    fn excludes_profile_mismatch() {
        let mut flags = candidate();
        flags.has_profile_overrides = true;
        let decision = decide_support_resolution(&flags);
        assert_eq!(
            decision,
            SupportResolutionDecision::Excluded("profile_not_applicable")
        );
    }

    #[test]
    fn normalizes_known_profiles_only() {
        assert_eq!(normalize_applicant_profile("Minor").unwrap(), "minor");
        assert!(normalize_applicant_profile("base").is_err());
    }
}

async fn set_page_lifecycle(
    pool: &PgPool,
    page_node_key: &str,
    to_state: &str,
    reason: &str,
) -> Result<(), DomainError> {
    if page_node_key.trim().is_empty() {
        return Ok(());
    }
    let previous_state: Option<String> = sqlx::query_scalar(
        r#"
        UPDATE site.page_nodes
        SET lifecycle_state = $1,
            updated_at = now()
        WHERE page_node_key = $2
        RETURNING lifecycle_state
        "#,
    )
    .bind(to_state)
    .bind(page_node_key)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?;

    if previous_state.is_some() {
        sqlx::query(
            r#"
            INSERT INTO site.page_lifecycle_events
                (page_node_key, from_state, to_state, reason, actor, event_payload)
            VALUES ($1, NULL, $2, $3, 'seo_system', $4)
            "#,
        )
        .bind(page_node_key)
        .bind(to_state)
        .bind(reason)
        .bind(Json(json!({ "source": "temporal_activity" })))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    Ok(())
}
