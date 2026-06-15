#[async_trait]
impl PlanningRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_graph_planning_context(
        &self,
        run_id: &str,
        scope_signature: &str,
    ) -> Result<GraphPlanningContext, DomainError> {
        sqlx_seo_adapter::load_graph_planning_context(self.pool, run_id, scope_signature).await
    }

    async fn persist_serp_ingest_output(
        &self,
        input: &SerpIngestInputPayload,
        output: &SerpIngestOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_ingest_output(self.pool, input, output).await
    }

    async fn persist_live_serp_query_results(
        &self,
        run_id: &str,
        query_batch_key: &str,
        ordinal: usize,
        query: &str,
        response: &OrganicSerpResponse,
    ) -> Result<(), DomainError> {
        let job_id = primitives::hash::content_hash_v1(&format!(
            "{run_id}|{query_batch_key}|{ordinal}|{query}"
        ));
        sqlx_serp_adapter::save_raw_snapshot(
            self.pool,
            &sqlx_serp_adapter::RawSnapshotRecord {
                run_id: run_id.to_string(),
                job_id: job_id.clone(),
                url_norm: query.to_string(),
                query: query.to_string(),
                raw_payload_utf8: response.raw_payload_utf8.clone(),
            },
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO raw snapshot: {err}"),
        })?;
        let results = response
            .organic_results
            .iter()
            .map(|result| dataforseo_serp_adapter::DataForSeoOrganicResult {
                rank: result.rank,
                title: result.title.clone(),
                url: result.url.clone(),
                url_norm: result.url_norm.clone(),
                domain_norm: result.domain_norm.clone(),
                source_tier: result.source_tier.clone(),
                snippet: result.snippet.clone(),
                raw_json: serde_json::Value::Null,
            })
            .collect::<Vec<_>>();
        sqlx_serp_adapter::save_dataforseo_organic_results_and_enqueue(
            self.pool,
            run_id,
            &job_id,
            query_batch_key,
            &results,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO organic results: {err}"),
        })?;
        Ok(())
    }

    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_normalize_output(self.pool, input, output).await
    }

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_opportunity_build_output(self.pool, input, output).await
    }

    async fn persist_ia_build_output(
        &self,
        input: &IaBuildInputPayload,
        output: &IaBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(self.pool, input, output).await
    }

    async fn persist_link_recommend_output(
        &self,
        input: &LinkRecommendInputPayload,
        output: &LinkRecommendOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_link_recommend_output(self.pool, input, output).await
    }

    async fn persist_global_site_reconcile_output(
        &self,
        input: &GlobalSiteReconcileInputPayload,
        output: &GlobalSiteReconcileOutputPayload,
    ) -> Result<GlobalNavigationPersistReport, DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(
            self.pool,
            &IaBuildInputPayload {
                run_id: input.run_id.clone(),
                scope: input.scope.clone(),
                keyword_clusters: Vec::new(),
                graph_context: None,
            },
            &IaBuildOutputPayload {
                page_nodes: output.page_nodes.clone(),
                page_blueprints: Vec::new(),
                cannibalization_conflicts: Vec::new(),
            },
        )
        .await?;
        sqlx_seo_adapter::persist_link_recommend_output(
            self.pool,
            &LinkRecommendInputPayload {
                run_id: input.run_id.clone(),
                page_nodes: output.page_nodes.clone(),
                max_links_per_page: 0,
                graph_context: None,
            },
            &LinkRecommendOutputPayload {
                link_recommendations: output.link_recommendations.clone(),
            },
        )
        .await?;
        sqlx_seo_adapter::persist_opportunity_build_output(
            self.pool,
            &OpportunityBuildInputPayload {
                run_id: input.run_id.clone(),
                scope: input.scope.clone(),
                serp_patterns: Vec::new(),
                graph_context: None,
            },
            &OpportunityBuildOutputPayload {
                keyword_clusters: Vec::new(),
                content_gaps: output.content_gaps.clone(),
            },
        )
        .await?;
        let scope = input.scope.as_ref().cloned().unwrap_or_default();
        let navigation = sqlx_seo_adapter::persist_global_navigation_from_active_pages(
            self.pool,
            &scope.market,
            &scope.locale,
            &input.reconcile_reason,
        )
        .await?;
        Ok(GlobalNavigationPersistReport {
            navigation_tree_key: navigation.navigation_tree_key,
            scope_count: navigation.scope_count,
            page_item_count: navigation.page_item_count,
            silo_group_count: navigation.silo_group_count,
            rebuild_plan_count: navigation.rebuild_plan_count,
        })
    }
}

