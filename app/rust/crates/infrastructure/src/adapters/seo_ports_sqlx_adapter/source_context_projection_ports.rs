#[async_trait]
impl SourceContextRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_source_context_chunks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SourceContextChunkState>, DomainError> {
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
                    message:
                        "source context retrieval requires VOYAGE_API_KEY under hard-required retrieval contract"
                            .to_string(),
                });
            }
            return Ok(Vec::new());
        }
        let mut chunks = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let per_collection_limit = limit.max(4);
        for (collection_name, surface) in [
            (
                "verified_rules_4",
                semantic_search_adapter::VoyageSearchSurface::Standard,
            ),
            (
                "raw_chunks_ctx",
                semantic_search_adapter::VoyageSearchSurface::Contextualized,
            ),
            (
                "editorial_topics_4",
                semantic_search_adapter::VoyageSearchSurface::Standard,
            ),
            (
                "raw_chunks_4",
                semantic_search_adapter::VoyageSearchSurface::Standard,
            ),
        ] {
            let found = match search_context_collection(
                collection_name,
                query,
                per_collection_limit,
                surface,
            )
            .await
            {
                Ok(found) => found,
                Err(err) if !retrieval_required => {
                    let _ = err;
                    continue;
                }
                Err(err) => return Err(err),
            };
            for chunk in found {
                if seen.insert(chunk.chunk_key.clone()) {
                    chunks.push(chunk);
                }
                if chunks.len() >= limit {
                    return Ok(chunks);
                }
            }
        }
        Ok(chunks)
    }
}

#[async_trait]
impl ProjectionStatusRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_projection_barrier_status(
        &self,
        run_id: &str,
    ) -> Result<ProjectionBarrierStatus, DomainError> {
        let statuses =
            sqlx_seo_adapter::read_projection_sync_status_for_run(self.pool, run_id).await?;
        Ok(ProjectionBarrierStatus {
            blocked_events: statuses
                .iter()
                .map(|status| status.blocking_event_count())
                .sum(),
            max_open_lag_ms: statuses
                .iter()
                .map(|status| status.max_open_lag_ms)
                .max()
                .unwrap_or(0),
        })
    }
}

