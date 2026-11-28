#[async_trait]
impl GraphReasoningPort for SqlxSeoRuntimeRepository<'_> {
    async fn load_planning_graph_context(
        &self,
        scope_signature: &str,
        run_id: &str,
    ) -> Result<GraphPlanningContext, DomainError> {
        ensure_graph_contract_if_required("planning_graph_context").await?;
        let mut context =
            sqlx_seo_adapter::load_graph_planning_context(self.pool, run_id, scope_signature)
                .await?;
        let neo4j_uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
        let neo4j_user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
        let neo4j_password =
            std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
        let graph = graph_neo4j::connect_neo4j(&neo4j_uri, &neo4j_user, &neo4j_password)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("neo4j planning context connect failed: {err}"),
            })?;
        let links = graph_neo4j::planning_links::load_planning_links(&graph, scope_signature)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("neo4j planning context query failed: {err}"),
            })?;
        for link in links {
            let source_page_key = link.source_page_key;
            let target_page_key = link.target_page_key;
            let score = link.score;
            let link_role = link.link_role;
            let support_ref =
                format!("neo4j://PageNode/{source_page_key}/RECOMMENDS_LINK_TO/{target_page_key}");
            if let Some(existing) = context.link_recommendations.iter_mut().find(|link| {
                link.source_page_key == source_page_key && link.target_page_key == target_page_key
            }) {
                if !existing.support_refs.contains(&support_ref) {
                    existing.support_refs.push(support_ref);
                }
                existing.graph_confidence = format!(
                    "{:.4}",
                    existing
                        .graph_confidence
                        .parse::<f64>()
                        .unwrap_or_default()
                        .max(score)
                );
                continue;
            }
            context
                .link_recommendations
                .push(runtime_models::LinkRecommendation {
                    link_recommendation_key: seo_domain::artifact_identity::seo_artifact_key(
                        "link_recommendation",
                        &[
                            scope_signature,
                            &source_page_key,
                            &target_page_key,
                            &link_role,
                            "neo4j@1",
                        ],
                    ),
                    scope_signature: scope_signature.to_string(),
                    source_page_key,
                    target_page_key,
                    link_role,
                    anchor_strategy: "graph_neighborhood".to_string(),
                    required_flag: false,
                    score,
                    status: "candidate".to_string(),
                    reason_code: "neo4j_neighborhood".to_string(),
                    topic_keys: Vec::new(),
                    triple_refs: Vec::new(),
                    graph_confidence: format!("{score:.4}"),
                    support_refs: vec![support_ref],
                });
        }
        Ok(context)
    }
}

#[async_trait]
impl GraphCapabilityPort for SqlxSeoRuntimeRepository<'_> {
    async fn ensure_graph_contract(&self, context_key: &str) -> Result<(), DomainError> {
        ensure_graph_contract_if_required(context_key).await
    }
}
