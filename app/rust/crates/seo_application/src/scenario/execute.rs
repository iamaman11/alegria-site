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
        + SourceContextRepository
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
    let mut state = ScenarioState::default();
    let mut phase_reports = Vec::new();
    let mut final_status = "done".to_string();

    let mut cursor = SeoExecutionCursor {
        segment: Some(SeoExecutionSegment::Initial),
        ..Default::default()
    };
    loop {
        match next_phase(initial_phase_keys(), &cursor) {
            SeoPhaseDecision::Complete(_) => break,
            SeoPhaseDecision::Run(input) => {
                run_initial_phase(repo, request, &mut state, input.key, &mut phase_reports).await?;
                if input.key == SeoPhaseKey::RawKnowledgeIngestion {
                    if let Some(status) = enforce_projection_policy(
                        repo,
                        &request.site_input.run_id,
                        request.policy.projection,
                        phase_label(input.key),
                        &mut phase_reports,
                    )
                    .await?
                    {
                        final_status = status;
                    }
                }
            }
        }
        advance_cursor(&mut cursor);
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

    cursor = SeoExecutionCursor {
        segment: Some(SeoExecutionSegment::Planning),
        ..Default::default()
    };
    let planning_keys = planning_phase_keys(&plan, !raw_knowledge_changed_truth_keys.is_empty());
    loop {
        match next_phase(&planning_keys, &cursor) {
            SeoPhaseDecision::Complete(_) => break,
            SeoPhaseDecision::Run(input) => {
                if let Some(status) =
                    run_planning_phase(repo, request, &mut state, input.key, &mut phase_reports)
                        .await?
                {
                    final_status = status;
                }
            }
        }
        advance_cursor(&mut cursor);
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

    let mut scenario_status = final_status;
    let mut published_pages = 0u32;
    let page_keys = page_phase_keys(&plan);
    let ia = require_state_ref(&state.ia, "ia_build_required")?;
    for page_node in page_nodes.iter().cloned() {
        let page_blueprint = ia
            .page_blueprints
            .iter()
            .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
            .cloned()
            .unwrap_or_default();
        let required_links = link_recommendations
            .iter()
            .filter(|link| link.required_flag && link.source_page_key == page_node.page_node_key)
            .cloned()
            .collect::<Vec<_>>();
        let mut page_state = PageState::default();
        cursor = SeoExecutionCursor {
            segment: Some(SeoExecutionSegment::PerPage),
            page_total: page_total as usize,
            ..Default::default()
        };
        loop {
            match next_phase(&page_keys, &cursor) {
                SeoPhaseDecision::Complete(_) => break,
                SeoPhaseDecision::Run(input) => {
                    if let Some(status) = run_page_phase(
                        repo,
                        request,
                        &page_node,
                        &page_blueprint,
                        &required_links,
                        &factual_fragments,
                        &state.verified_support,
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
                    }
                }
            }
            advance_cursor(&mut cursor);
        }
        if scenario_status.starts_with("blocked") || scenario_status == "hitl_required" {
            break;
        }
    }

    cursor = SeoExecutionCursor {
        segment: Some(SeoExecutionSegment::Finalize),
        ..Default::default()
    };
    loop {
        match next_phase(final_phase_keys(&plan), &cursor) {
            SeoPhaseDecision::Complete(_) => break,
            SeoPhaseDecision::Run(input) => match input.key {
                SeoPhaseKey::RebuildDetect => {
                    let rebuild = rebuild_detect::execute(
                        repo,
                        &RebuildDetectInputPayload {
                            run_id: request.site_input.run_id.clone(),
                            changed_truth_keys: raw_knowledge_changed_truth_keys.clone(),
                            page_nodes: page_nodes.clone(),
                        },
                    )
                    .await?;
                    phase_reports.push(phase_report(
                        phase_label(input.key),
                        &rebuild.verdict,
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

