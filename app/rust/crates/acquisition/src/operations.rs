//! Pure acquisition decisions, registered by their own domain owner.
//!
//! These operations are *not* crawler transports, Temporal Activities, SQL
//! journals or permission to fetch remote pages. They expose exact typed
//! inputs and outputs for a future acquisition-owner one-step adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, ContractCatalog, IdempotencyPolicy, Operation, OperationContract, OperationId,
    OperationOwnerId, OperationRef, PlanError, PortId,
};
use serde::{Deserialize, Serialize};

use crate::source_relevance::{
    self, BlockRole, DomBlockInput, DomBlockRelevanceOutput, PageUtilityClassifierInput,
    PageUtilityClassifierOutput,
};

/// Versioned, strict external wire boundary. The existing V1/Temporal DTOs in
/// `source_relevance` intentionally remain permissive for historical replay.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassifySourcePageInput {
    pub url: String,
    pub title: String,
    pub raw_text: String,
}

impl From<&ClassifySourcePageInput> for PageUtilityClassifierInput {
    fn from(input: &ClassifySourcePageInput) -> Self {
        Self {
            url: input.url.clone(),
            title: input.title.clone(),
            raw_text: input.raw_text.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomBlockRelevanceInput {
    pub blocks: Vec<StrictDomBlockInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrictDomBlockInput {
    pub dom_block_id: String,
    pub block_role: BlockRole,
    pub text: String,
}

impl From<&StrictDomBlockInput> for DomBlockInput {
    fn from(input: &StrictDomBlockInput) -> Self {
        Self {
            dom_block_id: input.dom_block_id.clone(),
            block_role: input.block_role.clone(),
            text: input.text.clone(),
        }
    }
}

/// Existing pure decision functions stay in acquisition. An owner adapter can
/// register them without granting acquisition network or persistence access.
pub trait PureAcquisitionOperation: Operation<Failure = Infallible> {
    fn evaluate(input: &Self::Input) -> Self::Output;
}

fn pure_contract(id: &str, input_type: &str, output_type: &str) -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId::from(id),
            revision: 1,
        },
        owner: OperationOwnerId::from("acquisition"),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            ArtifactSchema {
                id: input_type.into(),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            ArtifactSchema {
                id: output_type.into(),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::new(),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
    }
}

/// ACQ-01: one explicit page classification, independent of source download.
pub struct ClassifySourcePage;

impl Operation for ClassifySourcePage {
    type Input = ClassifySourcePageInput;
    type Output = PageUtilityClassifierOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "acquisition.classify_source_page",
            "acquisition.page_utility_classifier_input",
            "acquisition.page_utility_classifier_output",
        )
    }
}

impl PureAcquisitionOperation for ClassifySourcePage {
    fn evaluate(input: &Self::Input) -> Self::Output {
        source_relevance::classify_page(&input.into())
    }
}

/// ACQ-02: independent *raw* block relevance with the same scoring rule used
/// by the crawler. This is not an extraction authorization. The legacy
/// extraction_prelude applies the ACQ-01 page-level structural-extraction
/// gate *after* block scoring; a composed DAG must enforce the same gate
/// before consuming any positive block score for extraction.
pub struct EvaluateDomBlocks;

impl Operation for EvaluateDomBlocks {
    type Input = DomBlockRelevanceInput;
    type Output = DomBlockRelevanceOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "acquisition.evaluate_dom_blocks",
            "acquisition.dom_block_relevance_input",
            "acquisition.dom_block_relevance_output",
        )
    }
}

impl PureAcquisitionOperation for EvaluateDomBlocks {
    fn evaluate(input: &Self::Input) -> Self::Output {
        source_relevance::score_dom_blocks(
            &input
                .blocks
                .iter()
                .map(DomBlockInput::from)
                .collect::<Vec<_>>(),
        )
    }
}

/// An owner-authored list of exact pure contracts, not a runtime admission
/// grant. The independent worker must still be installed and verified before
/// these can be offered as runnable siteArchitect operations.
pub fn acquisition_pure_contracts() -> Result<ContractCatalog, PlanError> {
    let mut catalog = ContractCatalog::default();
    catalog.register::<ClassifySourcePage>()?;
    catalog.register::<EvaluateDomBlocks>()?;
    Ok(catalog)
}

// JSON encoding/decoding tests belong to a named wire-boundary module, not
// this pure owner implementation (enforced by check_json_boundary_policy).
#[cfg(test)]
#[path = "operations_json.rs"]
mod tests;
