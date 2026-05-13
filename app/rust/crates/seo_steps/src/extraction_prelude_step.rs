use serde::{Deserialize, Serialize};

use crate::dom_block_relevance_step::{
    execute as run_dom_filter, DomBlockInput, DomBlockRelevanceOutput,
};
use crate::page_utility_classifier_step::{
    execute as run_page_utility, PageUtilityClassifierInput, PageUtilityClassifierOutput,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionPreludeInput {
    pub page: PageUtilityClassifierInput,
    pub dom_blocks: Vec<DomBlockInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionPreludeOutput {
    pub page_utility: PageUtilityClassifierOutput,
    pub dom_relevance: DomBlockRelevanceOutput,
}

pub fn execute(input: &ExtractionPreludeInput) -> ExtractionPreludeOutput {
    let page_utility = run_page_utility(&input.page);
    let mut dom_relevance = run_dom_filter(&input.dom_blocks);

    if !page_utility.allow_structural_extraction {
        for block in &mut dom_relevance.blocks {
            block.allow_extraction = false;
        }
    }

    ExtractionPreludeOutput {
        page_utility,
        dom_relevance,
    }
}
