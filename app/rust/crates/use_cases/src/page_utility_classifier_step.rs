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

pub fn execute(input: &PageUtilityClassifierInput) -> PageUtilityClassifierOutput {
    let text = format!("{}\n{}\n{}", input.url, input.title, input.raw_text).to_lowercase();
    let page_mode = if text.contains("sitemap")
        || text.contains("all countries")
        || text.contains("каталог")
        || text.contains("directory")
    {
        PageMode::DirectoryPage
    } else if text.contains("menu") || text.contains("навигац") || text.contains("breadcrumb") {
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
