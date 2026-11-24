//! Immutable, typed operations admitted by the first durable Temporal V2 runner.
//!
//! This is deliberately a small functional catalog, not a second listing of
//! Temporal Activity names. A bundle may contain several retryable Activities;
//! those operational details belong to the durable executor. V1's longer
//! command history and the unrelated non-Temporal `SeoPhaseRecipe` are not
//! silently interpreted as this plan.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SEO_DURABLE_PLAN_SCHEMA_VERSION: u16 = 1;

/// Current V2 support is intentionally explicit. A future scenario needs a
/// new implementation and a new compatibility decision before it is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeoDurableScenario {
    SourceAcquisitionOnly,
}

/// Stable wire identifiers for independent functional bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeoDurableStep {
    LoadVerifiedSupport,
    SourceAcquisition,
}

/// Produced/required data, independent of the current position of a bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeoDurableArtifact {
    SiteInput,
    VerifiedSupport,
    CrawledEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeoDurableStepContract {
    pub consumes: &'static [SeoDurableArtifact],
    pub produces: &'static [SeoDurableArtifact],
}

impl SeoDurableStep {
    pub fn contract(self) -> SeoDurableStepContract {
        use SeoDurableArtifact::*;
        match self {
            Self::LoadVerifiedSupport => SeoDurableStepContract {
                consumes: &[SiteInput],
                produces: &[VerifiedSupport],
            },
            Self::SourceAcquisition => SeoDurableStepContract {
                consumes: &[SiteInput],
                produces: &[CrawledEvidence],
            },
        }
    }
}

/// Serialized in V2 workflow-start input and therefore recorded in its
/// durable history. Unknown scenarios/steps are denied by Serde, and the
/// explicit schema version is validated before any workflow Activity runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeoDurablePlan {
    pub schema_version: u16,
    pub scenario: SeoDurableScenario,
    pub steps: Vec<SeoDurableStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeoDurablePlanError {
    UnsupportedVersion(u16),
    DuplicateStep(SeoDurableStep),
    MissingInput {
        step: SeoDurableStep,
        artifact: SeoDurableArtifact,
    },
    MissingRequiredStep(SeoDurableStep),
}

impl fmt::Display for SeoDurablePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported durable SEO plan schema version {version}")
            }
            Self::DuplicateStep(step) => write!(f, "durable SEO step {step:?} is repeated"),
            Self::MissingInput { step, artifact } => {
                write!(
                    f,
                    "durable SEO step {step:?} requires unavailable {artifact:?}"
                )
            }
            Self::MissingRequiredStep(step) => {
                write!(f, "durable SEO scenario requires {step:?}")
            }
        }
    }
}

impl std::error::Error for SeoDurablePlanError {}

impl SeoDurablePlan {
    pub fn source_acquisition_only(
        steps: Vec<SeoDurableStep>,
    ) -> Result<Self, SeoDurablePlanError> {
        let plan = Self {
            schema_version: SEO_DURABLE_PLAN_SCHEMA_VERSION,
            scenario: SeoDurableScenario::SourceAcquisitionOnly,
            steps,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn ordered_steps(&self) -> &[SeoDurableStep] {
        &self.steps
    }

    /// Bootstrap always loads SiteInput and runs preflight. A recipe can
    /// reorder independent bundles or omit optional support, but it cannot
    /// finish a source-acquisition scenario without producing CrawledEvidence.
    /// This does not include the separate RawKnowledgeIngestion operation.
    pub fn validate(&self) -> Result<(), SeoDurablePlanError> {
        if self.schema_version != SEO_DURABLE_PLAN_SCHEMA_VERSION {
            return Err(SeoDurablePlanError::UnsupportedVersion(self.schema_version));
        }

        use SeoDurableArtifact::*;
        let mut available = BTreeSet::from([SiteInput]);
        let mut visited = BTreeSet::new();
        for &step in &self.steps {
            if !visited.insert(step) {
                return Err(SeoDurablePlanError::DuplicateStep(step));
            }
            let contract = step.contract();
            for &artifact in contract.consumes {
                if !available.contains(&artifact) {
                    return Err(SeoDurablePlanError::MissingInput { step, artifact });
                }
            }
            available.extend(contract.produces.iter().copied());
        }
        if !available.contains(&CrawledEvidence) {
            return Err(SeoDurablePlanError::MissingRequiredStep(
                SeoDurableStep::SourceAcquisition,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_real_bundle_orders_are_valid_and_keep_distinct_wire_identities() {
        use SeoDurableStep::*;
        let a_steps = vec![LoadVerifiedSupport, SourceAcquisition];
        let b_steps = vec![SourceAcquisition, LoadVerifiedSupport];
        let a = SeoDurablePlan::source_acquisition_only(a_steps).unwrap();
        let b = SeoDurablePlan::source_acquisition_only(b_steps).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.ordered_steps(), &[LoadVerifiedSupport, SourceAcquisition]);
        assert_eq!(b.ordered_steps(), &[SourceAcquisition, LoadVerifiedSupport]);
        let serialized = serde_json::to_string(&b).unwrap();
        assert!(serialized.contains("\"source_acquisition\""));
        assert!(serialized.contains("\"schema_version\":1"));
    }

    #[test]
    fn valid_minimal_subset_still_produces_source_acquisition_terminal_artifact() {
        let steps = vec![SeoDurableStep::SourceAcquisition];
        let plan = SeoDurablePlan::source_acquisition_only(steps).unwrap();
        assert_eq!(plan.ordered_steps(), &[SeoDurableStep::SourceAcquisition]);
    }

    #[test]
    fn invalid_versions_missing_terminal_step_and_duplicates_fail_before_io() {
        use SeoDurableStep::*;
        assert_eq!(
            SeoDurablePlan::source_acquisition_only(vec![LoadVerifiedSupport]),
            Err(SeoDurablePlanError::MissingRequiredStep(SourceAcquisition))
        );
        assert_eq!(
            SeoDurablePlan::source_acquisition_only(vec![SourceAcquisition, SourceAcquisition]),
            Err(SeoDurablePlanError::DuplicateStep(SourceAcquisition))
        );
        let mut future = SeoDurablePlan::source_acquisition_only(vec![SourceAcquisition]).unwrap();
        future.schema_version += 1;
        assert_eq!(
            future.validate(),
            Err(SeoDurablePlanError::UnsupportedVersion(2))
        );
    }
}
