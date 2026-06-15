async fn run_planning_phase<
    R: PlanningRepository
        + GraphReasoningPort
        + VerifiedSupportRepository
        + ProjectionStatusRepository
        + SerpSearchPort
        + SemanticLinkSearchPort
        + GraphReasoningPort
        + GraphCapabilityPort,
>(
    repo: &R,
    request: &SeoScenarioRequest,
    state: &mut ScenarioState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<Option<String>, DomainError> {
    let run_id = request.site_input.run_id.clone();
    match key {
        SeoPhaseKey::RefreshVerifiedSupportBundle => {
            state.verified_support = seo_runtime::load_verified_support_bundle(
                repo,
                &VerifiedSupportBundleRequest {
                    run_id,
                    context_key: request.site_input.context_key.clone(),
                    scope_signature: scope_signature(&request.site_input),
                    applicant_profile: applicant_profile(&request.site_input),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("support_count={}", state.verified_support.len()),
            ));
            Ok(None)
        }
        SeoPhaseKey::SerpNormalize => {
            let ingest = require_state_ref(&state.ingest, "serp_ingest_required")?;
            let output = planning::run_serp_normalize(
                repo,
                repo,
                &SerpNormalizeInputPayload {
                    run_id,
                    query_batch_key: ingest.query_batch_key.clone(),
                    scope: request.site_input.scope.clone(),
                    queries: request.site_input.queries.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("serp_patterns={}", output.serp_patterns.len()),
            ));
            state.serp = Some(output);
            Ok(None)
        }
        SeoPhaseKey::OpportunityBuild => {
            let serp = require_state_ref(&state.serp, "serp_normalize_required")?;
            let output = planning::run_opportunity_build(
                repo,
                repo,
                &OpportunityBuildInputPayload {
                    run_id,
                    scope: request.site_input.scope.clone(),
                    serp_patterns: serp.serp_patterns.clone(),
                    graph_context: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("keyword_clusters={}", output.keyword_clusters.len()),
            ));
            state.opportunities = Some(output);
            Ok(None)
        }
        SeoPhaseKey::IaBuild => {
            let opportunities =
                require_state_ref(&state.opportunities, "opportunity_build_required")?;
            let output = planning::run_ia_build(
                repo,
                repo,
                &IaBuildInputPayload {
                    run_id,
                    scope: request.site_input.scope.clone(),
                    keyword_clusters: opportunities.keyword_clusters.clone(),
                    graph_context: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("page_nodes={}", output.page_nodes.len()),
            ));
            state.ia = Some(output);
            Ok(None)
        }
        SeoPhaseKey::LinkRecommend => {
            let ia = require_state_ref(&state.ia, "ia_build_required")?;
            let output = planning::run_link_recommend(
                repo,
                repo,
                &LinkRecommendInputPayload {
                    run_id,
                    page_nodes: ia.page_nodes.clone(),
                    max_links_per_page: 3,
                    graph_context: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("link_recommendations={}", output.link_recommendations.len()),
            ));
            state.links = Some(output);
            Ok(None)
        }
        SeoPhaseKey::GlobalSiteReconcile => {
            let ia = require_state_ref(&state.ia, "ia_build_required")?;
            let links = require_state_ref(&state.links, "link_recommend_required")?;
            let output = planning::run_global_site_reconcile(
                repo,
                repo,
                &GlobalSiteReconcileInputPayload {
                    run_id,
                    scope: request.site_input.scope.clone(),
                    page_nodes: ia.page_nodes.clone(),
                    link_recommendations: links.link_recommendations.clone(),
                    reconcile_reason: format!("{}@1", kind_label(request.scenario)),
                    graph_context: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!("page_nodes={}", output.page_nodes.len()),
            ));
            state.reconciled = Some(output);
            enforce_projection_policy(
                repo,
                &request.site_input.run_id,
                request.policy.projection,
                phase_label(key),
                phase_reports,
            )
            .await
        }
        _ => unreachable!("invalid planning phase"),
    }
}

fn page_nodes_and_links(
    state: &ScenarioState,
) -> Result<
    (
    Vec<contracts::generated::alegria::temporal::v1::PageNodeState>,
    Vec<contracts::generated::alegria::temporal::v1::LinkRecommendationState>,
    ),
    DomainError,
> {
    let ia = require_state_ref(&state.ia, "ia_build_required")?;
    let links = require_state_ref(&state.links, "link_recommend_required")?;
    let reconciled = require_state_ref(&state.reconciled, "global_site_reconcile_required")?;
    let page_nodes = if reconciled.page_nodes.is_empty() {
        ia.page_nodes.clone()
    } else {
        reconciled.page_nodes.clone()
    };
    let link_recommendations = if reconciled.link_recommendations.is_empty() {
        links.link_recommendations.clone()
    } else {
        reconciled.link_recommendations.clone()
    };
    Ok((page_nodes, link_recommendations))
}

