//! Source-page and DOM relevance gates, before any Truth or editorial decisions.
//!
//! Acquisition owns the input classification. Temporal activities orchestrate
//! these pure policies and retain their existing wire/activity names.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageMode {
    ContentPage,
    MenuPage,
    DirectoryPage,
    LandingPage,
    UtilityPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilityClassifierInput {
    pub url: String,
    pub title: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilityClassifierOutput {
    pub page_mode: PageMode,
    pub allow_procedural_extraction: bool,
    pub allow_editorial_extraction: bool,
    pub allow_structural_extraction: bool,
}

pub fn classify_page(input: &PageUtilityClassifierInput) -> PageUtilityClassifierOutput {
    let text = format!("{}\n{}\n{}", input.url, input.title, input.raw_text).to_lowercase();
    let page_mode = if text.contains("sitemap")
        || text.contains("all countries")
        || text.contains("каталог")
        || text.contains("directory")
    {
        PageMode::DirectoryPage
    } else if text.contains("menu") || text.contains("навигац") || text.contains("breadcrumb")
    {
        PageMode::MenuPage
    } else if text.contains("login")
        || text.contains("signin")
        || text.contains("privacy")
        || text.contains("cookie")
        || text.contains("utility")
    {
        PageMode::UtilityPage
    } else if text.contains("landing")
        || text.contains("hero")
        || text.contains("book now")
        || text.contains("consultation")
    {
        PageMode::LandingPage
    } else {
        PageMode::ContentPage
    };

    let (allow_procedural_extraction, allow_editorial_extraction, allow_structural_extraction) =
        match page_mode {
            PageMode::ContentPage => (true, true, true),
            PageMode::LandingPage => (false, true, true),
            PageMode::MenuPage | PageMode::DirectoryPage | PageMode::UtilityPage => {
                (false, false, false)
            }
        };

    PageUtilityClassifierOutput {
        page_mode,
        allow_procedural_extraction,
        allow_editorial_extraction,
        allow_structural_extraction,
    }
}

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

/// Use identical section-role inference in the real activity and expert trace.
pub fn block_role_for_section(section_type: &str, heading: &str) -> BlockRole {
    let section_type = section_type.to_lowercase();
    let heading = heading.to_lowercase();
    if section_type.contains("nav") || heading.contains("breadcrumb") {
        BlockRole::Navigation
    } else if section_type.contains("toc") {
        BlockRole::Toc
    } else if section_type.contains("footer") {
        BlockRole::Footer
    } else {
        BlockRole::ContentMain
    }
}

pub fn score_dom_blocks(blocks: &[DomBlockInput]) -> DomBlockRelevanceOutput {
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

pub fn extraction_prelude(input: &ExtractionPreludeInput) -> ExtractionPreludeOutput {
    let page_utility = classify_page(&input.page);
    let mut dom_relevance = score_dom_blocks(&input.dom_blocks);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn page(raw_text: &str) -> PageUtilityClassifierInput {
        PageUtilityClassifierInput {
            url: "https://example.org/source".to_string(),
            title: "Travel requirements".to_string(),
            raw_text: raw_text.to_string(),
        }
    }

    fn block(role: BlockRole, text: &str) -> DomBlockInput {
        DomBlockInput {
            dom_block_id: "body-1".to_string(),
            block_role: role,
            text: text.to_string(),
        }
    }

    #[test]
    fn source_page_gates_match_acquisition_rules() {
        let content = classify_page(&page("Visa fee EUR 25"));
        assert_eq!(content.page_mode, PageMode::ContentPage);
        assert!(content.allow_procedural_extraction);
        assert!(content.allow_editorial_extraction);
        assert!(content.allow_structural_extraction);

        let landing = classify_page(&page("Hero consultation"));
        assert_eq!(landing.page_mode, PageMode::LandingPage);
        assert!(!landing.allow_procedural_extraction);
        assert!(landing.allow_editorial_extraction);
        assert!(landing.allow_structural_extraction);

        let utility = classify_page(&page("Cookie settings"));
        assert_eq!(utility.page_mode, PageMode::UtilityPage);
        assert!(!utility.allow_structural_extraction);
        assert!(!utility.allow_editorial_extraction);
    }

    #[test]
    fn structural_source_role_cannot_be_bypassed_by_keywords() {
        let blocks = score_dom_blocks(&[
            block(BlockRole::ContentMain, "Visa application fee EUR 25"),
            block(BlockRole::Navigation, "Visa fee EUR 25"),
        ]);
        assert_eq!(blocks.blocks.len(), 2);
        assert!(blocks.blocks[0].allow_extraction);
        assert!(!blocks.blocks[1].allow_extraction);
        assert!(
            blocks.blocks[0].content_relevance_score > blocks.blocks[1].content_relevance_score
        );
    }

    #[test]
    fn page_gate_blocks_dom_extraction_even_for_main_content() {
        let result = extraction_prelude(&ExtractionPreludeInput {
            page: page("Privacy and cookies"),
            dom_blocks: vec![block(BlockRole::ContentMain, "Visa fee EUR 25")],
        });
        assert!(!result.page_utility.allow_structural_extraction);
        assert!(!result.dom_relevance.blocks[0].allow_extraction);
    }

    #[test]
    fn section_role_and_json_discriminators_remain_stable() {
        assert_eq!(
            block_role_for_section("NAVIGATION", "Visa"),
            BlockRole::Navigation
        );
        assert_eq!(
            block_role_for_section("text", "Breadcrumbs"),
            BlockRole::Navigation
        );
        assert_eq!(block_role_for_section("TOC", "Visa"), BlockRole::Toc);
        assert_eq!(block_role_for_section("footer", "Visa"), BlockRole::Footer);
        assert_eq!(
            block_role_for_section("content", "Visa"),
            BlockRole::ContentMain
        );
        assert_eq!(
            serde_json::to_string(&PageMode::DirectoryPage).unwrap(),
            "\"directory_page\""
        );
        assert_eq!(
            serde_json::to_string(&BlockRole::ContentMain).unwrap(),
            "\"content_main\""
        );
    }
}
