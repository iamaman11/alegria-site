use crate::scenario::{SeoExecutionMode, SeoScenarioKind, SeoScenarioRequest};

pub const SEO_RUN_MODE_DRY_RUN: &str = "dry_run";
pub const SEO_RUN_MODE_CRAWL_ONLY: &str = "crawl_only";
pub const SEO_RUN_MODE_DRAFT_ONLY: &str = "draft_only";
pub const SEO_RUN_MODE_PUBLISH_WITH_HITL: &str = "publish_with_hitl";
pub const SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL: &str = "full_auto_after_approval";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPersistencePolicy {
    PersistAll,
    PersistWithoutPublish,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunInteractionPolicy {
    AllowHitlPause,
    FailIfHitlRequired,
    RequirePreApprovedDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunProjectionPolicy {
    ObserveProjectionBarrier,
    WarnOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPublishPolicy {
    NoPublish,
    PublishIfApproved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeoRunPolicy {
    pub persistence: RunPersistencePolicy,
    pub interaction: RunInteractionPolicy,
    pub projection: RunProjectionPolicy,
    pub publish: RunPublishPolicy,
}

impl SeoRunPolicy {
    pub fn for_temporal_durable() -> Self {
        Self {
            persistence: RunPersistencePolicy::PersistAll,
            interaction: RunInteractionPolicy::AllowHitlPause,
            projection: RunProjectionPolicy::ObserveProjectionBarrier,
            publish: RunPublishPolicy::PublishIfApproved,
        }
    }

    pub fn for_semi_auto_operator(
        publish: bool,
        require_preapproved_decision: bool,
        warn_only_projections: bool,
    ) -> Self {
        Self {
            persistence: if publish {
                RunPersistencePolicy::PersistAll
            } else {
                RunPersistencePolicy::PersistWithoutPublish
            },
            interaction: if require_preapproved_decision {
                RunInteractionPolicy::RequirePreApprovedDecision
            } else {
                RunInteractionPolicy::FailIfHitlRequired
            },
            projection: if warn_only_projections {
                RunProjectionPolicy::WarnOnly
            } else {
                RunProjectionPolicy::ObserveProjectionBarrier
            },
            publish: if publish {
                RunPublishPolicy::PublishIfApproved
            } else {
                RunPublishPolicy::NoPublish
            },
        }
    }

    pub fn for_manual_test() -> Self {
        Self {
            persistence: RunPersistencePolicy::DryRun,
            interaction: RunInteractionPolicy::FailIfHitlRequired,
            projection: RunProjectionPolicy::WarnOnly,
            publish: RunPublishPolicy::NoPublish,
        }
    }
}

pub fn normalize_run_mode(run_mode: &str) -> &'static str {
    match run_mode.trim() {
        SEO_RUN_MODE_DRY_RUN => SEO_RUN_MODE_DRY_RUN,
        SEO_RUN_MODE_CRAWL_ONLY => SEO_RUN_MODE_CRAWL_ONLY,
        SEO_RUN_MODE_DRAFT_ONLY => SEO_RUN_MODE_DRAFT_ONLY,
        SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL => SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL,
        SEO_RUN_MODE_PUBLISH_WITH_HITL => SEO_RUN_MODE_PUBLISH_WITH_HITL,
        _ => SEO_RUN_MODE_PUBLISH_WITH_HITL,
    }
}

pub fn scenario_kind_for_run_mode(run_mode: &str) -> SeoScenarioKind {
    match normalize_run_mode(run_mode) {
        SEO_RUN_MODE_CRAWL_ONLY => SeoScenarioKind::CrawlIngestOnly,
        SEO_RUN_MODE_DRY_RUN | SEO_RUN_MODE_DRAFT_ONLY => SeoScenarioKind::Full,
        SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL | SEO_RUN_MODE_PUBLISH_WITH_HITL => {
            SeoScenarioKind::Full
        }
        _ => SeoScenarioKind::Full,
    }
}

pub fn policy_for_run_mode(run_mode: &str, execution_mode: SeoExecutionMode) -> SeoRunPolicy {
    match normalize_run_mode(run_mode) {
        SEO_RUN_MODE_CRAWL_ONLY => match execution_mode {
            SeoExecutionMode::TemporalDurable => SeoRunPolicy {
                persistence: RunPersistencePolicy::PersistAll,
                interaction: RunInteractionPolicy::FailIfHitlRequired,
                projection: RunProjectionPolicy::ObserveProjectionBarrier,
                publish: RunPublishPolicy::NoPublish,
            },
            SeoExecutionMode::SemiAutoOperator => {
                SeoRunPolicy::for_semi_auto_operator(false, false, false)
            }
            SeoExecutionMode::ManualTest => SeoRunPolicy::for_manual_test(),
        },
        SEO_RUN_MODE_DRY_RUN => match execution_mode {
            SeoExecutionMode::TemporalDurable => SeoRunPolicy {
                persistence: RunPersistencePolicy::PersistWithoutPublish,
                interaction: RunInteractionPolicy::FailIfHitlRequired,
                projection: RunProjectionPolicy::WarnOnly,
                publish: RunPublishPolicy::NoPublish,
            },
            SeoExecutionMode::SemiAutoOperator => {
                SeoRunPolicy::for_semi_auto_operator(false, false, true)
            }
            SeoExecutionMode::ManualTest => SeoRunPolicy::for_manual_test(),
        },
        SEO_RUN_MODE_DRAFT_ONLY => match execution_mode {
            SeoExecutionMode::TemporalDurable => SeoRunPolicy {
                persistence: RunPersistencePolicy::PersistWithoutPublish,
                interaction: RunInteractionPolicy::FailIfHitlRequired,
                projection: RunProjectionPolicy::ObserveProjectionBarrier,
                publish: RunPublishPolicy::NoPublish,
            },
            SeoExecutionMode::SemiAutoOperator => {
                SeoRunPolicy::for_semi_auto_operator(false, false, false)
            }
            SeoExecutionMode::ManualTest => SeoRunPolicy::for_manual_test(),
        },
        SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL => match execution_mode {
            SeoExecutionMode::TemporalDurable => SeoRunPolicy {
                persistence: RunPersistencePolicy::PersistAll,
                interaction: RunInteractionPolicy::RequirePreApprovedDecision,
                projection: RunProjectionPolicy::ObserveProjectionBarrier,
                publish: RunPublishPolicy::PublishIfApproved,
            },
            SeoExecutionMode::SemiAutoOperator => {
                SeoRunPolicy::for_semi_auto_operator(true, true, false)
            }
            SeoExecutionMode::ManualTest => SeoRunPolicy::for_manual_test(),
        },
        SEO_RUN_MODE_PUBLISH_WITH_HITL => match execution_mode {
            SeoExecutionMode::TemporalDurable => SeoRunPolicy::for_temporal_durable(),
            SeoExecutionMode::SemiAutoOperator => {
                SeoRunPolicy::for_semi_auto_operator(true, false, false)
            }
            SeoExecutionMode::ManualTest => SeoRunPolicy::for_manual_test(),
        },
        _ => SeoRunPolicy::for_temporal_durable(),
    }
}

pub fn run_mode_for_scenario(
    scenario: SeoScenarioKind,
    publish: bool,
    require_preapproved_decision: bool,
    warn_only_projections: bool,
) -> &'static str {
    match scenario {
        SeoScenarioKind::CrawlIngestOnly => SEO_RUN_MODE_CRAWL_ONLY,
        SeoScenarioKind::Full | SeoScenarioKind::PublishOnly => {
            if publish && require_preapproved_decision {
                SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL
            } else if publish {
                SEO_RUN_MODE_PUBLISH_WITH_HITL
            } else if warn_only_projections {
                SEO_RUN_MODE_DRY_RUN
            } else {
                SEO_RUN_MODE_DRAFT_ONLY
            }
        }
        SeoScenarioKind::PlanningOnly | SeoScenarioKind::DraftingOnly => SEO_RUN_MODE_DRAFT_ONLY,
        SeoScenarioKind::RebuildOnly => SEO_RUN_MODE_PUBLISH_WITH_HITL,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeoPhaseKey {
    LoadVerifiedSupportBundle,
    SerpIngest,
    CrawlSources,
    RawKnowledgeIngestion,
    RefreshVerifiedSupportBundle,
    SerpNormalize,
    OpportunityBuild,
    IaBuild,
    LinkRecommend,
    GlobalSiteReconcile,
    DraftAssemble,
    EditorialDraftGenerate,
    DraftNormalize,
    ContentContractValidate,
    DraftQa,
    CmsRequestReview,
    HumanApprovalWait,
    CmsPublishApproved,
    PublishMaterialize,
    RenderPreviewValidate,
    FinalizePublish,
    RebuildDetect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeoExecutionSegment {
    Initial,
    Planning,
    PerPage,
    Finalize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeoPhaseInput {
    pub key: SeoPhaseKey,
    pub page_index: Option<usize>,
    pub page_total: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeoPhaseOutput {
    pub key: SeoPhaseKey,
    pub status: String,
    pub page_node_key: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeoPhaseDecision {
    Run(SeoPhaseInput),
    Complete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeoExecutionPlan {
    pub scenario: SeoScenarioKind,
    pub mode: SeoExecutionMode,
    pub policy: SeoRunPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SeoExecutionCursor {
    pub segment: Option<SeoExecutionSegment>,
    pub phase_index: usize,
    pub page_index: usize,
    pub page_total: usize,
}

pub fn build_execution_plan(request: &SeoScenarioRequest) -> SeoExecutionPlan {
    SeoExecutionPlan {
        scenario: request.scenario,
        mode: request.mode,
        policy: request.policy,
    }
}

pub fn phase_label(key: SeoPhaseKey) -> &'static str {
    match key {
        SeoPhaseKey::LoadVerifiedSupportBundle => "load_verified_support_bundle",
        SeoPhaseKey::SerpIngest => "serp_ingest",
        SeoPhaseKey::CrawlSources => "crawl_sources",
        SeoPhaseKey::RawKnowledgeIngestion => "raw_knowledge_ingestion",
        SeoPhaseKey::RefreshVerifiedSupportBundle => "load_verified_support_bundle.refresh",
        SeoPhaseKey::SerpNormalize => "serp_normalize",
        SeoPhaseKey::OpportunityBuild => "opportunity_build",
        SeoPhaseKey::IaBuild => "ia_build",
        SeoPhaseKey::LinkRecommend => "link_recommend",
        SeoPhaseKey::GlobalSiteReconcile => "global_site_reconcile",
        SeoPhaseKey::DraftAssemble => "draft_assemble",
        SeoPhaseKey::EditorialDraftGenerate => "editorial_draft_generate",
        SeoPhaseKey::DraftNormalize => "draft_normalize",
        SeoPhaseKey::ContentContractValidate => "content_contract_validate",
        SeoPhaseKey::DraftQa => "draft_qa",
        SeoPhaseKey::CmsRequestReview => "cms_request_review",
        SeoPhaseKey::HumanApprovalWait => "human_approval_wait",
        SeoPhaseKey::CmsPublishApproved => "cms_publish_approved",
        SeoPhaseKey::PublishMaterialize => "publish_materialize",
        SeoPhaseKey::RenderPreviewValidate => "render_preview_validate",
        SeoPhaseKey::FinalizePublish => "finalize_publish",
        SeoPhaseKey::RebuildDetect => "rebuild_detect",
    }
}

pub fn blocked_interaction_status(policy: RunInteractionPolicy) -> &'static str {
    match policy {
        RunInteractionPolicy::AllowHitlPause => "hitl_required",
        RunInteractionPolicy::FailIfHitlRequired => "blocked_hitl_required",
        RunInteractionPolicy::RequirePreApprovedDecision => "blocked_preapproval_required",
    }
}

pub fn blocked_projection_status() -> &'static str {
    "blocked_projection_barrier"
}

pub fn blocked_publish_gate_status() -> &'static str {
    "blocked_publish_gate"
}

pub fn initial_phase_keys() -> &'static [SeoPhaseKey] {
    &[
        SeoPhaseKey::LoadVerifiedSupportBundle,
        SeoPhaseKey::SerpIngest,
        SeoPhaseKey::CrawlSources,
        SeoPhaseKey::RawKnowledgeIngestion,
    ]
}

pub fn planning_phase_keys(plan: &SeoExecutionPlan, refresh_support: bool) -> Vec<SeoPhaseKey> {
    let mut phases = Vec::new();
    if matches!(plan.scenario, SeoScenarioKind::CrawlIngestOnly) {
        return phases;
    }
    if refresh_support {
        phases.push(SeoPhaseKey::RefreshVerifiedSupportBundle);
    }
    phases.extend_from_slice(&[
        SeoPhaseKey::SerpNormalize,
        SeoPhaseKey::OpportunityBuild,
        SeoPhaseKey::IaBuild,
        SeoPhaseKey::LinkRecommend,
        SeoPhaseKey::GlobalSiteReconcile,
    ]);
    phases
}

pub fn should_publish(plan: &SeoExecutionPlan) -> bool {
    if plan.policy.persistence == RunPersistencePolicy::PersistWithoutPublish {
        return false;
    }
    matches!(plan.policy.publish, RunPublishPolicy::PublishIfApproved)
        && matches!(
            plan.scenario,
            SeoScenarioKind::Full | SeoScenarioKind::PublishOnly
        )
}

pub fn page_phase_keys(plan: &SeoExecutionPlan) -> Vec<SeoPhaseKey> {
    let mut phases = vec![
        SeoPhaseKey::DraftAssemble,
        SeoPhaseKey::EditorialDraftGenerate,
        SeoPhaseKey::DraftNormalize,
        SeoPhaseKey::ContentContractValidate,
        SeoPhaseKey::DraftQa,
    ];
    if should_publish(plan) {
        phases.extend_from_slice(&[
            SeoPhaseKey::CmsRequestReview,
            SeoPhaseKey::HumanApprovalWait,
            SeoPhaseKey::CmsPublishApproved,
            SeoPhaseKey::PublishMaterialize,
            SeoPhaseKey::RenderPreviewValidate,
            SeoPhaseKey::FinalizePublish,
        ]);
    }
    phases
}

pub fn final_phase_keys(plan: &SeoExecutionPlan) -> &'static [SeoPhaseKey] {
    if matches!(
        plan.scenario,
        SeoScenarioKind::Full | SeoScenarioKind::RebuildOnly
    ) {
        &[SeoPhaseKey::RebuildDetect]
    } else {
        &[]
    }
}

pub fn next_phase(phases: &[SeoPhaseKey], cursor: &SeoExecutionCursor) -> SeoPhaseDecision {
    if cursor.phase_index >= phases.len() {
        return SeoPhaseDecision::Complete("done".to_string());
    }
    SeoPhaseDecision::Run(SeoPhaseInput {
        key: phases[cursor.phase_index],
        page_index: if matches!(cursor.segment, Some(SeoExecutionSegment::PerPage)) {
            Some(cursor.page_index)
        } else {
            None
        },
        page_total: if matches!(cursor.segment, Some(SeoExecutionSegment::PerPage)) {
            Some(cursor.page_total)
        } else {
            None
        },
    })
}

pub fn advance_cursor(cursor: &mut SeoExecutionCursor) {
    cursor.phase_index += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_only_phase_order_is_shared_across_modes() {
        let plan_temporal = SeoExecutionPlan {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::TemporalDurable,
            policy: SeoRunPolicy::for_temporal_durable(),
        };
        let plan_manual = SeoExecutionPlan {
            scenario: SeoScenarioKind::PlanningOnly,
            mode: SeoExecutionMode::ManualTest,
            policy: SeoRunPolicy::for_manual_test(),
        };
        assert_eq!(initial_phase_keys(), initial_phase_keys());
        assert_eq!(
            planning_phase_keys(&plan_temporal, true),
            planning_phase_keys(&plan_manual, true)
        );
    }

    #[test]
    fn persist_without_publish_removes_publish_phases() {
        let plan = SeoExecutionPlan {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::SemiAutoOperator,
            policy: SeoRunPolicy {
                persistence: RunPersistencePolicy::PersistWithoutPublish,
                interaction: RunInteractionPolicy::FailIfHitlRequired,
                projection: RunProjectionPolicy::ObserveProjectionBarrier,
                publish: RunPublishPolicy::PublishIfApproved,
            },
        };
        assert!(!page_phase_keys(&plan).contains(&SeoPhaseKey::CmsRequestReview));
    }

    #[test]
    fn unknown_run_mode_defaults_to_publish_with_hitl() {
        assert_eq!(
            normalize_run_mode("unknown-mode"),
            SEO_RUN_MODE_PUBLISH_WITH_HITL
        );
        assert_eq!(
            scenario_kind_for_run_mode("unknown-mode"),
            SeoScenarioKind::Full
        );
    }

    #[test]
    fn crawl_only_mode_maps_to_non_publish_plan() {
        let policy =
            policy_for_run_mode(SEO_RUN_MODE_CRAWL_ONLY, SeoExecutionMode::TemporalDurable);
        assert_eq!(
            scenario_kind_for_run_mode(SEO_RUN_MODE_CRAWL_ONLY),
            SeoScenarioKind::CrawlIngestOnly
        );
        assert_eq!(policy.publish, RunPublishPolicy::NoPublish);
    }
}
