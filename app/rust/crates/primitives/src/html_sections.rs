//! Deterministic HTML section extraction (R1).
//!
//! Splits an HTML page by H1–H4 headings into typed sections.
//! Each section gets a BLAKE3 content hash for idempotency.
//! Mirrors the semantics of `pipeline/crawl/extractor.py:PageExtractor.extract_sections()`.

use ego_tree::NodeRef;
use scraper::{node::Node, Html, Selector};
use serde::{Deserialize, Serialize};

use crate::hash::content_hash_v1;

// (section_type, keywords) — checked as case-insensitive substrings of heading text.
static SECTION_PATTERNS: &[(&str, &[&str])] = &[
    (
        "fees",
        &["стоимость", "сборы", "цена", "fee", "консульск", "оплата"],
    ),
    (
        "documents",
        &["документы", "список", "перечень", "справки", "анкета"],
    ),
    ("timelines", &["сроки", "срок", "processing", "рассмотрен"]),
    (
        "where_to_apply",
        &[
            "куда подавать",
            "где подавать",
            "адрес",
            "центр",
            "посольство",
        ],
    ),
    (
        "appointment",
        &["запись", "appointment", "записаться", "талон"],
    ),
    (
        "eligibility",
        &["требования", "условия", "кто может", "право на"],
    ),
    (
        "steps",
        &["шаги", "этапы", "процесс", "как получить", "порядок"],
    ),
];

fn identify_section_type(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    for (stype, keywords) in SECTION_PATTERNS {
        for kw in *keywords {
            if lower.contains(kw) {
                return stype;
            }
        }
    }
    "other"
}

fn heading_level(name: &str) -> usize {
    match name {
        "h1" => 1,
        "h2" => 2,
        "h3" => 3,
        "h4" => 4,
        _ => 0,
    }
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4")
}

/// Recursively collect all text leaves from a node subtree.
fn collect_text_dfs(node: NodeRef<Node>, parts: &mut Vec<String>) {
    // A crawled HTML snapshot retains every byte, but SEO knowledge must not
    // turn JavaScript, CSS or inert DOM templates into evidence or embeddings.
    if let Node::Element(element) = node.value() {
        if matches!(
            element.name(),
            "script" | "style" | "template" | "svg" | "iframe" | "canvas"
        ) {
            return;
        }
    }
    for child in node.children() {
        match child.value() {
            Node::Text(t) => {
                let s: &str = t;
                let s = s.trim();
                if !s.is_empty() {
                    parts.push(s.to_string());
                }
            }
            Node::Element(_) => {
                collect_text_dfs(child, parts);
            }
            _ => {}
        }
    }
}

