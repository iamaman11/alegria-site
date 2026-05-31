use serde::{Deserialize, Serialize};

use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload, ContentContractValidateInputPayload,
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DraftAssembleInputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, EditorialDraftGenerateInputPayload,
    EditorialDraftGenerateOutputPayload, FinalizePublishInputPayload,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, RebuildDetectInputPayload,
    RenderPreviewValidateInputPayload, SeoSiteBuildInputPayload, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{
    CmsReviewPort, CrawlIngestRepository, DraftRepository, EditorialGenerationPort,
    GraphReasoningPort, PlanningRepository, ProjectionStatusRepository, PublishArtifactRepository,
    RebuildRepository, SectionTemplateRepository, SemanticLinkSearchPort, SeoBuildInputRepository,
    SerpSearchPort, SourceContextRepository, VerifiedSupportBundleRequest,
    VerifiedSupportRepository,
};

use crate::execution::{
    advance_cursor, blocked_interaction_status, blocked_projection_status,
    blocked_publish_gate_status, build_execution_plan, final_phase_keys, initial_phase_keys,
    next_phase, page_phase_keys, phase_label, planning_phase_keys, RunInteractionPolicy,
    RunProjectionPolicy, SeoExecutionCursor, SeoExecutionSegment, SeoPhaseDecision, SeoPhaseKey,
    SeoRunPolicy,
};
use crate::{crawl_ingest, drafting, planning, rebuild_detect, review_publish, seo_runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeoExecutionMode {
    TemporalDurable,
    SemiAutoOperator,
    ManualTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeoScenarioKind {
    Full,
    PlanningOnly,
    DraftingOnly,
    PublishOnly,
    RebuildOnly,
    CrawlIngestOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeoScenarioRequest {
    pub scenario: SeoScenarioKind,
    pub mode: SeoExecutionMode,
    pub policy: SeoRunPolicy,
    pub output_dir: String,
    pub base_url: String,
    pub site_input: SeoSiteBuildInputPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SeoPhaseReport {
    pub phase: String,
    pub status: String,
    pub page_node_key: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SeoScenarioResult {
    pub scenario: String,
    pub mode: String,
    pub status: String,
    pub page_total: u32,
    pub published_pages: u32,
    pub changed_truth_keys: Vec<String>,
    pub phase_reports: Vec<SeoPhaseReport>,
}

#[derive(Default)]
struct ScenarioState {
    verified_support: Vec<contracts::generated::alegria::temporal::v1::SeoVerifiedFactSupportState>,
    ingest: Option<SerpIngestOutputPayload>,
    crawl_sources: Option<CrawlSourcesOutputPayload>,
    raw_knowledge: Option<RawKnowledgeIngestionOutputPayload>,
    serp: Option<SerpNormalizeOutputPayload>,
    opportunities: Option<OpportunityBuildOutputPayload>,
    ia: Option<IaBuildOutputPayload>,
    links: Option<LinkRecommendOutputPayload>,
    reconciled: Option<GlobalSiteReconcileOutputPayload>,
}

#[derive(Default)]
struct PageState {
    draft_plan: Option<DraftAssembleOutputPayload>,
    editorial_candidate: Option<EditorialDraftGenerateOutputPayload>,
    draft: Option<DraftNormalizeOutputPayload>,
    qa: Option<DraftQaOutputPayload>,
    cms_requested: Option<CmsPublishOutputPayload>,
    cms_approved: Option<CmsPublishOutputPayload>,
    materialized: Option<PublishMaterializeOutputPayload>,
}

fn phase_report(
    phase: &str,
    status: &str,
    page_node_key: &str,
    detail: impl Into<String>,
) -> SeoPhaseReport {
    SeoPhaseReport {
        phase: phase.to_string(),
        status: status.to_string(),
        page_node_key: page_node_key.to_string(),
        detail: detail.into(),
    }
}

fn scope_signature(input: &SeoSiteBuildInputPayload) -> String {
    input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default()
}

fn applicant_profile(input: &SeoSiteBuildInputPayload) -> String {
    input
        .scope
        .as_ref()
        .map(|scope| scope.applicant_profile.clone())
        .unwrap_or_default()
}

fn kind_label(kind: SeoScenarioKind) -> &'static str {
    match kind {
        SeoScenarioKind::Full => "site_build_full",
        SeoScenarioKind::PlanningOnly => "site_build_planning_only",
        SeoScenarioKind::DraftingOnly => "site_build_drafting_only",
        SeoScenarioKind::PublishOnly => "site_build_publish_only",
        SeoScenarioKind::RebuildOnly => "site_build_rebuild_only",
        SeoScenarioKind::CrawlIngestOnly => "site_build_crawl_ingest_only",
    }
}

fn mode_label(mode: SeoExecutionMode) -> &'static str {
    match mode {
        SeoExecutionMode::TemporalDurable => "temporal_durable",
        SeoExecutionMode::SemiAutoOperator => "semi_auto_operator",
        SeoExecutionMode::ManualTest => "manual_test",
    }
}

async fn enforce_projection_policy<R: ProjectionStatusRepository>(
    repo: &R,
    run_id: &str,
    policy: RunProjectionPolicy,
    checkpoint: &str,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<Option<String>, DomainError> {
    let status = repo.load_projection_barrier_status(run_id).await?;
    if status.blocked_events == 0 {
        phase_reports.push(phase_report(
            checkpoint,
            "projection_clear",
            "",
            format!(
                "blocked_events=0 max_open_lag_ms={}",
                status.max_open_lag_ms
            ),
        ));
        return Ok(None);
    }
    match policy {
        RunProjectionPolicy::ObserveProjectionBarrier => {
            phase_reports.push(phase_report(
                checkpoint,
                blocked_projection_status(),
                "",
                format!(
                    "blocked_events={} max_open_lag_ms={}",
                    status.blocked_events, status.max_open_lag_ms
                ),
            ));
            Ok(Some(blocked_projection_status().to_string()))
        }
        RunProjectionPolicy::WarnOnly => {
            phase_reports.push(phase_report(
                checkpoint,
                "projection_warn",
                "",
                format!(
                    "blocked_events={} max_open_lag_ms={}",
                    status.blocked_events, status.max_open_lag_ms
                ),
            ));
            Ok(None)
        }
    }
}

async fn run_initial_phase<
    R: VerifiedSupportRepository + PlanningRepository + CrawlIngestRepository + SerpSearchPort,
>(
    repo: &R,
    request: &SeoScenarioRequest,
    state: &mut ScenarioState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<(), DomainError> {
    let run_id = request.site_input.run_id.clone();
    match key {
        SeoPhaseKey::LoadVerifiedSupportBundle => {
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
                if state.verified_support.is_empty() {
                    "empty"
                } else {
                    "done"
                },
                "",
                format!("support_count={}", state.verified_support.len()),
            ));
        }
        SeoPhaseKey::SerpIngest => {
            let output = planning::run_serp_ingest(
                repo,
                repo,
                &SerpIngestInputPayload {
                    run_id,
                    query_batch_key: request.site_input.query_batch_key.clone(),
                    scope: request.site_input.scope.clone(),
                    queries: request.site_input.queries.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                "",
                format!(
                    "persisted_snapshot_count={}",
                    output.persisted_snapshot_count
                ),
            ));
            state.ingest = Some(output);
        }
        SeoPhaseKey::CrawlSources => {
            let ingest = state.ingest.as_ref().expect("serp_ingest must run first");
            let output = crawl_ingest::run_crawl_sources(
                repo,
                &CrawlSourcesInputPayload {
                    run_id,
                    query_batch_key: ingest.query_batch_key.clone(),
                    limit: 25,
                    emit_qdrant: true,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                "",
                format!(
                    "claimed={} crawled={} failed={} raw_pages={}",
                    output.claimed_count,
                    output.crawled_count,
                    output.failed_count,
                    output.raw_page_count
                ),
            ));
            state.crawl_sources = Some(output);
        }
        SeoPhaseKey::RawKnowledgeIngestion => {
            let ingest = state.ingest.as_ref().expect("serp_ingest must run first");
            let crawl_sources = state
                .crawl_sources
                .as_ref()
                .expect("crawl_sources must run first");
            let output = crawl_ingest::run_raw_knowledge_ingestion(
                repo,
                &RawKnowledgeIngestionInputPayload {
                    run_id,
                    context_key: request.site_input.context_key.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    raw_page_ids: crawl_sources.raw_page_ids.clone(),
                    source_policy: "candidate_only_truth_extraction@1".to_string(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                "",
                format!(
                    "verified_rules={} changed_truth_keys={}",
                    output.verified_rule_count,
                    output.changed_truth_keys.len()
                ),
            ));
            state.raw_knowledge = Some(output);
        }
        _ => unreachable!("invalid initial phase"),
    }
    Ok(())
}

async fn run_planning_phase<
    R: PlanningRepository
        + GraphReasoningPort
        + VerifiedSupportRepository
        + ProjectionStatusRepository
        + SerpSearchPort
        + SemanticLinkSearchPort
        + GraphReasoningPort,
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
            let ingest = state.ingest.as_ref().expect("serp_ingest required");
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
            let serp = state.serp.as_ref().expect("serp_normalize required");
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
            let opportunities = state
                .opportunities
                .as_ref()
                .expect("opportunity_build required");
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
            let ia = state.ia.as_ref().expect("ia_build required");
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
            let ia = state.ia.as_ref().expect("ia_build required");
            let links = state.links.as_ref().expect("link_recommend required");
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
) -> (
    Vec<contracts::generated::alegria::temporal::v1::PageNodeState>,
    Vec<contracts::generated::alegria::temporal::v1::LinkRecommendationState>,
) {
    let ia = state.ia.as_ref().expect("ia_build required");
    let links = state.links.as_ref().expect("link_recommend required");
    let reconciled = state
        .reconciled
        .as_ref()
        .expect("global_site_reconcile required");
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
    (page_nodes, link_recommendations)
}

async fn run_page_phase<
    R: DraftRepository
        + SectionTemplateRepository
        + SourceContextRepository
        + EditorialGenerationPort
        + CmsReviewPort
        + PublishArtifactRepository,
>(
    repo: &R,
    request: &SeoScenarioRequest,
    page_node: &contracts::generated::alegria::temporal::v1::PageNodeState,
    page_blueprint: &contracts::generated::alegria::temporal::v1::PageBlueprintState,
    required_links: &[contracts::generated::alegria::temporal::v1::LinkRecommendationState],
    factual_fragments: &[String],
    verified_support: &[contracts::generated::alegria::temporal::v1::SeoVerifiedFactSupportState],
    page_state: &mut PageState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<Option<String>, DomainError> {
    let run_id = request.site_input.run_id.clone();
    match key {
        SeoPhaseKey::DraftAssemble => {
            let output = drafting::run_draft_assemble(
                repo,
                &DraftAssembleInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    page_blueprint: Some(page_blueprint.clone()),
                    factual_fragments: factual_fragments.to_vec(),
                    verified_support: verified_support.to_vec(),
                    required_links: required_links.to_vec(),
                    section_templates: Vec::new(),
                    llm_candidate: None,
                    source_context_chunks: Vec::new(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "assembled",
            ));
            page_state.draft_plan = Some(output);
            Ok(None)
        }
        SeoPhaseKey::EditorialDraftGenerate => {
            let draft_plan = page_state
                .draft_plan
                .as_ref()
                .expect("draft_assemble required");
            let output = drafting::run_editorial_draft_generate(
                repo,
                &EditorialDraftGenerateInputPayload {
                    run_id,
                    request: draft_plan.llm_request.clone(),
                    provider_policy: "multi_provider:first_available".to_string(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                &page_node.page_node_key,
                output.provider_key.clone(),
            ));
            page_state.editorial_candidate = Some(output);
            Ok(None)
        }
        SeoPhaseKey::DraftNormalize => {
            let draft_plan = page_state
                .draft_plan
                .as_ref()
                .expect("draft_assemble required");
            let editorial_candidate = page_state
                .editorial_candidate
                .as_ref()
                .expect("editorial_draft_generate required");
            let output = drafting::run_draft_normalize(
                repo,
                &DraftNormalizeInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    page_blueprint: Some(page_blueprint.clone()),
                    factual_fragments: factual_fragments.to_vec(),
                    verified_support: verified_support.to_vec(),
                    required_links: required_links.to_vec(),
                    section_templates: draft_plan
                        .editorial_brief
                        .as_ref()
                        .map(|brief| brief.section_templates.clone())
                        .unwrap_or_default(),
                    content_block_plan: draft_plan.content_block_plan.clone(),
                    llm_candidate: editorial_candidate.candidate.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "normalized",
            ));
            page_state.draft = Some(output);
            Ok(None)
        }
        SeoPhaseKey::ContentContractValidate => {
            let draft = page_state.draft.as_ref().expect("draft_normalize required");
            let _output = drafting::run_content_contract_validate(
                repo,
                &ContentContractValidateInputPayload {
                    run_id,
                    draft: draft.draft.clone(),
                    required_links: required_links.to_vec(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "validated",
            ));
            Ok(None)
        }
        SeoPhaseKey::DraftQa => {
            let draft = page_state.draft.as_ref().expect("draft_normalize required");
            let output = drafting::run_draft_qa(
                repo,
                &DraftQaInputPayload {
                    run_id,
                    draft: draft.draft.clone(),
                    supported_fragments: factual_fragments.to_vec(),
                    required_links: required_links.to_vec(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                "qa_complete",
            ));
            if let Some(draft_state) = page_state
                .draft
                .as_mut()
                .and_then(|state| state.draft.as_mut())
            {
                draft_state.qa_verdict = output.verdict.clone();
            }
            page_state.qa = Some(output);
            Ok(None)
        }
        SeoPhaseKey::CmsRequestReview => {
            let draft = page_state.draft.as_ref().expect("draft_normalize required");
            let output = review_publish::run_cms_publish(
                repo,
                &CmsPublishInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    draft: draft.draft.clone(),
                    actor_role: "seo_system".to_string(),
                    publish_mode: "request_review".to_string(),
                    approval_decision: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "review_requested" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.cms_requested = Some(output);
            Ok(None)
        }
        SeoPhaseKey::HumanApprovalWait => {
            let cms_requested = page_state
                .cms_requested
                .as_ref()
                .expect("cms_request_review required");
            match request.policy.interaction {
                RunInteractionPolicy::AllowHitlPause | RunInteractionPolicy::FailIfHitlRequired => {
                    let status = blocked_interaction_status(request.policy.interaction).to_string();
                    phase_reports.push(phase_report(
                        phase_label(key),
                        &status,
                        &page_node.page_node_key,
                        cms_requested.revision_id.clone(),
                    ));
                    Ok(Some(status))
                }
                RunInteractionPolicy::RequirePreApprovedDecision => {
                    match review_publish::load_cms_approval_decision(
                        repo,
                        &page_node.page_node_key,
                        &cms_requested.revision_id,
                    )
                    .await
                    {
                        Ok(decision) if decision.decision == "approved" => {
                            phase_reports.push(phase_report(
                                phase_label(key),
                                "approved",
                                &page_node.page_node_key,
                                decision.revision_id.clone(),
                            ));
                            Ok(None)
                        }
                        Ok(decision) => {
                            let status =
                                blocked_interaction_status(request.policy.interaction).to_string();
                            phase_reports.push(phase_report(
                                phase_label(key),
                                &status,
                                &page_node.page_node_key,
                                decision.decision,
                            ));
                            Ok(Some(status))
                        }
                        Err(_) => {
                            let status =
                                blocked_interaction_status(request.policy.interaction).to_string();
                            phase_reports.push(phase_report(
                                phase_label(key),
                                &status,
                                &page_node.page_node_key,
                                "missing_approval_decision",
                            ));
                            Ok(Some(status))
                        }
                    }
                }
            }
        }
        SeoPhaseKey::CmsPublishApproved => {
            let draft = page_state.draft.as_ref().expect("draft_normalize required");
            let cms_requested = page_state
                .cms_requested
                .as_ref()
                .expect("cms_request_review required");
            let approval = review_publish::load_cms_approval_decision(
                repo,
                &page_node.page_node_key,
                &cms_requested.revision_id,
            )
            .await
            .ok();
            let output = review_publish::run_cms_publish(
                repo,
                &CmsPublishInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    draft: draft.draft.clone(),
                    actor_role: "seo_system".to_string(),
                    publish_mode: "approved_publish".to_string(),
                    approval_decision: approval,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "approved" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.cms_approved = Some(output);
            Ok(None)
        }
        SeoPhaseKey::PublishMaterialize => {
            let cms_requested = page_state
                .cms_requested
                .as_ref()
                .expect("cms_request_review required");
            let output = review_publish::run_publish_materialize(
                repo,
                &PublishMaterializeInputPayload {
                    run_id,
                    page_node_key: page_node.page_node_key.clone(),
                    revision_id: cms_requested.revision_id.clone(),
                    cms_document_id: cms_requested.cms_document_id.clone(),
                    canonical_url_path: page_node.canonical_url_path.clone(),
                    publish_artifact: cms_requested.publish_artifact.clone().or(page_state
                        .cms_approved
                        .as_ref()
                        .and_then(|approved| approved.publish_artifact.clone())),
                    output_dir: request.output_dir.clone(),
                    base_url: request.base_url.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.materialization_status,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.materialization_status.contains("failed") {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.materialized = Some(output);
            Ok(None)
        }
        SeoPhaseKey::RenderPreviewValidate => {
            let materialized = page_state
                .materialized
                .as_ref()
                .expect("publish_materialize required");
            let output =
                review_publish::run_render_preview_validate(&RenderPreviewValidateInputPayload {
                    run_id,
                    preview_pages: materialized.preview_pages.clone(),
                });
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "render_ready" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            Ok(None)
        }
        SeoPhaseKey::FinalizePublish => {
            let cms_requested = page_state
                .cms_requested
                .as_ref()
                .expect("cms_request_review required");
            let materialized = page_state
                .materialized
                .as_ref()
                .expect("publish_materialize required");
            let render_validation =
                review_publish::run_render_preview_validate(&RenderPreviewValidateInputPayload {
                    run_id: run_id.clone(),
                    preview_pages: materialized.preview_pages.clone(),
                });
            let output = review_publish::run_finalize_publish(
                repo,
                &FinalizePublishInputPayload {
                    run_id,
                    page_node_key: page_node.page_node_key.clone(),
                    revision_id: cms_requested.revision_id.clone(),
                    publish_artifact: materialized.publish_artifact.clone(),
                    render_validation: Some(render_validation),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output
                    .publish_artifact
                    .as_ref()
                    .map(|artifact| artifact.artifact_uri.clone())
                    .unwrap_or_default(),
            ));
            if output.verdict != "published" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            Ok(None)
        }
        _ => unreachable!("invalid page phase"),
    }
}

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
        + GraphReasoningPort,
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
        .expect("raw_knowledge_ingestion required")
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
                    .expect("raw_knowledge_ingestion required")
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

    let (page_nodes, link_recommendations) = page_nodes_and_links(&state);
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
    let ia = state.ia.as_ref().expect("ia_build required");
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        CmsApprovalDecision, EditorialDraftGenerateOutputPayload, GlobalSiteReconcileOutputPayload,
        IaBuildOutputPayload, LinkRecommendOutputPayload, LlmDraftCandidate,
        OpportunityBuildOutputPayload, RawKnowledgeIngestionOutputPayload, SectionTemplateBinding,
        SeoScopePayload, SeoVerifiedFactSupportState, SerpIngestOutputPayload,
        SerpNormalizeOutputPayload,
    };
    use seo_ports::{
        CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
        CrawlIngestRepository, GraphCoverageEvaluation, GraphNeighborhoodHit, GraphReasoningPort,
        OrganicSerpResponse, PlanningRepository, ProjectionBarrierStatus,
        ProjectionStatusRepository, PublishArtifactRepository, RebuildDependencyEvidence,
        RebuildRepository, SemanticLinkCandidate, SemanticLinkSearchPort, SeoBuildInputRepository,
        SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest, SerpSearchPort,
        SourceContextRepository, VerifiedSupportBundleRequest, VerifiedSupportRepository,
    };

    #[derive(Default)]
    struct FakeRepo;

    #[async_trait]
    impl SeoBuildInputRepository for FakeRepo {
        async fn load_site_build_input(
            &self,
            _run_id: &str,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl SeoBuildRegistrationRepository for FakeRepo {
        async fn register_site_build_input(
            &self,
            _request: &SeoSiteBuildRegistrationRequest,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl VerifiedSupportRepository for FakeRepo {
        async fn load_verified_support_bundle(
            &self,
            _request: &VerifiedSupportBundleRequest,
        ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
            Ok(vec![SeoVerifiedFactSupportState {
                fragment_text: "Passport required".to_string(),
                support_ref: "rule:passport".to_string(),
                role_type: "document_required".to_string(),
                source_label: "Consulate".to_string(),
                source_tier: "official".to_string(),
                freshness_class: "watch".to_string(),
                observed_at: String::new(),
                valid_until: String::new(),
            }])
        }
    }

    #[async_trait]
    impl CrawlIngestRepository for FakeRepo {
        async fn crawl_sources(
            &self,
            _input: &CrawlSourcesInputPayload,
        ) -> Result<CrawlSourcesOutputPayload, DomainError> {
            Ok(CrawlSourcesOutputPayload {
                claimed_count: 1,
                crawled_count: 1,
                failed_count: 0,
                raw_page_count: 1,
                raw_section_count: 1,
                qdrant_event_count: 0,
                status: "done".to_string(),
                raw_page_ids: vec![7],
                failed_urls: Vec::new(),
            })
        }

        async fn ingest_raw_knowledge(
            &self,
            _input: &RawKnowledgeIngestionInputPayload,
        ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
            Ok(RawKnowledgeIngestionOutputPayload {
                raw_page_count: 1,
                raw_section_count: 1,
                extracted_rule_count: 1,
                verified_rule_count: 1,
                outbox_event_count: 1,
                changed_truth_keys: vec!["verified.rule_instance:passport".to_string()],
                status: "done".to_string(),
            })
        }
    }

    #[async_trait]
    impl ProjectionStatusRepository for FakeRepo {
        async fn load_projection_barrier_status(
            &self,
            _run_id: &str,
        ) -> Result<ProjectionBarrierStatus, DomainError> {
            Ok(ProjectionBarrierStatus::default())
        }
    }

    #[async_trait]
    impl SerpSearchPort for FakeRepo {
        async fn fetch_google_organic_live_advanced(
            &self,
            _locale: Option<&str>,
            _query: &str,
        ) -> Result<Option<OrganicSerpResponse>, DomainError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl SemanticLinkSearchPort for FakeRepo {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(Vec::new())
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(Vec::new())
        }

        async fn cluster_demand_queries(
            &self,
            _queries: &[String],
        ) -> Result<Vec<seo_ports::SemanticDemandCluster>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl PlanningRepository for FakeRepo {
        async fn load_graph_planning_context(
            &self,
            _run_id: &str,
            _scope_signature: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }

        async fn persist_serp_ingest_output(
            &self,
            _input: &SerpIngestInputPayload,
            _output: &SerpIngestOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_live_serp_query_results(
            &self,
            _run_id: &str,
            _query_batch_key: &str,
            _ordinal: usize,
            _query: &str,
            _response: &OrganicSerpResponse,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_serp_normalize_output(
            &self,
            _input: &SerpNormalizeInputPayload,
            _output: &SerpNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_opportunity_build_output(
            &self,
            _input: &OpportunityBuildInputPayload,
            _output: &OpportunityBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_ia_build_output(
            &self,
            _input: &IaBuildInputPayload,
            _output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_link_recommend_output(
            &self,
            _input: &LinkRecommendInputPayload,
            _output: &LinkRecommendOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_global_site_reconcile_output(
            &self,
            _input: &GlobalSiteReconcileInputPayload,
            output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<seo_ports::GlobalNavigationPersistReport, DomainError> {
            Ok(seo_ports::GlobalNavigationPersistReport {
                navigation_tree_key: "synthetic-navigation".to_string(),
                scope_count: 1,
                page_item_count: output.page_nodes.len() as u64,
                silo_group_count: 0,
                rebuild_plan_count: 0,
            })
        }
    }

    #[async_trait]
    impl GraphReasoningPort for FakeRepo {
        async fn load_planning_graph_context(
            &self,
            _scope_signature: &str,
            _run_id: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }

        async fn find_conflict_neighborhood(
            &self,
            _scope_signature: &str,
            _page_node_key: &str,
        ) -> Result<Vec<GraphNeighborhoodHit>, DomainError> {
            Ok(Vec::new())
        }

        async fn find_rebuild_impact_neighborhood(
            &self,
            _changed_truth_keys: &[String],
            _page_nodes: &[contracts::generated::alegria::temporal::v1::PageNodeState],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }

        async fn evaluate_draft_coverage_neighborhood(
            &self,
            page_node_key: &str,
            _draft_markdown: &str,
        ) -> Result<GraphCoverageEvaluation, DomainError> {
            Ok(GraphCoverageEvaluation {
                page_node_key: page_node_key.to_string(),
                coverage_score: 0.0,
                missing_topics: Vec::new(),
                reason_codes: vec!["graph_draft_coverage".to_string()],
                support_refs: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl SectionTemplateRepository for FakeRepo {
        async fn load_section_templates(
            &self,
            page_type_key: &str,
            dominant_intent: &str,
        ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
            Ok(vec![SectionTemplateBinding {
                template_key: "tmpl:overview".to_string(),
                page_type_key: page_type_key.to_string(),
                dominant_intent: dominant_intent.to_string(),
                section_role: "overview".to_string(),
                heading: "Overview".to_string(),
                template_body: "overview template".to_string(),
                template_version: 1,
                required: true,
            }])
        }
    }

    #[async_trait]
    impl SourceContextRepository for FakeRepo {
        async fn load_source_context_chunks(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<
            Vec<contracts::generated::alegria::temporal::v1::SourceContextChunkState>,
            DomainError,
        > {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl DraftRepository for FakeRepo {
        async fn persist_draft_assemble_output(
            &self,
            _run_id: &str,
            _output: &DraftAssembleOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_draft_normalize_output(
            &self,
            _input: &DraftNormalizeInputPayload,
            _output: &DraftNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_content_contract_validate_output(
            &self,
            _input: &ContentContractValidateInputPayload,
            _output: &contracts::generated::alegria::temporal::v1::ContentContractValidateOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn persist_draft_qa_output(
            &self,
            _input: &DraftQaInputPayload,
            _output: &DraftQaOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl EditorialGenerationPort for FakeRepo {
        async fn generate_editorial_draft(
            &self,
            input: &EditorialDraftGenerateInputPayload,
        ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
            Ok(EditorialDraftGenerateOutputPayload {
                provider_key: "fake".to_string(),
                model_key: "fake-model".to_string(),
                status: "selected".to_string(),
                candidate: Some(LlmDraftCandidate {
                    candidate_key: "cand-1".to_string(),
                    request_key: input
                        .request
                        .as_ref()
                        .map(|x| x.request_key.clone())
                        .unwrap_or_default(),
                    provider_key: "fake".to_string(),
                    model_key: "fake-model".to_string(),
                    prompt_version: "prompt@1".to_string(),
                    body_markdown: "draft".to_string(),
                    sections: Vec::new(),
                    claim_ledger: Vec::new(),
                    content_blocks: Vec::new(),
                    faq_json: "[]".to_string(),
                    schema_markup_json: "{}".to_string(),
                    status: "selected".to_string(),
                }),
            })
        }
    }

    #[async_trait]
    impl CmsReviewPort for FakeRepo {
        async fn persist_cms_publish_output(
            &self,
            _input: &CmsPublishInputPayload,
            output: &CmsPublishOutputPayload,
        ) -> Result<CmsPublishOutputPayload, DomainError> {
            Ok(output.clone())
        }
        async fn load_latest_approval_decision(
            &self,
            _page_node_key: &str,
            _revision_id: &str,
        ) -> Result<Option<CmsApprovalDecision>, DomainError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl PublishArtifactRepository for FakeRepo {
        async fn persist_publish_materialize_output(
            &self,
            _input: &PublishMaterializeInputPayload,
            output: &PublishMaterializeOutputPayload,
        ) -> Result<PublishMaterializeOutputPayload, DomainError> {
            Ok(output.clone())
        }
        async fn persist_finalize_publish_output(
            &self,
            _input: &FinalizePublishInputPayload,
            output: &contracts::generated::alegria::temporal::v1::FinalizePublishOutputPayload,
        ) -> Result<
            contracts::generated::alegria::temporal::v1::FinalizePublishOutputPayload,
            DomainError,
        > {
            Ok(output.clone())
        }
    }

    #[async_trait]
    impl RebuildRepository for FakeRepo {
        async fn narrow_rebuild_impacts(
            &self,
            _changed_truth_keys: &[String],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }
        async fn persist_rebuild_detect_output(
            &self,
            _input: &RebuildDetectInputPayload,
            _output: &contracts::generated::alegria::temporal::v1::RebuildDetectOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn semantic_neighbor_impacts(
            &self,
            _changed_truth_keys: &[String],
            _page_nodes: &[contracts::generated::alegria::temporal::v1::PageNodeState],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl CmsReviewDecisionPort for FakeRepo {
        async fn apply_human_review_decision(
            &self,
            _request: &CmsReviewDecisionRequest,
        ) -> Result<CmsReviewDecisionOutcome, DomainError> {
            unreachable!()
        }
    }

    fn sample_site_input() -> SeoSiteBuildInputPayload {
        SeoSiteBuildInputPayload {
            run_id: "run-1".to_string(),
            context_key: "ES|tourist||BY".to_string(),
            scope: Some(SeoScopePayload {
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                applicant_profile: "standard".to_string(),
                raw_scope_tuple: String::new(),
                scope_signature: "sig-1".to_string(),
            }),
            query_batch_key: "batch-1".to_string(),
            queries: vec!["spain tourist visa".to_string()],
            verified_support: Vec::new(),
            required_page_types: Vec::new(),
            run_mode: "publish_with_hitl".to_string(),
        }
    }

    #[tokio::test]
    async fn scenario_planning_only_uses_shared_plan() {
        let repo = FakeRepo;
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::PlanningOnly,
                mode: SeoExecutionMode::ManualTest,
                policy: SeoRunPolicy::for_manual_test(),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.scenario, "site_build_planning_only");
        assert!(result
            .phase_reports
            .iter()
            .any(|r| r.phase == "global_site_reconcile"));
    }

    #[tokio::test]
    async fn scenario_publish_blocks_without_human_approval() {
        let repo = FakeRepo;
        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::PublishOnly,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(true, false, true),
                output_dir: "/tmp".to_string(),
                base_url: "https://example.com".to_string(),
                site_input: sample_site_input(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.status, "blocked_publish_gate");
    }

    #[test]
    fn scenario_report_surface_serializes_phase_reports() {
        let result = SeoScenarioResult {
            scenario: "site_build_full".to_string(),
            mode: "temporal_durable".to_string(),
            status: "ok".to_string(),
            page_total: 1,
            published_pages: 0,
            changed_truth_keys: vec!["truth_key".to_string()],
            phase_reports: vec![SeoPhaseReport {
                phase: "draft_assemble".to_string(),
                status: "done".to_string(),
                page_node_key: "page_1".to_string(),
                detail: "assembled".to_string(),
            }],
        };

        assert_eq!(result.phase_reports[0].phase, "draft_assemble");
        assert_eq!(result.phase_reports[0].status, "done");
        assert_eq!(result.changed_truth_keys[0], "truth_key");
    }
}
