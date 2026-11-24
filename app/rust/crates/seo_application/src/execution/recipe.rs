//! Typed data dependencies for composing site-build steps.
//!
//! A step's identity is distinct from its position in the current site-build
//! recipe. Independent steps can change order, while a missing prerequisite is
//! rejected before an executor performs any side effect. These contracts
//! describe data produced by the current executor, not remote service owners.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::{
    final_phase_keys, initial_phase_keys, page_phase_keys, phase_label, planning_phase_keys,
    should_publish, SeoExecutionPlan, SeoExecutionSegment, SeoPhaseKey,
};
use crate::scenario::SeoScenarioKind;

/// The meaningful input/output boundary of a functional step, independent of
/// how the current website-building workflow orders those steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeoStepArtifact {
    VerifiedSupport,
    QueryBatch,
    CrawledPages,
    RawKnowledge,
    SerpPatterns,
    Opportunities,
    InformationArchitecture,
    LinkRecommendations,
    ReconciledSite,
    DraftPlan,
    EditorialCandidate,
    NormalizedDraft,
    ContentValidated,
    QualityChecked,
    ReviewRequested,
    HumanApproved,
    ApprovedRevision,
    MaterializedArtifact,
    RenderValidated,
    PublishedRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeoStepContract {
    pub segment: SeoExecutionSegment,
    pub consumes: &'static [SeoStepArtifact],
    pub produces: &'static [SeoStepArtifact],
}

impl SeoPhaseKey {
    pub fn from_stable_id(id: &str) -> Option<Self> {
        Some(match id {
            "load_verified_support_bundle" => Self::LoadVerifiedSupportBundle,
            "serp_ingest" => Self::SerpIngest,
            "crawl_sources" => Self::CrawlSources,
            "raw_knowledge_ingestion" => Self::RawKnowledgeIngestion,
            "load_verified_support_bundle.refresh" => Self::RefreshVerifiedSupportBundle,
            "serp_normalize" => Self::SerpNormalize,
            "opportunity_build" => Self::OpportunityBuild,
            "ia_build" => Self::IaBuild,
            "link_recommend" => Self::LinkRecommend,
            "global_site_reconcile" => Self::GlobalSiteReconcile,
            "draft_assemble" => Self::DraftAssemble,
            "editorial_draft_generate" => Self::EditorialDraftGenerate,
            "draft_normalize" => Self::DraftNormalize,
            "content_contract_validate" => Self::ContentContractValidate,
            "draft_qa" => Self::DraftQa,
            "cms_request_review" => Self::CmsRequestReview,
            "human_approval_wait" => Self::HumanApprovalWait,
            "cms_publish_approved" => Self::CmsPublishApproved,
            "publish_materialize" => Self::PublishMaterialize,
            "render_preview_validate" => Self::RenderPreviewValidate,
            "finalize_publish" => Self::FinalizePublish,
            "rebuild_detect" => Self::RebuildDetect,
            _ => return None,
        })
    }

