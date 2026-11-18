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

    async fn existing_page_owners_by_cluster(
        &self,
        scope_signature: &str,
        cluster_keys: &[String],
    ) -> Result<Vec<(String, String)>, DomainError> {
        if cluster_keys.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT keyword_cluster_key, page_node_key
             FROM site.page_nodes
             WHERE scope_signature = $1 AND keyword_cluster_key = ANY($2)
               AND lifecycle_state <> 'deprecated'
             ORDER BY keyword_cluster_key, created_at",
        )
        .bind(scope_signature)
        .bind(cluster_keys)
        .fetch_all(self.pool)
        .await
        .map_err(|error| DomainError::InfraUnavailable {
            message: format!("read canonical page ownership: {error}"),
        })?;
        let mut owners = std::collections::BTreeMap::<String, String>::new();
        for row in rows {
            let cluster: String = row.get("keyword_cluster_key");
            let page: String = row.get("page_node_key");
            if let Some(previous) = owners.insert(cluster.clone(), page.clone()) {
                if previous != page {
                    return Err(DomainError::ConflictViolation {
                        message: format!("multiple active pages own keyword cluster {cluster}"),
                    });
                }
            }
        }
        Ok(owners.into_iter().collect())
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
                verified_support: Vec::new(),
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
        let mut links = output.link_recommendations.clone();
        let mut gaps = output.content_gaps.clone();
        // Reconcile historical plans through the same persistence owners.
        // Preserve unrelated scopes, manual decisions and published dependencies
        // outside this run's evaluated plan.
        if let Some(context) = input.graph_context.as_ref() {
            let keys = output.page_nodes.iter().map(|node| node.page_node_key.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let link_keys = links.iter().map(|link| link.link_recommendation_key.clone())
                .collect::<std::collections::BTreeSet<_>>();
            for prior in &context.link_recommendations {
                if matches!(prior.status.as_str(), "candidate" | "accepted")
                    && keys.contains(prior.source_page_key.as_str())
                    && keys.contains(prior.target_page_key.as_str())
                    && !link_keys.contains(&prior.link_recommendation_key)
                {
                    let mut expired = prior.clone();
                    expired.status = "expired".into();
                    links.push(expired);
                }
            }
            if !input.page_blueprints.is_empty() {
                let gap_keys = gaps.iter().map(|gap| gap.content_gap_key.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                for prior in &context.content_gaps {
                    if prior.status == "open"
                        && matches!(prior.reason_code.as_str(),
                            "page_missing_verified_support" | "page_duplicate_verified_evidence")
                        && !gap_keys.contains(&prior.content_gap_key)
                        && output.page_nodes.iter().any(|page|
                            page.page_node_key == prior.page_node_key
                            && !matches!(page.lifecycle_state.as_str(), "blocked" | "deprecated" | "stale"))
                    {
                        let mut resolved = prior.clone();
                        resolved.status = "resolved".into();
                        gaps.push(resolved);
                    }
                }
            }
        }
        sqlx_seo_adapter::persist_link_recommend_output(
            self.pool,
            &LinkRecommendInputPayload {
                run_id: input.run_id.clone(),
                page_nodes: output.page_nodes.clone(),
                max_links_per_page: 0,
                graph_context: None,
            },
            &LinkRecommendOutputPayload {
                link_recommendations: links,
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
                content_gaps: gaps,
            },
        )
        .await?;
        let scope = input.scope.as_ref().cloned().unwrap_or_default();
        let current_keys = output.page_nodes.iter().map(|page| page.page_node_key.clone()).collect::<Vec<_>>();
        sqlx_seo_adapter::retire_opaque_unpublished_plans(self.pool, &input.run_id,
            &scope.scope_signature, &current_keys).await?;
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
