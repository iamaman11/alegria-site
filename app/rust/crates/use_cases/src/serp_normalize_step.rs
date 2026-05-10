use contracts::generated::alegria::temporal::v1::{
    SerpNormalizeInputPayload, SerpNormalizeOutputPayload, SerpPatternState,
};

use crate::seo_step_support::{artifact_key, scope_signature, score};

pub fn execute(input: &SerpNormalizeInputPayload) -> SerpNormalizeOutputPayload {
    let scope_signature = scope_signature(input.scope.as_ref());
    let query_batch_key = if input.query_batch_key.trim().is_empty() {
        artifact_key("query_batch", &[&input.run_id, &scope_signature])
    } else {
        input.query_batch_key.clone()
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut serp_patterns = Vec::new();
    for query in &input.queries {
        let normalized_query = query.trim().to_ascii_lowercase();
        if normalized_query.is_empty() || !seen.insert(normalized_query.clone()) {
            continue;
        }
        let token_count = normalized_query
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .count();
        let dominant_intent = if normalized_query.contains("cost")
            || normalized_query.contains("fee")
            || normalized_query.contains("price")
            || normalized_query.contains("стоим")
        {
            "commercial"
        } else if normalized_query.contains("compare") || normalized_query.contains("сравн") {
            "comparison"
        } else if normalized_query.contains("problem")
            || normalized_query.contains("refusal")
            || normalized_query.contains("отказ")
        {
            "troubleshooting"
        } else {
            "informational"
        };
        let reliability = if token_count <= 1 {
            0.35
        } else if dominant_intent == "informational" && token_count == 2 {
            0.55
        } else {
            0.72
        };
        let status = if reliability < 0.5 {
            "partial"
        } else {
            "active"
        };
        let serp_pattern_key = artifact_key(
            "serp_pattern",
            &[
                &query_batch_key,
                &scope_signature,
                &normalized_query,
                dominant_intent,
            ],
        );
        serp_patterns.push(SerpPatternState {
            serp_pattern_key,
            query_batch_key: query_batch_key.clone(),
            scope_signature: scope_signature.clone(),
            query: normalized_query,
            pattern_type: "query_intent".to_string(),
            dominant_intent: dominant_intent.to_string(),
            reliability_score: score(reliability),
            evidence_ref: query_batch_key.clone(),
            status: status.to_string(),
        });
    }
    SerpNormalizeOutputPayload {
        scope_signature,
        normalized_query_count: serp_patterns.len() as u32,
        serp_patterns,
    }
}