    /// Declare input/output ownership once, independently of any particular
    /// scenario, mode, or executor's hard-coded control flow.
    pub fn contract(self) -> SeoStepContract {
        use SeoExecutionSegment::{Finalize, Initial, PerPage, Planning};
        use SeoStepArtifact::*;
        let (segment, consumes, produces): (_, &'static [_], &'static [_]) = match self {
            Self::LoadVerifiedSupportBundle => (Initial, &[], &[VerifiedSupport]),
            Self::SerpIngest => (Initial, &[], &[QueryBatch]),
            Self::CrawlSources => (Initial, &[QueryBatch], &[CrawledPages]),
            Self::RawKnowledgeIngestion => (Initial, &[QueryBatch, CrawledPages], &[RawKnowledge]),
            Self::RefreshVerifiedSupportBundle => (Planning, &[RawKnowledge], &[VerifiedSupport]),
            Self::SerpNormalize => (Planning, &[QueryBatch], &[SerpPatterns]),
            Self::OpportunityBuild => (Planning, &[SerpPatterns], &[Opportunities]),
            Self::IaBuild => (
                Planning,
                &[Opportunities, VerifiedSupport],
                &[InformationArchitecture],
            ),
            Self::LinkRecommend => (Planning, &[InformationArchitecture], &[LinkRecommendations]),
            Self::GlobalSiteReconcile => (
                Planning,
                &[
                    InformationArchitecture,
                    LinkRecommendations,
                    VerifiedSupport,
                ],
                &[ReconciledSite],
            ),
            Self::DraftAssemble => (
                PerPage,
                &[ReconciledSite, InformationArchitecture, VerifiedSupport],
                &[DraftPlan],
            ),
            Self::EditorialDraftGenerate => (PerPage, &[DraftPlan], &[EditorialCandidate]),
            Self::DraftNormalize => (
                PerPage,
                &[DraftPlan, EditorialCandidate, VerifiedSupport],
                &[NormalizedDraft],
            ),
            Self::ContentContractValidate => (PerPage, &[NormalizedDraft], &[ContentValidated]),
            Self::DraftQa => (
                PerPage,
                &[NormalizedDraft, VerifiedSupport],
                &[QualityChecked],
            ),
            Self::CmsRequestReview => (
                PerPage,
                &[NormalizedDraft, ContentValidated, QualityChecked],
                &[ReviewRequested],
            ),
            Self::HumanApprovalWait => (PerPage, &[ReviewRequested], &[HumanApproved]),
            Self::CmsPublishApproved => (
                PerPage,
                &[ReviewRequested, HumanApproved, NormalizedDraft],
                &[ApprovedRevision],
            ),
            Self::PublishMaterialize => (
                PerPage,
                &[ReviewRequested, ApprovedRevision],
                &[MaterializedArtifact],
            ),
            Self::RenderPreviewValidate => (PerPage, &[MaterializedArtifact], &[RenderValidated]),
            Self::FinalizePublish => (
                PerPage,
                &[
                    ReviewRequested,
                    ApprovedRevision,
                    MaterializedArtifact,
                    RenderValidated,
                ],
                &[PublishedRevision],
            ),
            Self::RebuildDetect => (Finalize, &[RawKnowledge, ReconciledSite], &[]),
        };
        SeoStepContract {
            segment,
            consumes,
            produces,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeoStepRecipeError {
    WrongSegment {
        step: SeoPhaseKey,
        expected: SeoExecutionSegment,
        actual: SeoExecutionSegment,
    },
    DuplicateStep(SeoPhaseKey),
    MissingRequiredStep(SeoPhaseKey),
    ForbiddenByPolicy(SeoPhaseKey),
    UnreachableForScenario {
        scenario: SeoScenarioKind,
        step: SeoPhaseKey,
    },
    MissingInput {
        step: SeoPhaseKey,
        input: SeoStepArtifact,
    },
    UnsupportedManifestVersion(u16),
    UnknownManifestStep(String),
}

impl fmt::Display for SeoStepRecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSegment {
                step,
                expected,
                actual,
            } => write!(
                f,
                "step {step:?} requires {expected:?} segment, received {actual:?}"
            ),
            Self::DuplicateStep(step) => write!(f, "step {step:?} is repeated"),
            Self::MissingRequiredStep(step) => {
                write!(f, "scenario requires step {step:?}")
            }
            Self::ForbiddenByPolicy(step) => {
                write!(f, "policy does not permit step {step:?}")
            }
            Self::UnreachableForScenario { scenario, step } => {
                write!(f, "scenario {scenario:?} cannot execute step {step:?}")
            }
            Self::MissingInput { step, input } => {
                write!(f, "step {step:?} requires unavailable {input:?}")
            }
            Self::UnsupportedManifestVersion(version) => {
                write!(
                    f,
                    "unsupported SEO recipe manifest schema version {version}"
                )
            }
            Self::UnknownManifestStep(step) => write!(f, "unknown SEO recipe step {step}"),
        }
    }
}

impl std::error::Error for SeoStepRecipeError {}

/// Versioned, serializable step order. The manifest is intended to be pinned
/// to the workflow's immutable input rather than regenerated on replay.
pub const SEO_RECIPE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeoRecipeManifest {
    pub schema_version: u16,
    pub pre_page: Vec<String>,
    pub per_page: Vec<String>,
    pub finalize: Vec<String>,
}

/// A recipe has one ordered pre-page sequence, one repeated-per-page sequence
/// and optional finalizers. `Initial` and `Planning` classify step ownership,
/// but do not force the pre-page order or require adjacent planning steps.
/// The per-page boundary is an actual execution-scoping constraint because it
/// creates one page-local state for each admitted page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeoPhaseRecipe {
    pre_page: Vec<SeoPhaseKey>,
    initial: Vec<SeoPhaseKey>,
    planning: Vec<SeoPhaseKey>,
    per_page: Vec<SeoPhaseKey>,
    finalize: Vec<SeoPhaseKey>,
}

