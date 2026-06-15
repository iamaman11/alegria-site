#[async_trait]
impl RebuildRepository for SqlxSeoRuntimeRepository<'_> {
    async fn narrow_rebuild_impacts(
        &self,
        changed_truth_keys: &[String],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
        let impacted =
            sqlx_seo_adapter::resolve_rebuild_impacts(self.pool, changed_truth_keys).await?;
        Ok(impacted
            .into_iter()
            .map(
                |(page_node_key, reason_package)| RebuildDependencyEvidence {
                    page_node_key,
                    reason_package,
                },
            )
            .collect())
    }

    async fn persist_rebuild_detect_output(
        &self,
        input: &RebuildDetectInputPayload,
        output: &RebuildDetectOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_rebuild_detect_output(self.pool, input, output).await
    }

    async fn semantic_neighbor_impacts(
        &self,
        changed_truth_keys: &[String],
        page_nodes: &[PageNodeState],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
        let retrieval_required = std::env::var("RETRIEVAL_CAPABILITY_REQUIRED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if std::env::var("VOYAGE_API_KEY").is_err() {
            if retrieval_required {
                return Err(DomainError::InfraUnavailable {
                    message: "semantic rebuild widening requires VOYAGE_API_KEY under hard-required retrieval contract"
                        .to_string(),
                });
            }
            return Ok(Vec::new());
        }
        let mut impacts = Vec::new();
        let query = changed_truth_keys
            .iter()
            .filter_map(|key| {
                key.strip_prefix("verified.rule_instance:")
                    .or_else(|| key.strip_prefix("truth_support:"))
                    .or_else(|| key.strip_prefix("source_key:"))
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>()
            .join(" ");
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let candidates = semantic_search_adapter::search_by_text(&query, "editorial_topics_4", 16)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("semantic rebuild widening retrieval failed: {err}"),
            })?;
        let reranked = semantic_search_adapter::rerank_records(&query, candidates, Some(8))
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("semantic rebuild widening rerank failed: {err}"),
            })?;
        let page_node_set = page_nodes
            .iter()
            .map(|node| node.page_node_key.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for candidate in reranked {
            let page_node_key = candidate
                .payload
                .get("page_node_key")
                .cloned()
                .filter(|key| !key.trim().is_empty())
                .unwrap_or(candidate.entity_key);
            if !page_node_set.contains(page_node_key.as_str()) {
                continue;
            }
            impacts.push(RebuildDependencyEvidence {
                page_node_key: page_node_key.clone(),
                reason_package: json!({
                    "semantic_neighbor_widening": true,
                    "collection": "editorial_topics_4",
                    "score": candidate.score,
                    "matched_from": query,
                }),
            });
        }
        Ok(impacts)
    }
}

