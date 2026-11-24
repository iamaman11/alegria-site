/// Shared recipe traversal: scenario-specific handlers provide only the ports
/// needed by their functional steps. Validation happens before this function
/// performs any I/O; the handler never silently skips an unsupported step.
#[async_trait::async_trait]
trait PrePageStepExecutor: Send {
    fn raw_knowledge_unchanged(&self) -> bool;

    async fn execute_step(&mut self, key: SeoPhaseKey) -> Result<Option<String>, DomainError>;
}

async fn traverse_pre_page_recipe<E: PrePageStepExecutor>(
    executor: &mut E,
    recipe: &SeoPhaseRecipe,
) -> Result<String, DomainError> {
    let mut final_status = "done".to_string();
    for (index, &key) in recipe.pre_page_steps().iter().enumerate() {
        // The refresh is necessary when no earlier producer loaded support or
        // when ingest changed truth. Independent recipes can interleave steps.
        if key == SeoPhaseKey::RefreshVerifiedSupportBundle
            && executor.raw_knowledge_unchanged()
            && recipe.pre_page_steps()[..index].contains(&SeoPhaseKey::LoadVerifiedSupportBundle)
        {
            continue;
        }
        if let Some(status) = executor.execute_step(key).await? {
            final_status = status;
        }
    }
    Ok(final_status)
}

async fn execute_initial_step<R>(
    repo: &R,
    request: &SeoScenarioRequest,
    state: &mut ScenarioState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<Option<String>, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository,
{
    run_initial_phase(repo, request, state, key, phase_reports).await?;
    if key == SeoPhaseKey::RawKnowledgeIngestion {
        enforce_projection_policy(
            repo,
            &request.site_input.run_id,
            request.policy.projection,
            phase_label(key),
            phase_reports,
        )
        .await
    } else {
        Ok(None)
    }
}

struct InitialStepExecutor<'a, R> {
    repo: &'a R,
    request: &'a SeoScenarioRequest,
    state: &'a mut ScenarioState,
    phase_reports: &'a mut Vec<SeoPhaseReport>,
}

#[async_trait::async_trait]
impl<R> PrePageStepExecutor for InitialStepExecutor<'_, R>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository,
{
    fn raw_knowledge_unchanged(&self) -> bool {
        raw_knowledge_unchanged(self.state)
    }

    async fn execute_step(&mut self, key: SeoPhaseKey) -> Result<Option<String>, DomainError> {
        if key.contract().segment != SeoExecutionSegment::Initial {
            return Err(execution_plan_invariant_error("unsupported_crawl_ingest_step"));
        }
        execute_initial_step(
            self.repo,
            self.request,
            self.state,
            key,
            self.phase_reports,
        )
        .await
    }
}

struct PlanningStepExecutor<'a, R> {
    repo: &'a R,
    request: &'a SeoScenarioRequest,
    state: &'a mut ScenarioState,
    phase_reports: &'a mut Vec<SeoPhaseReport>,
}

#[async_trait::async_trait]
impl<R> PrePageStepExecutor for PlanningStepExecutor<'_, R>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository
        + PlanningRepository
        + GraphReasoningPort
        + SemanticLinkSearchPort
        + GraphCapabilityPort,
{
    fn raw_knowledge_unchanged(&self) -> bool {
        raw_knowledge_unchanged(self.state)
    }

    async fn execute_step(&mut self, key: SeoPhaseKey) -> Result<Option<String>, DomainError> {
        match key.contract().segment {
            SeoExecutionSegment::Initial => {
                execute_initial_step(
                    self.repo,
                    self.request,
                    self.state,
                    key,
                    self.phase_reports,
                )
                .await
            }
            SeoExecutionSegment::Planning => {
                run_planning_phase(
                    self.repo,
                    self.request,
                    self.state,
                    key,
                    self.phase_reports,
                )
                .await
            }
            SeoExecutionSegment::PerPage | SeoExecutionSegment::Finalize => {
                Err(execution_plan_invariant_error("unsupported_pre_page_step"))
            }
        }
    }
}

fn raw_knowledge_unchanged(state: &ScenarioState) -> bool {
    state
        .raw_knowledge
        .as_ref()
        .is_some_and(|output| output.changed_truth_keys.is_empty())
}

fn require_scenario(
    request: &SeoScenarioRequest,
    expected: SeoScenarioKind,
) -> Result<(), DomainError> {
    if request.scenario != expected {
        return Err(execution_plan_invariant_error("scenario_kind_mismatch"));
    }
    Ok(())
}

fn validate_recipe_for_request(
    request: &SeoScenarioRequest,
    recipe: &SeoPhaseRecipe,
) -> Result<(), DomainError> {
    recipe
        .validate_for_scenario(&build_execution_plan(request))
        .map_err(|error| execution_plan_invariant_error(&error.to_string()))
}

/// Crawl ingestion only compiles against source, truth-support, SERP and
/// projection ports; no CMS, publishing, drafting or Neo4j port is required.
pub async fn execute_crawl_ingest_only<R>(
    repo: &R,
    request: &SeoScenarioRequest,
) -> Result<SeoScenarioResult, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository,
{
    require_scenario(request, SeoScenarioKind::CrawlIngestOnly)?;
    let recipe = SeoPhaseRecipe::for_plan(&build_execution_plan(request), true)
        .map_err(|error| execution_plan_invariant_error(&error.to_string()))?;
    execute_crawl_ingest_only_with_recipe(repo, request, &recipe).await
}