impl SeoPhaseRecipe {
    /// Compose an ordered sequence of typed functional steps. Each prerequisite
    /// must be produced earlier; independent steps may be reordered.
    pub fn compose(
        initial: Vec<SeoPhaseKey>,
        planning: Vec<SeoPhaseKey>,
        per_page: Vec<SeoPhaseKey>,
        finalize: Vec<SeoPhaseKey>,
    ) -> Result<Self, SeoStepRecipeError> {
        for (segment, steps) in [
            (SeoExecutionSegment::Initial, initial.as_slice()),
            (SeoExecutionSegment::Planning, planning.as_slice()),
            (SeoExecutionSegment::PerPage, per_page.as_slice()),
            (SeoExecutionSegment::Finalize, finalize.as_slice()),
        ] {
            for &step in steps {
                let expected = step.contract().segment;
                if segment != expected {
                    return Err(SeoStepRecipeError::WrongSegment {
                        step,
                        expected,
                        actual: segment,
                    });
                }
            }
        }
        Self::compose_pre_page(
            initial.into_iter().chain(planning).collect(),
            per_page,
            finalize,
        )
    }

    /// Compose a dependency-checked sequence that can interleave any initial
    /// and planning functions. Only the page-local and finalization scoping
    /// boundaries are structural, as they use different state and inputs.
    pub fn compose_pre_page(
        pre_page: Vec<SeoPhaseKey>,
        per_page: Vec<SeoPhaseKey>,
        finalize: Vec<SeoPhaseKey>,
    ) -> Result<Self, SeoStepRecipeError> {
        let initial = pre_page
            .iter()
            .copied()
            .filter(|step| step.contract().segment == SeoExecutionSegment::Initial)
            .collect();
        let planning = pre_page
            .iter()
            .copied()
            .filter(|step| step.contract().segment == SeoExecutionSegment::Planning)
            .collect();
        let recipe = Self {
            pre_page,
            initial,
            planning,
            per_page,
            finalize,
        };
        recipe.validate()?;
        Ok(recipe)
    }

    /// The current canonical sequence is data passed through the same
    /// validation as any future composable recipe.
    pub fn for_plan(
        plan: &SeoExecutionPlan,
        refresh_support: bool,
    ) -> Result<Self, SeoStepRecipeError> {
        Self::compose(
            initial_phase_keys().to_vec(),
            planning_phase_keys(plan, refresh_support),
            if matches!(
                plan.scenario,
                SeoScenarioKind::CrawlIngestOnly | SeoScenarioKind::PlanningOnly
            ) {
                Vec::new()
            } else {
                page_phase_keys(plan)
            },
            final_phase_keys(plan).to_vec(),
        )
    }

    pub fn steps(&self, segment: SeoExecutionSegment) -> &[SeoPhaseKey] {
        match segment {
            SeoExecutionSegment::Initial => &self.initial,
            SeoExecutionSegment::Planning => &self.planning,
            SeoExecutionSegment::PerPage => &self.per_page,
            SeoExecutionSegment::Finalize => &self.finalize,
        }
    }

    pub fn pre_page_steps(&self) -> &[SeoPhaseKey] {
        &self.pre_page
    }

    /// Stable phase identities are explicit labels, not Rust's `Debug` output.
    /// Consumers must reject unknown IDs rather than skip an unrecognized step.
    pub fn manifest(&self) -> SeoRecipeManifest {
        SeoRecipeManifest {
            schema_version: SEO_RECIPE_SCHEMA_VERSION,
            pre_page: self
                .pre_page
                .iter()
                .copied()
                .map(phase_label)
                .map(str::to_string)
                .collect(),
            per_page: self
                .per_page
                .iter()
                .copied()
                .map(phase_label)
                .map(str::to_string)
                .collect(),
            finalize: self
                .finalize
                .iter()
                .copied()
                .map(phase_label)
                .map(str::to_string)
                .collect(),
        }
    }

