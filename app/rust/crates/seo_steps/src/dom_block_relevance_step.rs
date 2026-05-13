use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockRole {
    ContentMain,
    Navigation,
    Footer,
    Header,
    Sidebar,
    RelatedLinks,
    Toc,
    Breadcrumbs,
    Promo,
    Form,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockInput {
    pub dom_block_id: String,
    pub block_role: BlockRole,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockDecision {
    pub dom_block_id: String,
    pub block_role: BlockRole,
    pub content_relevance_score: f32,
    pub allow_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceOutput {
    pub blocks: Vec<DomBlockDecision>,
}

pub fn execute(blocks: &[DomBlockInput]) -> DomBlockRelevanceOutput {
    let mut decisions = Vec::with_capacity(blocks.len());
    for block in blocks {
        let text = block.text.to_lowercase();
        let mut score: f32 = match block.block_role {
            BlockRole::ContentMain => 0.85,
            BlockRole::RelatedLinks => 0.2,
            BlockRole::Toc | BlockRole::Breadcrumbs => 0.1,
            BlockRole::Navigation
            | BlockRole::Footer
            | BlockRole::Header
            | BlockRole::Sidebar
            | BlockRole::Promo
            | BlockRole::Form => 0.05,
        };

        if text.contains("visa") || text.contains("виза") || text.contains("документ") {
            score += 0.1;
        }
        if text.contains("fee") || text.contains("eur") || text.contains("€") {
            score += 0.05;
        }
        score = score.clamp(0.0, 1.0);

        let allow_extraction = matches!(block.block_role, BlockRole::ContentMain) && score >= 0.5;
        decisions.push(DomBlockDecision {
            dom_block_id: block.dom_block_id.clone(),
            block_role: block.block_role.clone(),
            content_relevance_score: score,
            allow_extraction,
        });
    }
    DomBlockRelevanceOutput { blocks: decisions }
}
