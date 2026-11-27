use contracts::generated::alegria::temporal::v1::{
    SerpIngestInputPayload, SerpIngestOutputPayload,
};
use primitives::errors::DomainError;

use crate::seo_step_support::{artifact_key, scope_signature};

pub fn execute(input: &SerpIngestInputPayload) -> Result<SerpIngestOutputPayload, DomainError> {
    let scope_signature = scope_signature(input.scope.as_ref())?;
    let query_batch_key = if input.query_batch_key.trim().is_empty() {
        artifact_key("query_batch", &[&input.run_id, &scope_signature])
    } else {
        input.query_batch_key.clone()
    };
    let query_count = input
        .queries
        .iter()
        .filter(|query| !query.trim().is_empty())
        .count() as u32;

    Ok(SerpIngestOutputPayload {
        query_batch_key,
        query_count,
        persisted_snapshot_count: query_count,
        status: if query_count == 0 { "blocked" } else { "done" }.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::SeoScopePayload;

    #[test]
    fn standalone_preflight_blocks_an_empty_query_batch() {
        let input = SerpIngestInputPayload {
            run_id: "run-empty".into(),
            queries: vec!["".into(), " \t ".into()],
            ..Default::default()
        };
        let result = execute(&input).unwrap();
        assert_eq!(result.query_count, 0);
        assert_eq!(result.persisted_snapshot_count, 0);
        assert_eq!(result.status, "blocked");
        assert!(!result.query_batch_key.is_empty());
        assert_eq!(execute(&input).unwrap(), result);
    }

    #[test]
    fn standalone_preflight_counts_nonblank_queries_without_deduplicating() {
        let result = execute(&SerpIngestInputPayload {
            run_id: "run-repeated".into(),
            query_batch_key: "provided-batch".into(),
            queries: vec![
                " spain visa cost ".into(),
                "spain visa cost".into(),
                "\n".into(),
            ],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(result.query_batch_key, "provided-batch");
        assert_eq!(result.query_count, 2);
        // This is a preliminary pure step, not a real provider/PG receipt.
        // The application owner replaces this provisional value after effects.
        assert_eq!(result.persisted_snapshot_count, 2);
        assert_eq!(result.status, "done");
    }

    #[test]
    fn forged_scope_is_a_typed_validation_failure_before_counting() {
        let invalid_scope = SeoScopePayload {
            market: "alegria-site".into(),
            locale: "ru-RU".into(),
            country_code: "ES".into(),
            visa_type: "tourist".into(),
            applicant_profile: "standard".into(),
            scope_signature: "untrusted-caller-signature".into(),
            ..Default::default()
        };
        let error = execute(&SerpIngestInputPayload {
            scope: Some(invalid_scope),
            queries: vec!["visa fee".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(error, DomainError::ValidationFailure { .. }));
        assert!(error
            .to_string()
            .contains("scope_signature does not match normalized scope"));
    }
}
