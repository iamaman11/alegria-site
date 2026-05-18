use contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload;
use primitives::errors::DomainError;
use seo_ports::{SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest};

pub async fn register_site_build_input<R: SeoBuildRegistrationRepository>(
    repo: &R,
    request: &SeoSiteBuildRegistrationRequest,
) -> Result<SeoSiteBuildInputPayload, DomainError> {
    repo.register_site_build_input(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::SeoScopePayload;

    struct FakeRepo;

    #[async_trait]
    impl SeoBuildRegistrationRepository for FakeRepo {
        async fn register_site_build_input(
            &self,
            request: &SeoSiteBuildRegistrationRequest,
        ) -> Result<SeoSiteBuildInputPayload, DomainError> {
            Ok(SeoSiteBuildInputPayload {
                run_id: request.run_id.clone(),
                context_key: request.context_key.clone().unwrap_or_default(),
                scope: Some(SeoScopePayload {
                    market: request.market.clone(),
                    locale: request.locale.clone(),
                    country_code: request.country_code.clone(),
                    visa_type: request.visa_type.clone(),
                    applicant_profile: request.applicant_profile.clone(),
                    raw_scope_tuple: String::new(),
                    scope_signature: "sig".to_string(),
                }),
                query_batch_key: request.query_batch_key.clone().unwrap_or_default(),
                queries: request.queries.clone(),
                verified_support: Vec::new(),
                required_page_types: Vec::new(),
                run_mode: request.run_mode.clone().unwrap_or_default(),
            })
        }
    }

    #[tokio::test]
    async fn registration_routes_through_repo() {
        let repo = FakeRepo;
        let input = register_site_build_input(
            &repo,
            &SeoSiteBuildRegistrationRequest {
                run_id: "run-1".to_string(),
                context_key: Some("ES|tourist||BY".to_string()),
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec!["spain tourist visa".to_string()],
                query_batch_key: Some("batch-1".to_string()),
                run_mode: Some("publish_with_hitl".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(input.run_id, "run-1");
        assert_eq!(input.queries.len(), 1);
    }
}
