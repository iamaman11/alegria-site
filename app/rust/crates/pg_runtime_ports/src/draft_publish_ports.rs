#[async_trait]
impl SectionTemplateRepository for SqlxSeoPgRepository<'_> {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplate>, DomainError> {
        sqlx_seo_adapter::load_section_templates(self.pool, page_type_key, dominant_intent).await
    }
}

#[async_trait]
impl DraftRepository for SqlxSeoPgRepository<'_> {
    async fn persist_draft_assemble_output(
        &self,
        run_id: &str,
        output: &DraftAssembleOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_assemble_output(self.pool, run_id, output).await
    }

    async fn persist_draft_normalize_output(
        &self,
        input: &DraftNormalizeInputPayload,
        output: &DraftNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_normalize_output(self.pool, input, output).await
    }

    async fn persist_content_contract_validate_output(
        &self,
        input: &ContentContractValidateInputPayload,
        output: &ContentContractValidateOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_content_contract_validate_output(self.pool, input, output).await
    }

    async fn persist_draft_qa_output(
        &self,
        input: &DraftQaInputPayload,
        output: &DraftQaOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_qa_output(self.pool, input, output).await
    }
}

#[async_trait]
impl CmsReviewPort for SqlxSeoPgRepository<'_> {
    async fn persist_cms_publish_output(
        &self,
        input: &CmsPublishInputPayload,
        output: &CmsPublishOutputPayload,
    ) -> Result<CmsPublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_cms_publish_output(self.pool, input, output).await
    }

    async fn load_latest_approval_decision(
        &self,
        page_node_key: &str,
        revision_id: &str,
    ) -> Result<Option<CmsApproval>, DomainError> {
        sqlx_seo_cms_adapter::load_latest_approval_decision(self.pool, page_node_key, revision_id)
            .await
    }
}

#[async_trait]
impl CmsReviewDecisionPort for SqlxSeoPgRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError> {
        sqlx_seo_cms_adapter::apply_human_review_decision(self.pool, request).await
    }
}

#[async_trait]
impl PublishArtifactRepository for SqlxSeoPgRepository<'_> {
    fn resolve_publish_materialize_input(
        &self,
        input: &PublishMaterializeInputPayload,
    ) -> PublishMaterializeInputPayload {
        sqlx_seo_cms_adapter::resolve_publish_materialize_input(input)
    }

    async fn persist_publish_materialize_output(
        &self,
        input: &PublishMaterializeInputPayload,
        output: &PublishMaterializeOutputPayload,
    ) -> Result<PublishMaterializeOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_publish_materialize_output(self.pool, input, output).await
    }

    async fn persist_finalize_publish_output(
        &self,
        input: &FinalizePublishInputPayload,
        output: &FinalizePublishOutputPayload,
    ) -> Result<FinalizePublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_finalize_publish_output(self.pool, input, output).await
    }
}

#[async_trait]
impl VerifiedRulesReader for SqlxSeoPgRepository<'_> {
    async fn read_verified_rules(
        &self,
        query: &VerifiedRulesQuery,
    ) -> Result<Option<VerifiedRulesView>, DomainError> {
        let context_key = &query.context_key;
        let active = sqlx_context_bundle_adapter::active_context_exists(self.pool, context_key)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!(
                    "knowledge.read_verified_rules stage=active_context run_id={} context_key={context_key}: {err}",
                    query.run_id
                ),
            })?;
        if !active {
            return Ok(None);
        }

        let rules_rows = sqlx_context_bundle_adapter::load_admissible_verified_rules(self.pool, context_key)
            .await
            .map_err(|err| match err {
                DomainError::ContractViolation { message } => DomainError::ContractViolation {
                    message: format!(
                        "knowledge.read_verified_rules stage=verified_rows run_id={} context_key={context_key}: {message}",
                        query.run_id
                    ),
                },
                other => DomainError::InfraUnavailable {
                    message: format!(
                        "knowledge.read_verified_rules stage=verified_rows run_id={} context_key={context_key}: {other}",
                        query.run_id
                    ),
                },
            })?;
        let rules = rules_rows
            .into_iter()
            .map(|r| {
                let role = RuleRoleType::parse(&r.role_type).ok_or_else(|| {
                    DomainError::ContractViolation {
                        message: format!(
                            "knowledge.read_verified_rules stage=role_decode run_id={} rule_id={} unknown role_type={}",
                            query.run_id, r.rule_instance_id, r.role_type
                        ),
                    }
                })?;
                Ok(VerifiedRule {
                    rule_instance_id: r.rule_instance_id,
                    rule_type_key: r.rule_type_key,
                    role,
                    params: r.params,
                    effective_from: r.effective_from,
                    effective_to: r.effective_to,
                    source_key: r.source_key,
                })
            })
            .collect::<Result<Vec<_>, DomainError>>()?;
        Ok(Some(VerifiedRulesView {
            context_key: context_key.clone(),
            rules,
        }))
    }
}
