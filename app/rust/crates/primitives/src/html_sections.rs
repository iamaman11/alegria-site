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
    ("fees",           &["стоимость", "сборы", "цена", "fee", "консульск", "оплата"]),
    ("documents",      &["документы", "список", "перечень", "справки", "анкета"]),
    ("timelines",      &["сроки", "срок", "processing", "рассмотрен"]),
    ("where_to_apply", &["куда подавать", "где подавать", "адрес", "центр", "посольство"]),
    ("appointment",    &["запись", "appointment", "записаться", "талон"]),
    ("eligibility",    &["требования", "условия", "кто может", "право на"]),
    ("steps",          &["шаги", "этапы", "процесс", "как получить", "порядок"]),
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

    let title = Selector::parse("title").ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    let meta_desc = Selector::parse("meta[name='description']").ok()
        .and_then(|sel| document.select(&sel).next())
        .and_then(|el| el.value().attr("content"))
        .unwrap_or("")
        .to_string();

    let canonical = Selector::parse("link[rel='canonical']").ok()
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

    sections
}
