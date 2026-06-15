#[async_trait]
impl SectionTemplateRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
        sqlx_seo_adapter::load_section_templates(self.pool, page_type_key, dominant_intent).await
    }
}

#[async_trait]
impl DraftRepository for SqlxSeoRuntimeRepository<'_> {
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
impl EditorialGenerationPort for SqlxSeoRuntimeRepository<'_> {
    async fn generate_editorial_draft(
        &self,
        input: &EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
        editorial_llm_adapter::generate_editorial_draft(input).await
    }
}

#[async_trait]
impl CmsReviewPort for SqlxSeoRuntimeRepository<'_> {
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
    ) -> Result<Option<CmsApprovalDecision>, DomainError> {
        sqlx_seo_cms_adapter::load_latest_approval_decision(self.pool, page_node_key, revision_id)
            .await
    }
}

#[async_trait]
impl CmsReviewDecisionPort for SqlxSeoRuntimeRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError> {
        sqlx_seo_cms_adapter::apply_human_review_decision(self.pool, request).await
    }
}

#[async_trait]
impl PublishArtifactRepository for SqlxSeoRuntimeRepository<'_> {
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
impl ContextBundleRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_context_bundle(&self, context_key: &str) -> Result<ContextBundle, DomainError> {
        let Some(ctx) =
            sqlx_context_bundle_adapter::load_context_bundle_base(self.pool, context_key).await?
        else {
            return Ok(ContextBundle {
                context_key: context_key.to_string(),
                country_code: String::new(),
                visa_family: String::new(),
                visa_subtype: String::new(),
                citizenship_code: String::new(),
                ontology_rules: vec![],
                citation_facts: vec![],
                related_links: vec![],
            });
        };

        let rules_rows =
            sqlx_context_bundle_adapter::load_context_bundle_rules(self.pool, context_key).await?;
        let mut facts = Vec::new();
        for r in rules_rows {
            let citation = if let Some(sk) = r.source_key.clone() {
                Some(SourceCitation {
                    source_key: sk,
                    source_label: r.source_label.unwrap_or_default(),
                    base_url: r.base_url.unwrap_or_default(),
                })
            } else {
                None
            };

            facts.push(CitationFact {
                rule_instance_id: r.rule_instance_id,
                rule_type_key: r.rule_type_key,
                status: r.status,
                effective_from: r.effective_from,
                effective_to: r.effective_to,
                citation,
                role_type: read_role_type(&r.role_type),
                params: map_params(&r.params),
            });
        }

        Ok(ContextBundle {
            context_key: context_key.to_string(),
            country_code: ctx.country_code,
            visa_family: ctx.visa_family,
            visa_subtype: ctx.visa_subtype,
            citizenship_code: ctx.citizenship_code,
            ontology_rules: vec![],
            citation_facts: facts,
            related_links: vec![],
        })
    }
}

#[async_trait]
impl HitlQueuePort for SqlxSeoRuntimeRepository<'_> {
    async fn enqueue_hitl_task(
        &self,
        task_type: &str,
        diagnostics: &HitlTaskContext,
        priority: i32,
    ) -> Result<i64, DomainError> {
        sqlx_hitl_adapter::enqueue_hitl_task(self.pool, task_type, diagnostics, priority).await
    }

    async fn resolve_hitl_task(
        &self,
        task_id: i64,
        resolution: &HitlDecision,
    ) -> Result<(), DomainError> {
        sqlx_hitl_adapter::resolve_hitl_task(self.pool, task_id, resolution).await
    }
}
