pub async fn persist_draft_assemble_output(
    pool: &PgPool,
    run_id: &str,
    output: &DraftAssembleOutputPayload,
) -> Result<(), DomainError> {
    let mut projection_events = Vec::new();
    non_empty(run_id, "run_id")?;
    // A draft, its support/dependency graph, runtime plan and projection
    // commands are one decision. Never expose a partly assembled page.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
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
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;

        let mut event = seo_graph_projection_event(
            "page_brief",
            &brief.page_brief_key,
            "",
        );
        event.run_id = run_id.to_string();
        projection_events.push(event);
    }

    if let Some(plan) = output.content_block_plan.as_ref() {
        upsert_runtime_blob_on_conn(&mut *tx, run_id, "content_block_plan", plan).await?;
    }

    if let Some(draft) = output.draft.as_ref() {
        non_empty(&draft.page_draft_key, "page_draft_key")?;
        let draft_write = sqlx::query(
            r#"
            INSERT INTO site.page_drafts
                (page_draft_key, page_node_key, page_brief_key, draft_revision,
                 body_markdown, traceability_manifest, qa_verdict, qa_blockers,
                 truth_snapshot_ref, assembly_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'seo_draft_assemble@4')
            ON CONFLICT (page_draft_key) DO UPDATE
            SET page_draft_key = EXCLUDED.page_draft_key
            WHERE site.page_drafts.page_node_key = EXCLUDED.page_node_key
              AND site.page_drafts.page_brief_key = EXCLUDED.page_brief_key
              AND site.page_drafts.draft_revision = EXCLUDED.draft_revision
              AND site.page_drafts.body_markdown = EXCLUDED.body_markdown
              AND site.page_drafts.traceability_manifest = EXCLUDED.traceability_manifest
              AND site.page_drafts.truth_snapshot_ref = EXCLUDED.truth_snapshot_ref
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
            "editorial_locale": draft.editorial_locale.clone(),
            "seo_title": output.page_brief.as_ref().map(|brief| brief.title.as_str()),
            "seo_meta_description": output.page_brief.as_ref().map(|brief| brief.meta_description.as_str()),
        })))
        .bind(&draft.qa_verdict)
        .bind(Json(json!([])))
        .bind(&draft.truth_snapshot_ref)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;

        if draft_write.rows_affected() != 1 {
            return Err(validation_failure(
                "immutable page draft changed during replay",
            ));
        }
        set_page_lifecycle_in_tx(&mut tx, &draft.page_node_key, "draft_ready", "draft_assembled")
            .await?;

        // One lookup for all support IDs, rather than one round trip for each
        // nested claim/support reference. Missing IDs simply have no source
        // provenance dependency, as before.
        let support_refs = draft
            .claim_ledger
            .iter()
            .flat_map(|claim| claim.support_refs.iter())
            .filter(|support_ref| !support_ref.trim().is_empty())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let source_keys = if support_refs.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, (String, String)>(
                r#"
                SELECT rule_instance_id, source_key
                FROM verified.rule_instances
                WHERE rule_instance_id = ANY($1)
                  AND source_key IS NOT NULL
                  AND source_key <> ''
                "#,
            )
            .bind(&support_refs)
            .fetch_all(&mut *tx)
            .await
            .map_err(classify_sqlx)?
            .into_iter()
            .collect::<HashMap<_, _>>()
        };

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
                .bind(seo_domain::artifact_identity::seo_artifact_key(
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
                .execute(&mut *tx)
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
                .bind(seo_domain::artifact_identity::seo_artifact_key(
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
                .execute(&mut *tx)
                .await
                .map_err(classify_sqlx)?;

                if let Some(source_key) = source_keys.get(support_ref) {
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
                    .bind(seo_domain::artifact_identity::seo_artifact_key(
                        "rebuild_dependency",
                        &[&draft.page_node_key, "source_provenance", source_key],
                    ))
                    .bind(&draft.page_node_key)
                    .bind(source_key)
                    .bind(Json(json!({
                        "page_draft_key": draft.page_draft_key,
                        "support_ref": support_ref,
                        "source_section_key": claim.source_section_key,
                    })))
                    .execute(&mut *tx)
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
                .bind(seo_domain::artifact_identity::seo_artifact_key(
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
                .execute(&mut *tx)
                .await
                .map_err(classify_sqlx)?;
            }
        }

        let mut event = seo_qdrant_projection_event(
            run_id,
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
        );
        event.run_id = run_id.to_string();
        // Protobuf map iteration order is not stable. Retrying the same
        // immutable draft must not create another Qdrant projection event.
        event.idempotency_key = primitives::hash::content_hash_v1(&format!(
            "draft_assemble@4|{}|{}|{}|{}|{}",
            run_id,
            draft.page_draft_key,
            draft.truth_snapshot_ref,
            draft.qa_verdict,
            primitives::hash::blake3_hex(draft.body_markdown.as_bytes()),
        ));
        projection_events.push(event);
    }

    super::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(&mut tx, &projection_events)
        .await?;
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(())
}

pub async fn persist_draft_normalize_output(
    pool: &PgPool,
    input: &DraftNormalizeInputPayload,
    output: &DraftNormalizeOutputPayload,
) -> Result<(), DomainError> {
    upsert_runtime_blob(pool, &input.run_id, "draft_normalize_output", output).await?;
    let output_as_assemble = DraftAssembleOutputPayload {
        page_brief: output.page_brief.clone(),
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
    // The content contract is a per-run intermediate result, not a second QA
    // verdict. Keep it in the runtime blob; Draft QA alone owns the durable
    // page_drafts.qa_verdict/qa_blockers fields and their lifecycle transition.
    upsert_runtime_blob(pool, &input.run_id, "content_contract_validation", output).await
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
    non_empty(&draft.page_node_key, "page_node_key")?;
    let lifecycle_state = seo_domain::draft_quality::review_lifecycle(
        &output.verdict,
        output.blocking_reasons.len(),
    )
    .ok_or_else(|| validation_failure("draft_qa verdict and blocking reasons must be consistent"))?;
    // The QA verdict, page lifecycle, audit failures and projection event are
    // one logical result: a failed late write must not expose half a QA run.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let updated_draft = sqlx::query(
        r#"
        UPDATE site.page_drafts
        SET qa_verdict  = $1,
            qa_blockers = $2,
            updated_at  = now()
        WHERE page_draft_key = $3 AND page_node_key = $4
        "#,
    )
    .bind(&output.verdict)
    .bind(Json(json!({
        "blocking_reasons": output.blocking_reasons,
        "blockers": output.blockers,
        "required_next_action": output.required_next_action,
    })))
    .bind(&draft.page_draft_key)
    .bind(&draft.page_node_key)
    .execute(&mut *tx)
    .await
    .map_err(classify_sqlx)?;
    if updated_draft.rows_affected() != 1 {
        return Err(validation_failure(
            "draft_qa persistence requires an existing draft belonging to its page",
        ));
    }

    set_page_lifecycle_in_tx(&mut tx, &draft.page_node_key, lifecycle_state, "draft_qa").await?;

    for reason in &output.blocking_reasons {
        let quality_failure_key = seo_domain::artifact_identity::seo_artifact_key(
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
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }

    let mut event = seo_qdrant_projection_event(
            &input.run_id,
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
        );
    event.run_id = input.run_id.clone();
    // Protobuf map fields may serialize in a different order on replay. QA
    // identity is therefore based on stable inputs, never encoded map bytes.
    event.idempotency_key = primitives::hash::content_hash_v1(&format!(
        "draft_qa@1|{}|{}|{}|{}|{}|{}",
        input.run_id,
        draft.page_draft_key,
        output.verdict,
        output.supported_claims,
        output.unsupported_claims,
        output.blocking_reasons.join("|"),
    ));
    super::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(&mut tx, &[event]).await?;
    tx.commit().await.map_err(classify_sqlx)?;

    Ok(())
}

pub async fn persist_rebuild_detect_output(
    pool: &PgPool,
    input: &RebuildDetectRequest,
    output: &RebuildDetectResult,
) -> Result<(), DomainError> {
    if output.verdict != RebuildVerdict::RebuildRequired {
        return Ok(());
    }
    non_empty(&input.run_id, "run_id")?;

    let mut projection_events = Vec::new();
    let impact_by_page = resolve_rebuild_impacts(pool, &input.changed_truth_keys).await?;
    let output_impact_by_page = output
        .impacts
        .iter()
        .map(|impact| {
            (
                impact.page_node_key.clone(),
                (
                    impact.trigger_type.clone(),
                    impact.reason_package.clone(),
                    impact.priority,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    // A rebuild decision affects multiple pages and two queues. Commit every
    // page, lifecycle transition and projection event together or none.
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    // Stable, deduplicated page lock order also prevents two multi-page
    // rebuild runs from deadlocking when their impact lists are reversed.
    for page_node_key in output
        .impacted_page_node_keys
        .iter()
        .filter(|page| !page.trim().is_empty())
        .collect::<BTreeSet<_>>()
    {
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
        let rebuild_request_key = seo_domain::artifact_identity::seo_artifact_key(
            "seo_rebuild",
            &[
                page_node_key,
                &trigger_type,
                changed_truth_key,
                &input.run_id,
                "rebuild_detect@1",
            ],
        );
        // Keep lock order page -> backlog, consistent with page lifecycle
        // writers. A processed event must never republish a rebuild task or
        // move a completed page back to needs_rebuild on Temporal retry.
        let page_exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1::bigint FROM site.page_nodes WHERE page_node_key = $1 FOR UPDATE",
        )
        .bind(page_node_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        if page_exists.is_none() {
            return Err(validation_failure(format!(
                "rebuild page_node not found: {page_node_key}"
            )));
        }
        let queued = sqlx::query(
            r#"
            INSERT INTO monitoring.seo_rebuild_backlog
                (rebuild_request_key, page_node_key, trigger_type, priority, status, reason)
            VALUES ($1, $2, $3, $4, 'queued', $5)
            ON CONFLICT (rebuild_request_key) DO UPDATE
                SET trigger_type = EXCLUDED.trigger_type,
                priority   = EXCLUDED.priority,
                reason     = EXCLUDED.reason,
                updated_at = now()
            WHERE monitoring.seo_rebuild_backlog.status = 'queued'
            "#,
        )
        .bind(rebuild_request_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(reason_package.to_string())
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        if queued.rows_affected() == 0 {
            continue;
        }
        if let Some(lifecycle_state) = rebuild::lifecycle_state_for_trigger(&trigger_type) {
            set_page_lifecycle_in_tx(&mut tx, page_node_key, lifecycle_state, &trigger_type)
                .await?;
        }
        let mut event = seo_graph_projection_event("page_node", page_node_key, "");
        event.run_id = input.run_id.clone();
        // One durable projection per distinct change-run, not one forever per
        // page. Temporal replay of the same run keeps the exact same key.
        event.idempotency_key = primitives::hash::content_hash_v1(&format!(
            "rebuild_detect@1|{}|{}|{}",
            input.run_id, page_node_key, trigger_type
        ));
        projection_events.push(event);

        let global_rebuild_plan_key = seo_domain::artifact_identity::seo_artifact_key(
            "global_rebuild_plan",
            &[page_node_key, &trigger_type, changed_truth_key, &input.run_id],
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
                updated_at = now()
            WHERE site.global_rebuild_plan.status = 'queued'
            "#,
        )
        .bind(global_rebuild_plan_key)
        .bind(page_node_key)
        .bind(&trigger_type)
        .bind(priority)
        .bind(Json(reason_package.clone()))
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }

    super::sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(&mut tx, &projection_events)
        .await?;
    tx.commit().await.map_err(classify_sqlx)?;
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
            has_unresolved_exception_conditions: false,
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

async fn set_page_lifecycle_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    page_node_key: &str,
    to_state: &str,
    reason: &str,
) -> Result<(), DomainError> {
    let from_state: Option<String> = sqlx::query_scalar(
        "SELECT lifecycle_state FROM site.page_nodes WHERE page_node_key = $1 FOR UPDATE",
    )
    .bind(page_node_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let Some(from_state) = from_state else {
        return Ok(());
    };

    // Generating or rechecking a new draft must never unpublish the live page.
    // Explicit rebuild transitions remain free to mark it as needs_rebuild.
    if from_state == to_state
        || (from_state == "published" && matches!(reason, "draft_assembled" | "draft_qa"))
    {
        return Ok(());
    }

    sqlx::query(
        "UPDATE site.page_nodes SET lifecycle_state = $1, updated_at = now() WHERE page_node_key = $2",
    )
    .bind(to_state)
    .bind(page_node_key)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    sqlx::query(
        r#"
        INSERT INTO site.page_lifecycle_events
            (page_node_key, from_state, to_state, reason, actor, event_payload)
        VALUES ($1, $2, $3, $4, 'seo_system', $5)
        "#,
    )
    .bind(page_node_key)
    .bind(&from_state)
    .bind(to_state)
    .bind(reason)
    .bind(Json(json!({ "source": "temporal_activity" })))
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}
