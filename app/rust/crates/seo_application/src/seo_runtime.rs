use contracts::generated::alegria::temporal::v1::{
    SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
};
use primitives::errors::DomainError;
use seo_ports::{SeoBuildInputRepository, VerifiedSupportBundleRequest, VerifiedSupportRepository};

pub async fn load_site_build_input<R: SeoBuildInputRepository>(
    repo: &R,
    run_id: &str,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    repo.load_site_build_input(run_id).await
}

/// Read verified support without imposing the downstream admissibility gate.
///
/// The canonical workflow intentionally performs this read before source discovery so
/// existing truth can enrich a run, but a fresh context is allowed to return an empty
/// bundle. Mandatory non-empty admissible support is enforced after `verified_truth_write`
/// by the post-truth projection/planning barrier and again immediately before drafting.
pub async fn load_verified_support_bundle<R: VerifiedSupportRepository>(
    repo: &R,
    request: &VerifiedSupportBundleRequest,
) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
    repo.load_verified_support_bundle(request).await
}

pub fn truth_admissibility_gate(
    bundle: &[SeoVerifiedFactSupportState],
    context_key: &str,
    applicant_profile: &str,
) -> Result<(), DomainError> {
    if bundle.is_empty() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "truth_admissibility_gate failed: admissible verified support is empty for context_key `{context_key}` and applicant_profile `{applicant_profile}`"
            ),
        });
    }
    for support in bundle {
        if support.support_ref.trim().is_empty()
            || support.fragment_text.trim().is_empty()
            || support.role_type.trim().is_empty()
            || support.source_label.trim().is_empty()
        {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "truth_admissibility_gate failed: support row `{}` is missing mandatory admissibility fields",
                    support.support_ref
                ),
            });
        }
        if support.freshness_class != "fresh" {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "truth_admissibility_gate failed: support row `{}` has non-fresh freshness_class `{}`",
                    support.support_ref, support.freshness_class
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::SeoScopePayload;

    struct FakeRepo;

    #[async_trait]
    impl SeoBuildInputRepository for FakeRepo {
        async fn load_site_build_input(
            &self,
            run_id: &str,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            Ok(SeoSiteBuildInputPayload {
                run_id: run_id.to_string(),
                context_key: "ES|tourist||BY".to_string(),
                scope: Some(SeoScopePayload {
                    market: "alegria-site".to_string(),
                    locale: "ru-RU".to_string(),
                    country_code: "ES".to_string(),
                    visa_type: "tourist".to_string(),
                    applicant_profile: "standard".to_string(),
                    raw_scope_tuple: String::new(),
                    scope_signature: "sig".to_string(),
                }),
                query_batch_key: "batch".to_string(),
                queries: vec!["spain tourist visa".to_string()],
                verified_support: Vec::new(),
                required_page_types: Vec::new(),
                run_mode: "publish_with_hitl".to_string(),
            })
        }
    }

    #[async_trait]
    impl VerifiedSupportRepository for FakeRepo {
        async fn load_verified_support_bundle(
            &self,
            _request: &VerifiedSupportBundleRequest,
        ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
            Ok(vec![SeoVerifiedFactSupportState {
                fragment_text: "Passport required".to_string(),
                support_ref: "rule:passport".to_string(),
                role_type: "document_required".to_string(),
                source_label: "Consulate".to_string(),
                source_tier: "official".to_string(),
                freshness_class: "fresh".to_string(),
                observed_at: String::new(),
                valid_until: String::new(),
            }])
        }
    }

    struct EmptySupportRepo;

    #[async_trait]
    impl VerifiedSupportRepository for EmptySupportRepo {
        async fn load_verified_support_bundle(
            &self,
            _request: &VerifiedSupportBundleRequest,
        ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
            Ok(Vec::new())
        }
    }

    fn support_with_freshness(freshness_class: &str) -> SeoVerifiedFactSupportState {
        SeoVerifiedFactSupportState {
            fragment_text: "Passport required".to_string(),
            support_ref: "rule:passport".to_string(),
            role_type: "document_required".to_string(),
            source_label: "Consulate".to_string(),
            source_tier: "official".to_string(),
            freshness_class: freshness_class.to_string(),
            observed_at: String::new(),
            valid_until: String::new(),
        }
    }

    #[tokio::test]
    async fn loads_site_build_input_without_sql_adapter() {
        let repo = FakeRepo;
        let input = load_site_build_input(&repo, "run-1").await.unwrap();
        assert_eq!(input.run_id, "run-1");
        assert_eq!(input.context_key, "ES|tourist||BY");
    }

    #[tokio::test]
    async fn loads_verified_support_without_sql_adapter() {
        let repo = FakeRepo;
        let bundle = load_verified_support_bundle(
            &repo,
            &VerifiedSupportBundleRequest {
                run_id: "run-1".to_string(),
                context_key: "ES|tourist||BY".to_string(),
                scope_signature: "sig".to_string(),
                applicant_profile: "standard".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(bundle.len(), 1);
        assert_eq!(bundle[0].support_ref, "rule:passport");
    }

    #[tokio::test]
    async fn initial_support_read_allows_zero_truth_bootstrap() {
        let bundle = load_verified_support_bundle(
            &EmptySupportRepo,
            &VerifiedSupportBundleRequest {
                run_id: "run-fresh".to_string(),
                context_key: "fresh-context".to_string(),
                scope_signature: "fresh-scope".to_string(),
                applicant_profile: "standard".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(bundle.is_empty());
    }

    #[test]
    fn rejects_empty_truth_bundle_at_admissibility_gate() {
        let err = truth_admissibility_gate(&[], "ES|tourist||BY", "standard").unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
        assert!(err.to_string().contains("truth_admissibility_gate failed"));
    }

    #[test]
    fn rejects_non_fresh_truth_bundle_at_admissibility_gate() {
        for freshness in ["watch", "stale", "unknown", ""] {
            let err = truth_admissibility_gate(
                &[support_with_freshness(freshness)],
                "ES|tourist||BY",
                "standard",
            )
            .unwrap_err();
            assert!(err.to_string().contains("non-fresh freshness_class"));
        }
        truth_admissibility_gate(
            &[support_with_freshness("fresh")],
            "ES|tourist||BY",
            "standard",
        )
        .unwrap();
    }
}
