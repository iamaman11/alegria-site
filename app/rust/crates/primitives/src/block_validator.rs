//! Quality gates for generated HTML blocks (typed core only).
//!
//! JSON boundary wrappers live in `block_validator_json.rs`.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

static NUMBER_RE: OnceLock<Regex> = OnceLock::new();
static PLACEHOLDER_RE: OnceLock<Regex> = OnceLock::new();

fn number_re() -> &'static Regex {
    NUMBER_RE.get_or_init(|| Regex::new(r"(?m)\b\d+(?:[.,]\d+)?\b").unwrap())
}

fn placeholder_re() -> &'static Regex {
    PLACEHOLDER_RE.get_or_init(|| Regex::new(r"\{\{[^}]+\}\}|\[\[[^\]]+\]\]").unwrap())
}

static FORBIDDEN_PHRASES: &[&str] = &[
    "в данной статье",
    "надеемся",
    "следует отметить",
    "не стоит забывать",
    "итак, как мы видим",
    "важно помнить",
];

static MIN_WORDS_BY_BLOCK: &[(&str, usize)] = &[
    ("intro", 100),
    ("documents_list", 80),
    ("fees_table", 50),
    ("faq", 150),
    ("steps", 120),
    ("eligibility", 80),
];

fn min_words(block_key: &str) -> usize {
    MIN_WORDS_BY_BLOCK
        .iter()
        .find(|(k, _)| *k == block_key)
        .map(|(_, v)| *v)
        .unwrap_or(0)
}

fn html_plain_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    doc.root_element()
        .text()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_selector(html: &str, css: &str) -> bool {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(css).unwrap();
    doc.select(&sel).next().is_some()
}

fn has_empty_list(html: &str) -> bool {
    let doc = Html::parse_document(html);
    let list_sel = Selector::parse("ul, ol").unwrap();
    let li_sel = Selector::parse("li").unwrap();
    for list in doc.select(&list_sel) {
        if list.select(&li_sel).next().is_none() {
            return true;
        }
    }
    false
}