/// Collect text from all siblings of `node_id` until the next heading (same parent level).
fn collect_siblings_text(document: &Html, node_id: ego_tree::NodeId) -> String {
    let node = match document.tree.get(node_id) {
        Some(n) => n,
        None => return String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    let mut cur = node.next_sibling();
    while let Some(sibling) = cur {
        match sibling.value() {
            Node::Element(el) => {
                if is_heading(el.name()) {
                    break;
                }
                collect_text_dfs(sibling, &mut parts);
            }
            Node::Text(t) => {
                let s: &str = t;
                let s = s.trim();
                if !s.is_empty() {
                    parts.push(s.to_string());
                }
            }
            _ => {}
        }
        cur = sibling.next_sibling();
    }
    parts.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HtmlMeta {
    pub title: String,
    pub meta_desc: String,
    pub canonical: String,
    pub word_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HtmlSection {
    pub heading_path: String,
    pub heading_level: usize,
    pub section_order: usize,
    pub section_type: String,
    pub content_md: String,
    pub content_hash: String,
}

/// Extract SEO metadata from an HTML document.
pub fn extract_meta_typed(html: &str) -> HtmlMeta {
    let document = Html::parse_document(html);

    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    let meta_desc = Selector::parse("meta[name='description']")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .and_then(|el| el.value().attr("content"))
        .unwrap_or("")
        .to_string();

    let canonical = Selector::parse("link[rel='canonical']")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .and_then(|el| el.value().attr("href"))
        .unwrap_or("")
        .to_string();

    let mut text_parts: Vec<String> = Vec::new();
    collect_text_dfs(document.tree.root(), &mut text_parts);
    let word_count = text_parts.join(" ").split_whitespace().count();

    HtmlMeta {
        title,
        meta_desc,
        canonical,
        word_count,
    }
}


/// Some official sites put the H1 in a title wrapper and the actual article
/// in the wrapper's *sibling*, not next to the H1 itself. If the heading
/// traversal yielded no usable sections, recover visible article text from a
/// semantic main-content container. Never fall back to all DOM text here:
/// navigation and footers are not legal evidence.
fn fallback_main_article_text(document: &Html) -> Option<String> {
    for css in [
        "main .single__text",
        "main article",
        "article",
        "[role='main'] .single__text",
        ".ms-rtestate-field",
        "main",
        "[role='main']",
    ] {
        let selector = Selector::parse(css).ok()?;
        for element in document.select(&selector) {
            let node = document.tree.get(element.id())?;
            let mut parts = Vec::new();
            collect_text_dfs(node, &mut parts);
            let text = parts.join("\n").trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Extract sections from HTML.
pub fn extract_sections_typed(html: &str, _url: &str) -> Vec<HtmlSection> {
    let document = Html::parse_document(html);
    let heading_sel = Selector::parse("h1, h2, h3, h4").unwrap();

    let headings: Vec<(String, String, ego_tree::NodeId)> = document
        .select(&heading_sel)
        .map(|el| {
            let tag = el.value().name().to_string();
            let text = el.text().collect::<String>().trim().to_string();
            let id = el.id();
            (tag, text, id)
        })
        .collect();

    let mut sections: Vec<HtmlSection> = Vec::new();

    if headings.is_empty() {
        // Fallback: whole page as one section.
        let mut parts: Vec<String> = Vec::new();
        collect_text_dfs(document.tree.root(), &mut parts);
        let text = parts.join("\n").trim().to_string();
        let hash = content_hash_v1(&text);
        sections.push(HtmlSection {
            heading_path: "root".to_string(),
            heading_level: 0,
            section_order: 0,
            section_type: "other".to_string(),
            content_md: text,
            content_hash: hash,
        });
    } else {
        for (i, (tag_name, heading_text, node_id)) in headings.iter().enumerate() {
            if heading_text.is_empty() {
                continue;
            }
            let content = collect_siblings_text(&document, *node_id);
            if content.is_empty() {
                continue;
            }
            let level = heading_level(tag_name);
            let stype = identify_section_type(heading_text);
            let hash = content_hash_v1(&content);
            sections.push(HtmlSection {
                heading_path: heading_text.clone(),
                heading_level: level,
                section_order: i,
                section_type: stype.to_string(),
                content_md: content,
                content_hash: hash,
            });
        }
    }

    // An H1 nested in a title wrapper can have no direct sibling text even
    // though a real article follows in main. Preserve its heading provenance
    // and only include the scoped visible article, never script/nav/footer.
    if sections.is_empty() && !headings.is_empty() {
        if let Some(text) = fallback_main_article_text(&document) {
            let (tag, heading, _) = headings
                .iter()
                .find(|(tag, heading, _)| tag == "h1" && !heading.is_empty())
                .or_else(|| headings.iter().find(|(_, heading, _)| !heading.is_empty()))
                .expect("nonempty headings or no scoped article");
            let hash = content_hash_v1(&text);
            sections.push(HtmlSection {
                heading_path: heading.clone(),
                heading_level: heading_level(tag),
                section_order: 0,
                section_type: identify_section_type(heading).to_string(),
                content_md: text,
                content_hash: hash,
            });
        }
    }

    sections
}

#[cfg(test)]
mod visible_text_tests {
    use super::*;

    #[test]
    fn skip_inert_dom_for_heading_sections_and_word_count() {
        let html = r#"<html><head>
            <title>Tourist visa guide</title>
            <script>window.WIZ_global_data = {"巨大":"do not embed"};</script>
            <style>body { display: none; } totally_inert_css</style>
            </head><body>
            <h1>Документы для визы</h1>
            <section><p>Паспорт и подтверждение проживания.</p>
              <script>secret_script_payload_123</script>
              <template>unrendered_template_payload_123</template>
            </section>
            </body></html>"#;
        let sections = extract_sections_typed(html, "https://example.org/visa");
        assert_eq!(sections.len(), 1);
        assert!(sections[0].content_md.contains("Паспорт и подтверждение"));
        assert!(!sections[0].content_md.contains("secret_script_payload"));
        assert!(!sections[0].content_md.contains("unrendered_template"));
        let meta = extract_meta_typed(html);
        assert_eq!(meta.title, "Tourist visa guide");
        assert!(meta.word_count > 0);
        assert!(
            meta.word_count < 30,
            "scripts/styles must not inflate word count"
        );
    }


    #[test]
    fn wrapped_official_heading_recovers_only_article_body() {
        // Real Spanish consular pages wrap the H1 in a distinct title DIV;
        // the next sibling of that wrapper contains the substantive content.
        let html = r#"<html lang="ru"><body>
          <nav><a>Главная Страница меню, не доказательство</a></nav>
          <main>
            <div class="section__header-title">
              <h1>Условия въезда в Испанию</h1>
            </div>
            <div class="single__text"><div>
              <p>Граждане третьих стран могут находиться в Испании до 90 дней.</p>
              <script>window.fake_rule = "600 дней"</script>
              <p>Для отдельных случаев действуют дополнительные условия.</p>
            </div></div>
          </main>
          <footer>Услуги и контакты</footer>
        </body></html>"#;
        let sections = extract_sections_typed(html, "https://www.exteriores.gob.es/");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading_path, "Условия въезда в Испанию");
        assert_eq!(sections[0].section_type, "eligibility");
        assert!(sections[0].content_md.contains("до 90 дней"));
        assert!(sections[0].content_md.contains("дополнительные условия"));
        assert!(!sections[0].content_md.contains("Главная Страница"));
        assert!(!sections[0].content_md.contains("Услуги и контакты"));
        assert!(!sections[0].content_md.contains("600 дней"));
        assert_eq!(sections[0].content_hash, content_hash_v1(&sections[0].content_md));
    }

    #[test]
    fn headingless_html_keeps_visible_content_without_javascript() {
        let html = r#"<html><body>
            <script>window.WIZ_global_data = "a very long invisible payload";</script>
            <style>.content { color: red }</style>
            <p>Для туристов из Беларуси полезно проверить актуальные правила.</p>
            <svg><text>svg_internal_string</text></svg>
            </body></html>"#;
        let parts = extract_sections_typed(html, "https://example.org/");
        assert_eq!(parts.len(), 1);
        assert!(parts[0].content_md.contains("туристов из Беларуси"));
        assert!(!parts[0].content_md.contains("WIZ_global_data"));
        assert!(!parts[0].content_md.contains("svg_internal_string"));
        assert_eq!(parts[0].content_hash, content_hash_v1(&parts[0].content_md));
    }
}