pub async fn execute_crawl_ingest_only_with_recipe<R>(
    repo: &R,
    request: &SeoScenarioRequest,
    recipe: &SeoPhaseRecipe,
) -> Result<SeoScenarioResult, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository,
{
    require_scenario(request, SeoScenarioKind::CrawlIngestOnly)?;
    validate_recipe_for_request(request, recipe)?;
    let mut state = ScenarioState::default();
    let mut phase_reports = Vec::new();
    let status = traverse_pre_page_recipe(
        &mut InitialStepExecutor {
            repo,
            request,
            state: &mut state,
            phase_reports: &mut phase_reports,
        },
        recipe,
    )
    .await?;
    let raw_knowledge =
        require_state_ref(&state.raw_knowledge, "raw_knowledge_ingestion_required")?;
    Ok(SeoScenarioResult {
        scenario: kind_label(request.scenario).to_string(),
        mode: mode_label(request.mode).to_string(),
        status: if status == "done" {
            raw_knowledge.status.clone()
        } else {
            status
        },
        page_total: 0,
        published_pages: 0,
        changed_truth_keys: raw_knowledge.changed_truth_keys.clone(),
        phase_reports,
    })
}

/// Planning uses graph/search/planning ports but has no compilation dependency
/// on any CMS review, editorial generation, publication or rebuild adapter.
pub async fn execute_planning_only<R>(
    repo: &R,
    request: &SeoScenarioRequest,
) -> Result<SeoScenarioResult, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository
        + PlanningRepository
        + GraphReasoningPort
        + SemanticLinkSearchPort
        + GraphCapabilityPort,
{
    require_scenario(request, SeoScenarioKind::PlanningOnly)?;
    let recipe = SeoPhaseRecipe::for_plan(&build_execution_plan(request), true)
        .map_err(|error| execution_plan_invariant_error(&error.to_string()))?;
    execute_planning_only_with_recipe(repo, request, &recipe).await
}

pub async fn execute_planning_only_with_recipe<R>(
    repo: &R,
    request: &SeoScenarioRequest,
    recipe: &SeoPhaseRecipe,
) -> Result<SeoScenarioResult, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
        + CrawlIngestRepository
        + SerpSearchPort
        + ProjectionStatusRepository
        + PlanningRepository
        + GraphReasoningPort
        + SemanticLinkSearchPort
        + GraphCapabilityPort,
{
    require_scenario(request, SeoScenarioKind::PlanningOnly)?;
    validate_recipe_for_request(request, recipe)?;
    let mut state = ScenarioState::default();
    let mut phase_reports = Vec::new();
    let status = traverse_pre_page_recipe(
        &mut PlanningStepExecutor {
            repo,
            request,
            state: &mut state,
            phase_reports: &mut phase_reports,
        },
        recipe,
    )
    .await?;
    let changed_truth_keys = require_state_ref(
        &state.raw_knowledge,
        "raw_knowledge_ingestion_required",
    )?
    .changed_truth_keys
    .clone();
    let (page_nodes, _) = page_nodes_and_links(&state)?;
    Ok(SeoScenarioResult {
        scenario: kind_label(request.scenario).to_string(),
        mode: mode_label(request.mode).to_string(),
        status: if page_nodes.is_empty() && status == "done" {
            "done:no_pages".to_string()
        } else {
            status
        },
        page_total: page_nodes.len() as u32,
        published_pages: 0,
        changed_truth_keys,
        phase_reports,
    })
}

pub async fn execute_site_build_scenario<R>(
    repo: &R,
    request: &SeoScenarioRequest,
) -> Result<SeoScenarioResult, DomainError>
where
    R: VerifiedSupportRepository
        + SerpIngestRepository
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
    R: VerifiedSupportRepository
        + SerpIngestRepository
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
    // Route existing callers through the same focused implementations, so
    // there is one source of truth for short-scenario results and policy.
    match request.scenario {
        SeoScenarioKind::CrawlIngestOnly => {
            return execute_crawl_ingest_only_with_recipe(repo, request, recipe).await;
        }
        SeoScenarioKind::PlanningOnly => {
            return execute_planning_only_with_recipe(repo, request, recipe).await;
        }
        _ => {}
    }
    // Validate all step contracts and publication policy before *any* I/O.
    validate_recipe_for_request(request, recipe)?;
    let mut state = ScenarioState::default();
    let mut phase_reports = Vec::new();
    // The same traversal drives both focused scenarios and the full runner.
    let final_status = traverse_pre_page_recipe(
        &mut PlanningStepExecutor {
            repo,
            request,
            state: &mut state,
            phase_reports: &mut phase_reports,
        },
        recipe,
    )
    .await?;

    let raw_knowledge_changed_truth_keys = state
        .raw_knowledge
        .as_ref()
        .ok_or_else(|| execution_plan_invariant_error("raw_knowledge_ingestion_required"))?
        .changed_truth_keys
        .clone();
    let (page_nodes, link_recommendations) = page_nodes_and_links(&state)?;
    let page_total = page_nodes.len() as u32;
    let factual_fragments = state
        .verified_support
        .iter()
        .map(|support| support.fragment_text.clone())
        .collect::<Vec<_>>();

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
