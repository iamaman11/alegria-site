//! Typed data dependencies for composing site-build steps.
//!
//! A step's identity is distinct from its position in the current site-build
//! recipe. Independent steps can change order, while a missing prerequisite is
//! rejected before an executor performs any side effect. These contracts
//! describe data produced by the current executor, not remote service owners.

use std::collections::BTreeSet;
use std::fmt;

use super::{
    final_phase_keys, initial_phase_keys, page_phase_keys, planning_phase_keys, should_publish,
    SeoExecutionPlan, SeoExecutionSegment, SeoPhaseKey,
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
        }
    }
}

impl std::error::Error for SeoStepRecipeError {}

/// One fully ordered recipe, with explicit boundaries where per-page state is
/// created. The recipe is validated before execution; it does not imply that
/// the existing Temporal workflow supports changing an in-flight history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeoPhaseRecipe {
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
        let recipe = Self {
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
        for segment in [Initial, Planning, PerPage, Finalize] {
            for &step in self.steps(segment) {
                let contract = contract_for(step);
                if contract.segment != segment {
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
