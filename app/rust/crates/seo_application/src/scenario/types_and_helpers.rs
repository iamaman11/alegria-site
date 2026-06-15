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
    GraphCapabilityPort, GraphReasoningPort, PlanningRepository, ProjectionStatusRepository,
    PublishArtifactRepository, RebuildRepository, SectionTemplateRepository,
    SemanticLinkSearchPort, SeoBuildInputRepository, SerpSearchPort, SourceContextRepository,
    VerifiedSupportBundleRequest, VerifiedSupportRepository,
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

fn execution_plan_invariant_error(reason: &str) -> DomainError {
    DomainError::ContractViolation {
        message: format!("blocked:execution_plan_invariant:{reason}"),
    }
}

fn require_state_ref<'a, T>(value: &'a Option<T>, reason: &str) -> Result<&'a T, DomainError> {
    value
        .as_ref()
        .ok_or_else(|| execution_plan_invariant_error(reason))
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

