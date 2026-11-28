// The composition-local repository retains provider-capable trait impls,
// while all SQL code is now owned by pg_runtime_ports. Forwarding is typed:
// each exact application port is implemented once here for the local wrapper
// and once by the canonical PostgreSQL adapter, with no duplicate SQL.

#[async_trait]
impl SeoBuildInputRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_site_build_input(
        &self,
        run_id: &str,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        self.pg().load_site_build_input(run_id).await
    }
}

#[async_trait]
impl SeoBuildRegistrationRepository for SqlxSeoRuntimeRepository<'_> {
    async fn register_site_build_input(
        &self,
        request: &SeoSiteBuildRegistrationRequest,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        self.pg().register_site_build_input(request).await
    }
}

#[async_trait]
impl VerifiedSupportRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_verified_support_bundle(
        &self,
        request: &VerifiedSupportBundleRequest,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
        self.pg().load_verified_support_bundle(request).await
    }
}

#[async_trait]
impl SerpIngestRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_serp_ingest_output(
        &self,
        input: &SerpIngestInputPayload,
        output: &SerpIngestOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_serp_ingest_output(input, output).await
    }

    async fn persist_live_serp_query_results(
        &self,
        run_id: &str,
        query_batch_key: &str,
        ordinal: usize,
        query: &str,
        response: &OrganicSerpResponse,
    ) -> Result<(), DomainError> {
        self.pg()
            .persist_live_serp_query_results(run_id, query_batch_key, ordinal, query, response)
            .await
    }
}

#[async_trait]
impl PlanningRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_serp_normalize_output(input, output).await
    }

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<OpportunityBuildOutputPayload, DomainError> {
        self.pg().persist_opportunity_build_output(input, output).await
    }

    async fn existing_page_owners_by_cluster(
        &self,
        scope_signature: &str,
        cluster_keys: &[String],
    ) -> Result<Vec<(String, String)>, DomainError> {
        self.pg()
            .existing_page_owners_by_cluster(scope_signature, cluster_keys)
            .await
    }

    async fn persist_ia_build_output(
        &self,
        input: &IaBuildInputPayload,
        output: &IaBuildOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_ia_build_output(input, output).await
    }

    async fn persist_link_recommend_output(
        &self,
        input: &LinkRecommendInputPayload,
        output: &LinkRecommendOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_link_recommend_output(input, output).await
    }

    async fn persist_global_site_reconcile_output(
        &self,
        input: &GlobalSiteReconcileInputPayload,
        output: &GlobalSiteReconcileOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_global_site_reconcile_output(input, output).await
    }
}

#[async_trait]
impl SectionTemplateRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplate>, DomainError> {
        self.pg().load_section_templates(page_type_key, dominant_intent).await
    }
}

#[async_trait]
impl DraftRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_draft_assemble_output(
        &self,
        run_id: &str,
        output: &DraftAssembleOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_draft_assemble_output(run_id, output).await
    }

    async fn persist_draft_normalize_output(
        &self,
        input: &DraftNormalizeInputPayload,
        output: &DraftNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_draft_normalize_output(input, output).await
    }

    async fn persist_content_contract_validate_output(
        &self,
        input: &ContentContractValidateInputPayload,
        output: &ContentContractValidateOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_content_contract_validate_output(input, output).await
    }

    async fn persist_draft_qa_output(
        &self,
        input: &DraftQaInputPayload,
        output: &DraftQaOutputPayload,
    ) -> Result<(), DomainError> {
        self.pg().persist_draft_qa_output(input, output).await
    }
}

#[async_trait]
impl CmsReviewPort for SqlxSeoRuntimeRepository<'_> {
    async fn persist_cms_publish_output(
        &self,
        input: &CmsPublishInputPayload,
        output: &CmsPublishOutputPayload,
    ) -> Result<CmsPublishOutputPayload, DomainError> {
        self.pg().persist_cms_publish_output(input, output).await
    }

    async fn load_latest_approval_decision(
        &self,
        page_node_key: &str,
        revision_id: &str,
    ) -> Result<Option<CmsApproval>, DomainError> {
        self.pg().load_latest_approval_decision(page_node_key, revision_id).await
    }
}

#[async_trait]
impl CmsReviewDecisionPort for SqlxSeoRuntimeRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError> {
        self.pg().apply_human_review_decision(request).await
    }
}

#[async_trait]
impl PublishArtifactRepository for SqlxSeoRuntimeRepository<'_> {
    fn resolve_publish_materialize_input(
        &self,
        input: &PublishMaterializeInputPayload,
    ) -> PublishMaterializeInputPayload {
        self.pg().resolve_publish_materialize_input(input)
    }

    async fn persist_publish_materialize_output(
        &self,
        input: &PublishMaterializeInputPayload,
        output: &PublishMaterializeOutputPayload,
    ) -> Result<PublishMaterializeOutputPayload, DomainError> {
        self.pg().persist_publish_materialize_output(input, output).await
    }

    async fn persist_finalize_publish_output(
        &self,
        input: &FinalizePublishInputPayload,
        output: &FinalizePublishOutputPayload,
    ) -> Result<FinalizePublishOutputPayload, DomainError> {
        self.pg().persist_finalize_publish_output(input, output).await
    }
}

#[async_trait]
impl VerifiedRulesReader for SqlxSeoRuntimeRepository<'_> {
    async fn read_verified_rules(
        &self,
        query: &VerifiedRulesQuery,
    ) -> Result<Option<VerifiedRulesView>, DomainError> {
        self.pg().read_verified_rules(query).await
    }
}

#[async_trait]
impl ProjectionStatusRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_projection_barrier_status(
        &self,
        run_id: &str,
    ) -> Result<ProjectionBarrierStatus, DomainError> {
        self.pg().load_projection_barrier_status(run_id).await
    }
}
