pub async fn execute_site_build_scenario<R>(
    repo: &R,
    request: &SeoScenarioRequest,
) -> Result<SeoScenarioResult, DomainError>
where
    R: SeoBuildInputRepository
        + VerifiedSupportRepository
        + PlanningRepository
        + CrawlIngestRepository
        + DraftRepository
        + SectionTemplateRepository
        + SourceContextSearchPort
        + EditorialGenerationPort
        + CmsReviewPort
        + PublishArtifactRepository
        + RebuildRepository
        + ProjectionStatusRepository
        + SerpSearchPort
        + SemanticLinkSearchPort
        + GraphReasoningPort
        + GraphCapabilityPort,
{
    // Explicit recipe supports the current fixed scenario as well as future
    // compositions without changing the scenario request or Temporal payload.
    let recipe = SeoPhaseRecipe::for_plan(&build_execution_plan(request), true)
        .map_err(|error| execution_plan_invariant_error(&error.to_string()))?;
    execute_site_build_scenario_with_recipe(repo, request, &recipe).await
}

/// Execute a preflight-validated recipe in the non-Temporal scenario runner.
/// Durable Temporal histories remain pinned to their own deployed workflow;
/// they do not dynamically adopt a reordered recipe during replay.
pub async fn execute_site_build_scenario_with_recipe<R>(
    repo: &R,
    request: &SeoScenarioRequest,
    recipe: &SeoPhaseRecipe,
) -> Result<SeoScenarioResult, DomainError>
where
    R: SeoBuildInputRepository
        + VerifiedSupportRepository
        + PlanningRepository
        + CrawlIngestRepository
        + DraftRepository
        + SectionTemplateRepository
        + SourceContextSearchPort
        + EditorialGenerationPort
        + CmsReviewPort
        + PublishArtifactRepository
        + RebuildRepository
        + ProjectionStatusRepository
        + SerpSearchPort
        + SemanticLinkSearchPort
        + GraphReasoningPort
        + GraphCapabilityPort,
{
    let plan = build_execution_plan(request);
    // Validate all step contracts and publication policy before *any* I/O.
    recipe
        .validate_for_scenario(&plan)
        .map_err(|error| execution_plan_invariant_error(&error.to_string()))?;
    let mut state = ScenarioState::default();
    let mut phase_reports = Vec::new();
    let mut final_status = "done".to_string();

    // Execute one dependency-validated pre-page recipe. Initial and planning
    // operations may interleave; the step contract identifies the owner that
    // executes each operation, not a predetermined site-building sequence.
    for (index, &key) in recipe.pre_page_steps().iter().enumerate() {
        match key.contract().segment {
            SeoExecutionSegment::Initial => {
                run_initial_phase(repo, request, &mut state, key, &mut phase_reports).await?;
                if key == SeoPhaseKey::RawKnowledgeIngestion {
                    if let Some(status) = enforce_projection_policy(
                        repo,
                        &request.site_input.run_id,
                        request.policy.projection,
                        phase_label(key),
                        &mut phase_reports,
                    )
                    .await?
                    {
                        final_status = status;
                    }
                }
            }
            SeoExecutionSegment::Planning => {
                // A clean truth batch does not need a second support read if
                // the first read has already happened in this recipe. When
                // there is no earlier producer, refresh is indispensable.
                let unchanged_truth = state
                    .raw_knowledge
                    .as_ref()
                    .is_some_and(|output| output.changed_truth_keys.is_empty());
                let support_already_loaded = recipe.pre_page_steps()[..index]
                    .contains(&SeoPhaseKey::LoadVerifiedSupportBundle);
                if key == SeoPhaseKey::RefreshVerifiedSupportBundle
                    && unchanged_truth
                    && support_already_loaded
                {
                    continue;
                }
                if let Some(status) =
                    run_planning_phase(repo, request, &mut state, key, &mut phase_reports).await?
                {
                    final_status = status;
                }
            }
            // Guaranteed by compose_pre_page / validate_for_scenario before IO.
            SeoExecutionSegment::PerPage | SeoExecutionSegment::Finalize => {
                unreachable!("invalid pre-page recipe step")
            }
        }
    }

    let raw_knowledge_changed_truth_keys = state
        .raw_knowledge
        .as_ref()
        .ok_or_else(|| execution_plan_invariant_error("raw_knowledge_ingestion_required"))?
        .changed_truth_keys
        .clone();
    if matches!(request.scenario, SeoScenarioKind::CrawlIngestOnly) {
        return Ok(SeoScenarioResult {
            scenario: kind_label(request.scenario).to_string(),
            mode: mode_label(request.mode).to_string(),
            status: if final_status == "done" {
                state
                    .raw_knowledge
                    .as_ref()
                    .ok_or_else(|| {
                        execution_plan_invariant_error("raw_knowledge_ingestion_required")
                    })?
                    .status
                    .clone()
            } else {
                final_status
            },
            page_total: 0,
            published_pages: 0,
            changed_truth_keys: raw_knowledge_changed_truth_keys.clone(),
            phase_reports,
        });
    }

    let (page_nodes, link_recommendations) = page_nodes_and_links(&state)?;
    let page_total = page_nodes.len() as u32;
    let factual_fragments = state
        .verified_support
        .iter()
        .map(|support| support.fragment_text.clone())
        .collect::<Vec<_>>();

    if matches!(request.scenario, SeoScenarioKind::PlanningOnly) {
        return Ok(SeoScenarioResult {
            scenario: kind_label(request.scenario).to_string(),
            mode: mode_label(request.mode).to_string(),
            status: if page_nodes.is_empty() && final_status == "done" {
                "done:no_pages".to_string()
            } else {
                final_status
            },
            page_total,
            published_pages: 0,
            changed_truth_keys: raw_knowledge_changed_truth_keys.clone(),
            phase_reports,
        });
    }

    let mut cursor;
    let mut scenario_status = final_status;
    let mut published_pages = 0u32;
    let page_keys = recipe.steps(SeoExecutionSegment::PerPage);
    let ia = require_state_ref(&state.ia, "ia_build_required")?;
    use seo_domain::page_readiness::PageAdmissibility;
    use seo_steps::page_wire::page_admissibility;
    let gaps =
        &require_state_ref(&state.reconciled, "global_site_reconcile_required")?.content_gaps;
    let requires_editorial = page_keys.contains(&SeoPhaseKey::EditorialDraftGenerate);
    let mut support_blocked = false;
    let mut published_page_keys = std::collections::BTreeSet::new();
    for page_node in page_nodes.iter().cloned() {
        let page_blueprint = ia
            .page_blueprints
            .iter()
            .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
            .cloned()
            .unwrap_or_default();
        let page_support =
            seo_steps::page_wire::page_verified_support(&page_blueprint, &state.verified_support);
        match page_admissibility(
            &page_node,
            &page_blueprint,
            &page_support,
            gaps,
            requires_editorial,
        ) {
            PageAdmissibility::Retired => continue,
            PageAdmissibility::Blocked(reason) => {
                support_blocked = true;
                phase_reports.push(phase_report(
                    "page_admissibility",
                    blocked_publish_gate_status(),
                    &page_node.page_node_key,
                    reason,
                ));
                continue;
            }
            PageAdmissibility::Eligible => {}
        }
        let page_fragments = factual_fragments
            .iter()
            .filter(|text| {
                page_support
                    .iter()
                    .any(|support| support.fragment_text.as_str() == text.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        let required_links = seo_steps::page_wire::required_links_for_published_targets(
            &link_recommendations,
            &page_node.page_node_key,
            &published_page_keys,
        );
        let mut page_state = PageState::default();
        cursor = SeoExecutionCursor {
            segment: Some(SeoExecutionSegment::PerPage),
            page_total: page_total as usize,
            ..Default::default()
        };
        loop {
            match next_phase(page_keys, &cursor) {
                SeoPhaseDecision::Complete(_) => break,
                SeoPhaseDecision::Run(input) => {
                    if let Some(status) = run_page_phase(
                        repo,
                        request,
                        &page_node,
                        &page_blueprint,
                        &required_links,
                        &page_fragments,
                        &page_support,
                        &mut page_state,
                        input.key,
                        &mut phase_reports,
                    )
                    .await?
                    {
                        scenario_status = status;
                        break;
                    }
                    if input.key == SeoPhaseKey::FinalizePublish {
                        published_pages += 1;
                        published_page_keys.insert(page_node.page_node_key.clone());
                    }
                }
            }
            advance_cursor(&mut cursor);
        }
        if scenario_status.starts_with("blocked") || scenario_status == "hitl_required" {
            break;
        }
    }

    if support_blocked && scenario_status == "done" {
        scenario_status = blocked_publish_gate_status().to_string();
    }

    cursor = SeoExecutionCursor {
        segment: Some(SeoExecutionSegment::Finalize),
        ..Default::default()
    };
    loop {
        match next_phase(recipe.steps(SeoExecutionSegment::Finalize), &cursor) {
            SeoPhaseDecision::Complete(_) => break,
            SeoPhaseDecision::Run(input) => match input.key {
                SeoPhaseKey::RebuildDetect => {
                    let rebuild = rebuild_detect::execute(
                        repo,
                        &RebuildDetectRequest {
                            run_id: request.site_input.run_id.clone(),
                            changed_truth_keys: raw_knowledge_changed_truth_keys.clone(),
                            page_node_keys: page_nodes
                                .iter()
                                .map(|page| page.page_node_key.clone())
                                .collect(),
                        },
                    )
                    .await?;
                    phase_reports.push(phase_report(
                        phase_label(input.key),
                        rebuild.verdict.as_str(),
                        "",
                        format!("impacted_pages={}", rebuild.impacted_page_node_keys.len()),
                    ));
                }
                _ => unreachable!("invalid final phase"),
            },
        }
        advance_cursor(&mut cursor);
    }

    Ok(SeoScenarioResult {
        scenario: kind_label(request.scenario).to_string(),
        mode: mode_label(request.mode).to_string(),
        status: if page_total == 0 && scenario_status == "done" {
            "done:no_pages".to_string()
        } else {
            scenario_status
        },
        page_total,
        published_pages,
        changed_truth_keys: raw_knowledge_changed_truth_keys,
        phase_reports,
    })
}
