#[async_trait]
impl SeoBuildInputRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_site_build_input(
        &self,
        run_id: &str,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        sqlx_seo_adapter::load_seo_site_build_input(self.pool, run_id).await
    }
}

#[async_trait]
impl SeoBuildRegistrationRepository for SqlxSeoRuntimeRepository<'_> {
    async fn register_site_build_input(
        &self,
        request: &SeoSiteBuildRegistrationRequest,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        let non_empty_queries = request
            .queries
            .iter()
            .map(|query| query.trim().to_string())
            .filter(|query| !query.is_empty())
            .collect::<Vec<_>>();
        if non_empty_queries.is_empty() {
            return Err(DomainError::ValidationFailure {
                message: "SeoSiteBuildWorkflow requires at least one --query".to_string(),
            });
        }
        Uuid::parse_str(&request.run_id).map_err(|err| DomainError::ValidationFailure {
            message: format!("SeoSiteBuild workflow_id must be a UUID: {err}"),
        })?;

        let scope = identity::derive_scope(
            &request.market,
            &request.locale,
            &request.country_code,
            &request.visa_type,
            &request.applicant_profile,
        )?;
        sqlx_seo_adapter::ensure_seo_runtime_registries(self.pool).await?;
        let normalized_profile = sqlx_seo_adapter::validate_applicant_profile_reference(
            self.pool,
            &scope.applicant_profile,
        )
        .await?;
        let scope = identity::ValidatedSeoScope {
            applicant_profile: normalized_profile,
            ..scope
        };
        let truth_identity = identity::derive_truth_identity(
            &request.country_code,
            &request.visa_type,
            request.visa_subtype.as_deref(),
            &request.citizenship_code,
        )?;
        let resolved_context_key = if request.bootstrap_context {
            sqlx_seo_adapter::bootstrap_seo_scope(
                self.pool,
                request.context_key.as_deref(),
                &truth_identity.country_code,
                &truth_identity.visa_family,
                if truth_identity.visa_subtype.is_empty() {
                    None
                } else {
                    Some(truth_identity.visa_subtype.as_str())
                },
                &truth_identity.citizenship_code,
            )
            .await?
            .context_key
        } else {
            request
                .context_key
                .clone()
                .unwrap_or_else(|| truth_identity.context_key.clone())
        };
        let input = SeoSiteBuildInputPayload {
            run_id: request.run_id.clone(),
            context_key: resolved_context_key,
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    market: scope.market.clone(),
                    locale: scope.locale.clone(),
                    country_code: scope.country_code.clone(),
                    visa_type: scope.visa_type.clone(),
                    applicant_profile: scope.applicant_profile.clone(),
                    raw_scope_tuple: scope.raw_scope_tuple.clone(),
                    scope_signature: scope.scope_signature.clone(),
                },
            ),
            query_batch_key: request.query_batch_key.clone().unwrap_or_else(|| {
                primitives::seo::seo_artifact_key(
                    "query_batch",
                    &[
                        &request.run_id,
                        &scope.scope_signature,
                        &non_empty_queries.join("|"),
                    ],
                )
            }),
            queries: non_empty_queries,
            verified_support: Vec::new(),
            required_page_types: Vec::new(),
            run_mode: request
                .run_mode
                .clone()
                .unwrap_or_else(|| "publish_with_hitl".to_string()),
        };
        sqlx_seo_adapter::upsert_seo_site_build_input(self.pool, &input).await?;
        Ok(input)
    }
}

#[async_trait]
impl VerifiedSupportRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_verified_support_bundle(
        &self,
        request: &VerifiedSupportBundleRequest,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
        sqlx_seo_adapter::load_verified_support_bundle(
            self.pool,
            &request.run_id,
            &request.context_key,
            &request.scope_signature,
            &request.applicant_profile,
        )
        .await
    }
}

