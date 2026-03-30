use crate::html_sections::{extract_meta_typed, extract_sections_typed};

pub fn extract_meta_json(html: &str) -> String {
    serde_json::to_string(&extract_meta_typed(html)).unwrap_or_else(|_| "{}".to_string())
}

pub fn extract_sections_json(html: &str, url: &str) -> String {
    let sections = extract_sections_typed(html, url);
    serde_json::to_string(&sections).unwrap_or_else(|_| "[]".to_string())
}