    pub fn from_manifest(manifest: &SeoRecipeManifest) -> Result<Self, SeoStepRecipeError> {
        if manifest.schema_version != SEO_RECIPE_SCHEMA_VERSION {
            return Err(SeoStepRecipeError::UnsupportedManifestVersion(
                manifest.schema_version,
            ));
        }
        fn parse(ids: &[String]) -> Result<Vec<SeoPhaseKey>, SeoStepRecipeError> {
            ids.iter()
                .map(|id| {
                    SeoPhaseKey::from_stable_id(id)
                        .ok_or_else(|| SeoStepRecipeError::UnknownManifestStep(id.clone()))
                })
                .collect()
        }
        Self::compose_pre_page(
            parse(&manifest.pre_page)?,
            parse(&manifest.per_page)?,
            parse(&manifest.finalize)?,
        )
    }

    /// Validate the entire graph *before* a step runs. A backwards edge, a
    /// missing producer, or an attempted cycle cannot pass this check.
    pub fn validate(&self) -> Result<(), SeoStepRecipeError> {
        self.validate_with_contracts(SeoPhaseKey::contract)
    }

    fn validate_with_contracts(
        &self,
        contract_for: impl Fn(SeoPhaseKey) -> SeoStepContract,
    ) -> Result<(), SeoStepRecipeError> {
        use SeoExecutionSegment::{Finalize, Initial, PerPage, Planning};
        let mut available = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for (segment, steps) in [
            (Initial, self.pre_page.as_slice()),
            (PerPage, self.per_page.as_slice()),
            (Finalize, self.finalize.as_slice()),
        ] {
            for &step in steps {
                let contract = contract_for(step);
                if contract.segment != segment
                    && !(segment == Initial && contract.segment == Planning)
                {
                    return Err(SeoStepRecipeError::WrongSegment {
                        step,
                        expected: contract.segment,
                        actual: segment,
                    });
                }
                if !visited.insert(step) {
                    return Err(SeoStepRecipeError::DuplicateStep(step));
                }
                for &input in contract.consumes {
                    if !available.contains(&input) {
                        return Err(SeoStepRecipeError::MissingInput { step, input });
                    }
                }
                available.extend(contract.produces.iter().copied());
            }
        }
        Ok(())
    }

