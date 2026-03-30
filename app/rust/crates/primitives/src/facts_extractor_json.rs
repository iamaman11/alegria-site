use crate::facts_extractor::{extract_facts_typed, ExtractedFactsEnvelope, ExtractionContext};

pub fn extract_facts_json(markdown: &str, context_json: &str) -> String {
    let context = serde_json::from_str::<ExtractionContext>(context_json).unwrap_or_default();
    let extracted: ExtractedFactsEnvelope = extract_facts_typed(markdown, &context);
    serde_json::to_string(&extracted)
        .unwrap_or_else(|_| "{\"rule_instances\":[],\"facts\":[]}".to_string())
}
