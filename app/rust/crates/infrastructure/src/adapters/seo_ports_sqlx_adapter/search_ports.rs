#[async_trait]
impl SerpSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn fetch_google_organic_live_advanced(
        &self,
        locale: Option<&str>,
        query: &str,
    ) -> Result<Option<OrganicSerpResponse>, DomainError> {
        let Some(config) = dataforseo_serp_adapter::DataForSeoConfig::from_env_with_locale(locale)
        else {
            return Ok(None);
        };
        let client =
            dataforseo_serp_adapter::DataForSeoSerpClient::from_config(config).map_err(|err| {
                DomainError::InfraUnavailable {
                    message: format!("configure DataForSEO client: {err}"),
                }
            })?;
        let response = client
            .google_organic_live_advanced(query)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("DataForSEO organic live advanced failed: {err}"),
            })?;
        Ok(Some(OrganicSerpResponse {
            raw_payload_utf8: response.raw_payload_utf8,
            organic_results: response
                .organic_results
                .into_iter()
                .map(|result| OrganicSerpResult {
                    rank: result.rank,
                    title: result.title,
                    url: result.url,
                    url_norm: result.url_norm,
                    domain_norm: result.domain_norm,
                    source_tier: result.source_tier,
                    snippet: result.snippet,
                })
                .collect(),
        }))
    }
}

#[async_trait]
impl SemanticLinkSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn search_link_targets(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
        let results = semantic_search_adapter::search_by_text(
            query,
            "editorial_topics_4",
            u64::try_from(limit).map_err(|_| DomainError::ValidationFailure {
                message: format!("semantic link search limit too large: {limit}"),
            })?,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("semantic search failed: {err}"),
        })?;
        let reranked = semantic_search_adapter::rerank_records(query, results, Some(limit))
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("semantic link rerank failed: {err}"),
            })?;
        Ok(reranked
            .into_iter()
            .map(|candidate| SemanticLinkCandidate {
                entity_key: candidate.entity_key,
                score: candidate.score,
            })
            .collect())
    }

    async fn search_keyword_clusters(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
        let results = semantic_search_adapter::search_by_text(
            query,
            "seo_keyword_clusters_4",
            u64::try_from(limit).map_err(|_| DomainError::ValidationFailure {
                message: format!("semantic keyword cluster search limit too large: {limit}"),
            })?,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("semantic keyword cluster search failed: {err}"),
        })?;
        let reranked = semantic_search_adapter::rerank_records(query, results, Some(limit))
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("semantic keyword cluster rerank failed: {err}"),
            })?;
        Ok(reranked
            .into_iter()
            .map(|candidate| SemanticLinkCandidate {
                entity_key: candidate.entity_key,
                score: candidate.score,
            })
            .collect())
    }

    async fn cluster_demand_queries(
        &self,
        queries: &[String],
    ) -> Result<Vec<SemanticDemandCluster>, DomainError> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let retrieval_required = std::env::var("RETRIEVAL_CAPABILITY_REQUIRED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let api_key = match std::env::var("VOYAGE_API_KEY") {
            Ok(key) => key,
            Err(_) if retrieval_required => {
                return Err(DomainError::InfraUnavailable {
                    message: "step0 demand clustering requires VOYAGE_API_KEY under hard-required retrieval contract"
                        .to_string(),
                })
            }
            Err(_) => {
                return Ok(queries
                    .iter()
                    .map(|query| SemanticDemandCluster {
                        cluster_key: primitives::seo::seo_artifact_key(
                            "step0_demand_cluster",
                            &[query, "deterministic@1"],
                        ),
                        member_queries: vec![query.clone()],
                        confidence: 1.0,
                    })
                    .collect())
            }
        };
        let model = std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
        let voyage = VoyageClient::new(api_key, model);
        let embeddings = voyage
            .embed_all_with_settings(
                queries,
                &VoyageEmbeddingOptions {
                    input_type: None,
                    output_dimension: Some(1024),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(false),
                },
            )
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("step0 demand clustering embeddings failed: {err}"),
            })?;
        Ok(cluster_queries_from_embeddings(queries, &embeddings, 0.86))
    }
}