fn extract_hrefs(html: &str) -> HashSet<String> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href]").unwrap();
    doc.select(&sel)
        .filter_map(|el| el.value().attr("href"))
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TypedValidationDiagnostic {
    pub severity: String,
    pub gate: String,
    pub block_key: String,
    pub message: String,
    #[serde(default)]
    pub context_json_utf8: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TypedValidationEnvelope {
    #[serde(default)]
    pub diagnostics: Vec<TypedValidationDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequiredLink {
    pub to_url: String,
    #[serde(default = "default_priority")]
    pub priority: i64,
}

fn default_priority() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlockValidationInput {
    #[serde(default)]
    pub allowed_numbers: HashSet<String>,
    #[serde(default)]
    pub required_links: Vec<RequiredLink>,
    #[serde(default)]
    pub required_keys: Vec<String>,
    #[serde(default)]
    pub used_rule_keys: Vec<String>,
    #[serde(default)]
    pub used_fact_keys: Vec<String>,
    #[serde(default)]
    pub block_key: String,
    #[serde(default)]
    pub url_norm: String,
}

fn diag(
    severity: &str,
    gate: &str,
    block_key: &str,
    message: impl Into<String>,
    context_json_utf8: Vec<u8>,
) -> TypedValidationDiagnostic {
    TypedValidationDiagnostic {
        severity: severity.to_string(),
        gate: gate.to_string(),
        block_key: block_key.to_string(),
        message: message.into(),
        context_json_utf8,
    }
}

pub fn validate_block_typed(html: &str, input: &BlockValidationInput) -> TypedValidationEnvelope {
    let mut diagnostics = Vec::new();
    let required: HashSet<String> = input
        .required_keys
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut used_all: HashSet<String> = input
        .used_rule_keys
        .iter()
        .chain(input.used_fact_keys.iter())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if required.is_empty() {
        diagnostics.push(diag(
            "info",
            "coverage",
            &input.block_key,
            "No required keys provided for coverage gate.",
            b"{}".to_vec(),
        ));
    } else {
        let mut missing: Vec<String> = required.difference(&used_all).cloned().collect();
        missing.sort();
        if !missing.is_empty() {
            let covered = required.len() - missing.len();
            let ratio = covered as f64 / required.len() as f64;
            diagnostics.push(diag(
                "critical",
                "coverage",
                &input.block_key,
                format!("Coverage missing {} required keys.", missing.len()),
                serde_json::to_vec(&missing).unwrap_or_else(|_| b"[]".to_vec()),
            ));
            diagnostics.push(diag(
                "warning",
                "coverage",
                &input.block_key,
                format!("Coverage ratio below 100%: {:.2}%", ratio * 100.0),
                format!(
                    r#"{{"required_total":{},"covered":{}}}"#,
                    required.len(),
                    covered
                )
                .into_bytes(),
            ));
        } else {
            diagnostics.push(diag(
                "info",
                "coverage",
                &input.block_key,
                "Coverage complete.",
                format!(r#"{{"required_total":{}}}"#, required.len()).into_bytes(),
            ));
        }
    }

    let html_trimmed = html.trim();
    if html_trimmed.is_empty() {
        diagnostics.push(diag(
            "critical",
            "no_hallucination",
            &input.block_key,
            "Generated HTML is empty.",
            b"{}".to_vec(),
        ));
    } else {
        let placeholders: Vec<String> = placeholder_re()
            .find_iter(html_trimmed)
            .take(20)
            .map(|m| m.as_str().to_string())
            .collect();
        if !placeholders.is_empty() {
            diagnostics.push(diag(
                "critical",
                "no_hallucination",
                &input.block_key,
                "Unresolved template placeholders detected.",
                serde_json::to_vec(&placeholders).unwrap_or_else(|_| b"[]".to_vec()),
            ));
        }

        let lowered = html_trimmed.to_lowercase();
        for marker in ["todo", "tbd", "lorem ipsum", "insert here"] {
            if lowered.contains(marker) {
                diagnostics.push(diag(
                    "critical",
                    "no_hallucination",
                    &input.block_key,
                    format!("Suspicious artifact marker found: {}", marker),
                    b"{}".to_vec(),
                ));
            }
        }

        let html_numbers: HashSet<String> = number_re()
            .find_iter(html_trimmed)
            .map(|m| m.as_str().replace(',', "."))
            .collect();
        let mut unknown: Vec<String> = html_numbers
            .difference(&input.allowed_numbers)
            .cloned()
            .collect();
        unknown.sort();

        if !unknown.is_empty() {
            let severity = if unknown.len() >= 3 {
                "critical"
            } else {
                "warning"
            };
            let sample: Vec<&String> = unknown.iter().take(30).collect();
            diagnostics.push(diag(
                severity,
                "no_hallucination",
                &input.block_key,
                format!(
                    "Found numeric claims outside provided rules/facts: {}",
                    unknown.len()
                ),
                serde_json::to_vec(&sample).unwrap_or_else(|_| b"[]".to_vec()),
            ));
        } else {
            diagnostics.push(diag(
                "info",
                "no_hallucination",
                &input.block_key,
                "No unsupported numeric claims detected.",
                format!(r#"{{"checked_numbers_count":{}}}"#, html_numbers.len()).into_bytes(),
            ));
        }
    }

    if input.block_key == "fees_table" && !has_selector(html, "table") {
        diagnostics.push(diag(
            "critical",
            "html_structure",
            &input.block_key,
            "Missing <table> element",
            b"{}".to_vec(),
        ));
    }
    if input.block_key == "documents_list" && !has_selector(html, "ul, ol") {
        diagnostics.push(diag(
            "critical",
            "html_structure",
            &input.block_key,
            "Missing list (<ul>/<ol>) element",
            b"{}".to_vec(),
        ));
    }
    if has_empty_list(html) {
        diagnostics.push(diag(
            "warning",
            "html_structure",
            &input.block_key,
            "Found empty list without <li> elements",
            b"{}".to_vec(),
        ));
    }

    let text_content = html_plain_text(html).to_lowercase();
    for phrase in FORBIDDEN_PHRASES {
        if text_content.contains(phrase) {
            diagnostics.push(diag(
                "warning",
                "html_structure",
                &input.block_key,
                format!("Forbidden phrase found: '{}'", phrase),
                b"{}".to_vec(),
            ));
        }
    }

    let wc = text_content.split_whitespace().count();
    let min_wc = min_words(&input.block_key);
    if min_wc > 0 && wc < min_wc {
        diagnostics.push(diag(
            "warning",
            "html_structure",
            &input.block_key,
            format!("Word count too low. Expected {}, got {}", min_wc, wc),
            b"{}".to_vec(),
        ));
    }

    let present_hrefs = extract_hrefs(html);
    for link in &input.required_links {
        if link.priority == 1 && !link.to_url.is_empty() && !present_hrefs.contains(&link.to_url) {
            diagnostics.push(diag(
                "critical",
                "required_links",
                &input.block_key,
                format!("Missing mandatory internal link: {}", link.to_url),
                format!(r#"{{"expected_url":"{}"}}"#, link.to_url).into_bytes(),
            ));
        }
    }

    let _ = &mut used_all;
    TypedValidationEnvelope { diagnostics }
}
