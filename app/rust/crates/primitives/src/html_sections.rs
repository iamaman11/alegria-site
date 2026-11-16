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

/// Headings in real official pages are often nested in a title/header
/// wrapper, while the substantive article is the NEXT sibling of that
/// wrapper. The heading's own sibling text then contains only a subtitle.
/// Lift one level only for an explicit header wrapper: never scan an entire
/// body, main, navigation sidebar, or neighboring article for legal evidence.
fn collect_wrapped_heading_body(document: &Html, heading_id: ego_tree::NodeId) -> String {
    let Some(heading) = document.tree.get(heading_id) else {
        return String::new();
    };
    let Some(wrapper) = heading.parent() else {
        return String::new();
    };
    let Node::Element(element) = wrapper.value() else {
        return String::new();
    };
    let header_class = element.attr("class").unwrap_or("").to_ascii_lowercase();
    // EC Drupal/ECL articles wrap their H2 in a plain div.ecl and put
    // substantive paragraphs in the next div.ecl. A dated H1 elsewhere
    // on the page can already create a tiny section, so the whole-document
    // empty-sections fallback never fires. Only lift these ECL headings when
    // they belong to a real article; never lift navigation or sidebar text.
    let mut ancestor = wrapper.parent();
    let mut inside_article = false;
    while let Some(parent) = ancestor {
        if matches!(parent.value(), Node::Element(el) if el.name() == "article") {
            inside_article = true;
            break;
        }
        ancestor = parent.parent();
    }
    let ecl_article_heading = inside_article
        && element.name() == "div"
        && header_class.split_whitespace().any(|class| class == "ecl")
        && matches!(
            heading.value(),
            Node::Element(el)
                if el
                    .attr("class")
                    .unwrap_or("")
                    .split_whitespace()
                    .any(|class| class.starts_with("ecl-u-type-heading-"))
        );
    let is_header_wrapper = element.name() == "header"
        || header_class.split_whitespace().any(|class| {
            class.contains("header") || class.contains("heading") || class.ends_with("title")
        })
        || ecl_article_heading;
    if !is_header_wrapper {
        return String::new();
    }

    fn has_heading(node: NodeRef<Node>) -> bool {
        if let Node::Element(element) = node.value() {
            if is_heading(element.name()) {
                return true;
            }
        }
        node.children().any(has_heading)
    }

    let mut parts = Vec::new();
    let mut next = wrapper.next_sibling();
    while let Some(sibling) = next {
        // Never attach the following heading's content to this heading,
        // including when another article is nested in its own DIV.
        if has_heading(sibling) {
            break;
        }
        collect_text_dfs(sibling, &mut parts);
        next = sibling.next_sibling();
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

/// Recover a substantive, visibly titled article whose template uses a
/// styled paragraph instead of an H1-H4. A neighboring sidebar heading must
/// not make that article disappear. Stay within the article body: no page-wide
/// text scan, navigation, footer or unrelated sidebar evidence.
fn append_titled_headingless_articles(document: &Html, sections: &mut Vec<HtmlSection>) {
    let wrapper_selector = Selector::parse(".single__detail-Wrapper").unwrap();
    let title_selector = Selector::parse(".section__header-title").unwrap();
    let body_selector = Selector::parse(".single__text").unwrap();
    let heading_selector = Selector::parse("h1, h2, h3, h4").unwrap();

    for wrapper in document.select(&wrapper_selector) {
        let Some(body) = wrapper.select(&body_selector).next() else {
            continue;
        };
        // Ordinary heading-based extraction owns articles with actual headings.
        if body.select(&heading_selector).next().is_some() {
            continue;
        }
        let Some(title) = wrapper.select(&title_selector).next() else {
            continue;
        };
        let heading = title.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if heading.is_empty() {
            continue;
        }
        let Some(body_node) = document.tree.get(body.id()) else {
            continue;
        };
        let mut parts = Vec::new();
        collect_text_dfs(body_node, &mut parts);
        let content = parts.join("\n").trim().to_string();
        if content.is_empty()
            || sections.iter().any(|section| section.content_md.contains(&content))
        {
            continue;
        }
        let hash = content_hash_v1(&content);
        sections.push(HtmlSection {
            heading_path: heading.clone(),
            heading_level: heading_level(title.value().name()),
            section_order: sections.len(),
            section_type: identify_section_type(&heading).to_string(),
            content_md: content,
            content_hash: hash,
        });
    }
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
            let mut content = collect_siblings_text(&document, *node_id);
            let wrapped_body = collect_wrapped_heading_body(&document, *node_id);
            if !wrapped_body.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&wrapped_body);
            }
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

    // Also recover semantically scoped, headingless article bodies even when
    // a sidebar H2 already produced an unrelated section.
    append_titled_headingless_articles(&document, &mut sections);

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
        assert_eq!(
            sections[0].content_hash,
            content_hash_v1(&sections[0].content_md)
        );
    }

    #[test]
    fn wrapped_eu_faq_heading_retains_real_article_evidence() {
        // Based on the actual EEAS Drupal card structure: the heading lives
        // inside text-box-header and its factual paragraphs are in a sibling
        // card-text, not in the h2's next sibling. Navigation stays separate.
        let html = r#"<html><body>
          <nav><h3>On This Page</h3><ul><li>Visa</li><li>Information</li></ul></nav>
          <main>
            <div class="card"><div class="card-body">
              <div class="text-box-header">
                <h2 class="card-title">Information for EU citizens</h2>
                <div class="h5 card-subtitle">Travelling to Belarus</div>
              </div>
              <div class="card-text"><p>EU citizens travelling to Belarus.</p></div>
            </div></div>
            <div class="card"><div class="card-body">
              <div class="text-box-header">
                <h2 class="card-title">Information for Schengen Visa Applicants in Belarus</h2>
                <div class="h5 card-subtitle">Answers to the frequently asked questions</div>
              </div>
              <div class="card-text"><div class="field__item">
                <p>Spain: A Schengen visa application for Spain may be submitted at the Spanish Visa Center in Minsk.</p>
                <script>window.fake_fee = "1000 EUR"</script>
              </div></div>
            </div></div>
          </main>
          <footer>Unrelated telephone numbers</footer>
        </body></html>"#;
        let sections = extract_sections_typed(html, "https://www.eeas.europa.eu/belarus/");
        let schengen = sections
            .iter()
            .find(|section| {
                section.heading_path == "Information for Schengen Visa Applicants in Belarus"
            })
            .expect("Schengen FAQ heading exists");
        assert!(schengen
            .content_md
            .contains("Answers to the frequently asked questions"));
        assert!(schengen.content_md.contains("Spanish Visa Center in Minsk"));
        assert!(!schengen
            .content_md
            .contains("EU citizens travelling to Belarus"));
        assert!(!schengen.content_md.contains("window.fake_fee"));
        assert!(!schengen.content_md.contains("Unrelated telephone numbers"));
        assert_eq!(schengen.content_hash, content_hash_v1(&schengen.content_md));
    }

    #[test]
    fn european_commission_ecl_article_heading_recovers_exact_policy_body() {
        // Physical EC page: article H2 is in a plain div.ecl, factual text
        // in the next div.ecl, whereas the page H1 already yields a date.
        // Preserve independent heading boundaries, not navigation evidence.
        let html = r#"<html><body>
          <nav><h2 class="ecl-u-type-heading-2">Page contents</h2>
            <p>Menu entry, not legal evidence.</p></nav>
          <main><h1>Applying for a Schengen visa</h1>
            <div>2 December 2025</div>
            <article dir="ltr">
              <div class="ecl-u-mb-2xl">
                <div class="ecl"><h2 class="ecl-u-type-heading-2">What is a Schengen visa?</h2></div>
                <div class="ecl"><p>A <strong>Schengen visa</strong> is an entry permit
                  for non-EU nationals to make a short visit of up to
                  <strong>90 days in any 180-day period</strong>.</p></div>
              </div>
              <div class="ecl-u-mb-2xl">
                <div class="ecl"><h2 class="ecl-u-type-heading-2">How long can you stay?</h2></div>
                <div class="ecl"><p>The short-stay calculator can help
                  calculate the remaining period.</p></div>
              </div>
            </article>
          </main><footer>Unrelated marketing footer</footer>
        </body></html>"#;
        let sections = extract_sections_typed(
            html,
            "https://home-affairs.ec.europa.eu/policies/schengen/visa-policy/applying-schengen-visa_en",
        );
        let schengen = sections
            .iter()
            .find(|section| section.heading_path == "What is a Schengen visa?")
            .expect("ECL policy evidence must not disappear behind a date-only H1");
        assert!(schengen
            .content_md
            .contains("90 days in any 180-day period"));
        assert!(!schengen.content_md.contains("short-stay calculator"));
        assert!(!schengen.content_md.contains("Menu entry"));
        assert!(!schengen.content_md.contains("marketing footer"));
        assert_eq!(schengen.content_hash, content_hash_v1(&schengen.content_md));
        let calculator = sections
            .iter()
            .find(|section| section.heading_path == "How long can you stay?")
            .expect("second ECL heading retains its own section");
        assert!(calculator.content_md.contains("short-stay calculator"));
        assert!(!calculator
            .content_md
            .contains("90 days in any 180-day period"));
    }

    #[test]
    fn titled_article_without_h_tags_is_not_lost_to_sidebar_heading() {
        // The official consular page uses a styled paragraph title and
        // .single__text for its substantive body, with an H2 in the sidebar.
        let html = r#"<html><body>
            <nav><h2>Navigation</h2><p>Navigation only</p></nav>
            <div class="row"><div class="col-12 col-md-8">
              <section class="body__detail-Wrapper">
                <div class="single__detail-Wrapper">
                  <h1 class="section__header-title hide">Consular services</h1>
                </div>
              </section>
              <section class="body__detail-Wrapper">
                <div class="single__detail-Wrapper">
                  <p class="section__header-title">Where to submit an application</p>
                  <div class="single__text">
                    <p>Visa application centre</p>
                    <p><strong>Minsk</strong></p>
                    <p>8 Tolstoi Street</p>
                    <p>Applications accepted on weekdays.</p>
                    <script>fake_evidence = "another location"</script>
                  </div>
                </div>
              </section>
            </div><div class="aside-right">
              <aside><h2>More information</h2><p>Unrelated prize news</p></aside>
            </div></div>
        </body></html>"#;
        let sections = extract_sections_typed(html, "https://example.org/consular");
        let article = sections
            .iter()
            .find(|section| section.heading_path == "Where to submit an application")
            .expect("visible article must survive an unrelated sidebar H2");
        assert!(article.content_md.contains("Minsk"));
        assert!(article.content_md.contains("8 Tolstoi Street"));
        assert!(!article.content_md.contains("Unrelated prize news"));
        assert!(!article.content_md.contains("Navigation only"));
        assert!(!article.content_md.contains("fake_evidence"));
        assert_eq!(article.content_hash, content_hash_v1(&article.content_md));
    }

    #[test]
    fn titled_article_with_actual_headings_is_not_duplicated() {
        let html = r#"<html><body><div class="single__detail-Wrapper">
            <p class="section__header-title">Visa application</p>
            <div class="single__text">
              <h2>Required documents</h2><p>Valid passport and travel insurance</p>
            </div>
        </div></body></html>"#;
        let sections = extract_sections_typed(html, "https://example.org/visa");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading_path, "Required documents");
        assert!(sections[0].content_md.contains("Valid passport"));
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
