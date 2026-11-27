use contracts::generated::alegria::temporal::v1::{
    SerpNormalizeInputPayload, SerpNormalizeOutputPayload, SerpPatternState,
};
use primitives::errors::DomainError;

use crate::seo_step_support::{artifact_key, scope_signature, score};

pub fn execute(
    input: &SerpNormalizeInputPayload,
) -> Result<SerpNormalizeOutputPayload, DomainError> {
    let scope_signature = scope_signature(input.scope.as_ref())?;
    let query_batch_key = if input.query_batch_key.trim().is_empty() {
        artifact_key("query_batch", &[&input.run_id, &scope_signature])
    } else {
        input.query_batch_key.clone()
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut serp_patterns = Vec::new();
    for query in &input.queries {
        let normalized_query = query.trim().to_lowercase();
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
    Ok(SerpNormalizeOutputPayload {
        scope_signature,
        normalized_query_count: serp_patterns.len() as u32,
        serp_patterns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::SeoScopePayload;

    #[test]
    fn normalize_can_run_without_prior_ingest_or_provider_state() {
        let input = SerpNormalizeInputPayload {
            run_id: "independent-normalize".into(),
            queries: vec![
                "  Spain Visa COST ".into(),
                "spain visa cost".into(),
                " HOW TO COMPARE VISAS ".into(),
                "visa refusal problem".into(),
                "visa".into(),
                " ".into(),
            ],
            ..Default::default()
        };
        let result = execute(&input).unwrap();
        assert_eq!(result.normalized_query_count, 4);
        assert_eq!(result.serp_patterns.len(), 4);
        assert!(result.serp_patterns.iter().all(|pattern| {
            !pattern.serp_pattern_key.is_empty()
                && !pattern.query_batch_key.is_empty()
                && pattern.scope_signature == result.scope_signature
        }));
        assert_eq!(result.serp_patterns[0].query, "spain visa cost");
        assert_eq!(result.serp_patterns[0].dominant_intent, "commercial");
        assert_eq!(result.serp_patterns[0].status, "active");
        assert_eq!(result.serp_patterns[1].dominant_intent, "comparison");
        assert_eq!(result.serp_patterns[2].dominant_intent, "troubleshooting");
        assert_eq!(result.serp_patterns[3].dominant_intent, "informational");
        assert_eq!(result.serp_patterns[3].status, "partial");
        assert_eq!(execute(&input).unwrap(), result);
    }

    #[test]
    fn normalize_preserves_supplied_batch_key_but_rejects_forged_scope() {
        let valid = SerpNormalizeInputPayload {
            run_id: "standalone-run".into(),
            query_batch_key: "supplied-batch".into(),
            queries: vec!["price of spain visa".into()],
            ..Default::default()
        };
        assert_eq!(
            execute(&valid).unwrap().serp_patterns[0].query_batch_key,
            "supplied-batch"
        );
        let invalid = SerpNormalizeInputPayload {
            scope: Some(SeoScopePayload {
                market: "alegria-site".into(),
                locale: "ru-RU".into(),
                country_code: "ES".into(),
                visa_type: "tourist".into(),
                applicant_profile: "standard".into(),
                raw_scope_tuple: "forged-raw-tuple".into(),
                ..Default::default()
            }),
            ..valid
        };
        let error = execute(&invalid).unwrap_err();
        assert!(matches!(error, DomainError::ValidationFailure { .. }));
        assert!(error
            .to_string()
            .contains("raw_scope_tuple does not match canonical normalized scope"));
    }
}