    /// The scenario executor currently needs these two intermediate products
    /// to produce its run summary and per-page work. Publication is never
    /// enabled by a custom recipe when the run policy forbids it.
    pub fn validate_for_scenario(&self, plan: &SeoExecutionPlan) -> Result<(), SeoStepRecipeError> {
        self.validate()?;
        use SeoPhaseKey::*;
        // These scenarios return from the executor early. Never accept a
        // recipe whose validated steps would be silently skipped at runtime.
        let unreachable_segments: &[SeoExecutionSegment] = match plan.scenario {
            SeoScenarioKind::CrawlIngestOnly => &[
                SeoExecutionSegment::Planning,
                SeoExecutionSegment::PerPage,
                SeoExecutionSegment::Finalize,
            ],
            SeoScenarioKind::PlanningOnly => {
                &[SeoExecutionSegment::PerPage, SeoExecutionSegment::Finalize]
            }
            _ => &[],
        };
        for &segment in unreachable_segments {
            if let Some(&step) = self.steps(segment).first() {
                return Err(SeoStepRecipeError::UnreachableForScenario {
                    scenario: plan.scenario,
                    step,
                });
            }
        }
        if !self.initial.contains(&RawKnowledgeIngestion) {
            return Err(SeoStepRecipeError::MissingRequiredStep(
                RawKnowledgeIngestion,
            ));
        }
        if !matches!(plan.scenario, SeoScenarioKind::CrawlIngestOnly)
            && !self.planning.contains(&GlobalSiteReconcile)
        {
            return Err(SeoStepRecipeError::MissingRequiredStep(GlobalSiteReconcile));
        }
        if matches!(
            plan.scenario,
            SeoScenarioKind::Full
                | SeoScenarioKind::DraftingOnly
                | SeoScenarioKind::PublishOnly
                | SeoScenarioKind::RebuildOnly
        ) && !self.per_page.contains(&DraftNormalize)
        {
            return Err(SeoStepRecipeError::MissingRequiredStep(DraftNormalize));
        }
        if should_publish(plan) && !self.per_page.contains(&FinalizePublish) {
            // A publish-enabled scenario must not silently report success
            // when a caller removed every publishing operation.
            return Err(SeoStepRecipeError::MissingRequiredStep(FinalizePublish));
        }
        if matches!(plan.scenario, SeoScenarioKind::RebuildOnly)
            && !self.finalize.contains(&RebuildDetect)
        {
            return Err(SeoStepRecipeError::MissingRequiredStep(RebuildDetect));
        }
        if !should_publish(plan) {
            for step in [
                CmsRequestReview,
                HumanApprovalWait,
                CmsPublishApproved,
                PublishMaterialize,
                RenderPreviewValidate,
                FinalizePublish,
            ] {
                if self.per_page.contains(&step) {
                    return Err(SeoStepRecipeError::ForbiddenByPolicy(step));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::SeoRunPolicy;
    use crate::scenario::{SeoExecutionMode, SeoScenarioKind};

    fn full_plan() -> SeoExecutionPlan {
        SeoExecutionPlan {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::TemporalDurable,
            policy: SeoRunPolicy::for_temporal_durable(),
        }
    }

    #[test]
    fn canonical_scenario_is_a_valid_typed_recipe() {
        let recipe = SeoPhaseRecipe::for_plan(&full_plan(), true).unwrap();
        assert!(recipe
            .steps(SeoExecutionSegment::Initial)
            .contains(&SeoPhaseKey::CrawlSources));
        assert!(recipe
            .steps(SeoExecutionSegment::PerPage)
            .contains(&SeoPhaseKey::FinalizePublish));
    }

    #[test]
    fn initial_step_order_can_change_when_inputs_stay_available() {
        use SeoPhaseKey::*;
        let recipe = SeoPhaseRecipe::for_plan(&full_plan(), false).unwrap();
        let changed = SeoPhaseRecipe::compose(
            vec![
                SerpIngest,
                LoadVerifiedSupportBundle,
                CrawlSources,
                RawKnowledgeIngestion,
            ],
            recipe.planning.clone(),
            recipe.per_page.clone(),
            recipe.finalize.clone(),
        )
        .unwrap();
        assert_eq!(changed.steps(SeoExecutionSegment::Initial)[0], SerpIngest);
    }

    #[test]
    fn independent_page_gates_can_change_order_without_bypassing_either() {
        use SeoPhaseKey::*;
        let recipe = SeoPhaseRecipe::for_plan(&full_plan(), false).unwrap();
        let mut page = recipe.per_page.clone();
        let validation = page
            .iter()
            .position(|step| *step == ContentContractValidate)
            .unwrap();
        let qa = page.iter().position(|step| *step == DraftQa).unwrap();
        page.swap(validation, qa);
        let changed =
            SeoPhaseRecipe::compose(recipe.initial, recipe.planning, page, recipe.finalize)
                .unwrap();
        assert_eq!(
            changed.steps(SeoExecutionSegment::PerPage)[validation],
            DraftQa
        );
        assert!(changed
            .steps(SeoExecutionSegment::PerPage)
            .contains(&CmsRequestReview));
    }

    #[test]
    fn planning_can_interleave_initial_steps_when_data_contracts_allow_it() {
        use SeoPhaseKey::*;
        let standard = SeoPhaseRecipe::for_plan(&full_plan(), true).unwrap();
        let interleaved = SeoPhaseRecipe::compose_pre_page(
            vec![
                SerpIngest,
                SerpNormalize,
                OpportunityBuild,
                LoadVerifiedSupportBundle,
                CrawlSources,
                RawKnowledgeIngestion,
                RefreshVerifiedSupportBundle,
                IaBuild,
                LinkRecommend,
                GlobalSiteReconcile,
            ],
            standard.per_page.clone(),
            standard.finalize.clone(),
        )
        .unwrap();
        assert_eq!(interleaved.pre_page_steps()[1], SeoPhaseKey::SerpNormalize);
        assert_eq!(
            interleaved.steps(SeoExecutionSegment::Planning)[0],
            SeoPhaseKey::SerpNormalize
        );
        assert_ne!(standard.manifest(), interleaved.manifest());
    }

    #[test]
    fn recipe_manifest_serializes_and_rejects_drift() {
        let standard = SeoPhaseRecipe::for_plan(&full_plan(), true).unwrap();
        let manifest = standard.manifest();
        let encoded = serde_json::to_string(&manifest).unwrap();
        assert!(encoded.contains("\"schema_version\":1"));
        assert_eq!(SeoPhaseRecipe::from_manifest(&manifest).unwrap(), standard);

        let mut future = manifest.clone();
        future.schema_version += 1;
        assert!(matches!(
            SeoPhaseRecipe::from_manifest(&future),
            Err(SeoStepRecipeError::UnsupportedManifestVersion(_))
        ));
        let mut unknown = manifest.clone();
        unknown.pre_page[0] = "unknown_future_step".to_string();
        assert_eq!(
            SeoPhaseRecipe::from_manifest(&unknown),
            Err(SeoStepRecipeError::UnknownManifestStep(
                "unknown_future_step".to_string()
            ))
        );
        let mut backwards = manifest;
        backwards.pre_page.swap(1, 2);
        assert!(matches!(
            SeoPhaseRecipe::from_manifest(&backwards),
            Err(SeoStepRecipeError::MissingInput { .. })
        ));
    }

    #[test]
    fn publishing_and_drafting_require_explicit_terminal_steps() {
        use SeoPhaseKey::*;
        let standard = SeoPhaseRecipe::for_plan(&full_plan(), true).unwrap();
        let no_page = SeoPhaseRecipe::compose_pre_page(
            standard.pre_page.clone(),
            vec![],
            standard.finalize.clone(),
        )
        .unwrap();
        assert_eq!(
            no_page.validate_for_scenario(&full_plan()),
            Err(SeoStepRecipeError::MissingRequiredStep(DraftNormalize))
        );
        let no_publish = SeoPhaseRecipe::compose_pre_page(
            standard.pre_page.clone(),
            vec![DraftAssemble, EditorialDraftGenerate, DraftNormalize],
            standard.finalize.clone(),
        )
        .unwrap();
        assert_eq!(
            no_publish.validate_for_scenario(&full_plan()),
            Err(SeoStepRecipeError::MissingRequiredStep(FinalizePublish))
        );
    }

    #[test]
    fn invalid_order_and_missing_input_are_rejected_before_execution() {
        use SeoPhaseKey::*;
        let bad = SeoPhaseRecipe::compose(
            vec![
                LoadVerifiedSupportBundle,
                CrawlSources,
                SerpIngest,
                RawKnowledgeIngestion,
            ],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(
            bad,
            Err(SeoStepRecipeError::MissingInput {
                step: CrawlSources,
                input: SeoStepArtifact::QueryBatch,
            })
        );
        let missing_gate =
            SeoPhaseRecipe::compose(vec![SerpIngest], vec![], vec![FinalizePublish], vec![]);
        assert!(matches!(
            missing_gate,
            Err(SeoStepRecipeError::MissingInput { .. })
        ));
    }

    #[test]
    fn duplicate_and_wrong_segment_are_rejected() {
        use SeoPhaseKey::*;
        assert!(matches!(
            SeoPhaseRecipe::compose(vec![SerpIngest, SerpIngest], vec![], vec![], vec![]),
            Err(SeoStepRecipeError::DuplicateStep(SerpIngest))
        ));
        assert!(matches!(
            SeoPhaseRecipe::compose(vec![SerpNormalize], vec![], vec![], vec![]),
            Err(SeoStepRecipeError::WrongSegment { .. })
        ));
    }

    #[test]
    fn mutually_dependent_steps_cannot_be_composed_in_either_order() {
        use SeoExecutionSegment::Initial;
        use SeoPhaseKey::*;
        use SeoStepArtifact::*;
        // Mimic a future catalog change that accidentally makes two steps
        // depend on each other. There is no possible executable ordering.
        for order in [
            [LoadVerifiedSupportBundle, SerpIngest],
            [SerpIngest, LoadVerifiedSupportBundle],
        ] {
            let recipe = SeoPhaseRecipe {
                pre_page: order.to_vec(),
                initial: order.to_vec(),
                planning: Vec::new(),
                per_page: Vec::new(),
                finalize: Vec::new(),
            };
            let error = recipe.validate_with_contracts(|step| match step {
                LoadVerifiedSupportBundle => SeoStepContract {
                    segment: Initial,
                    consumes: &[QueryBatch],
                    produces: &[VerifiedSupport],
                },
                SerpIngest => SeoStepContract {
                    segment: Initial,
                    consumes: &[VerifiedSupport],
                    produces: &[QueryBatch],
                },
                _ => step.contract(),
            });
            assert!(matches!(
                error,
                Err(SeoStepRecipeError::MissingInput { .. })
            ));
        }
    }
}
