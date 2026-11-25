#[async_trait]
impl SourceContextSearchPort for SqlxSeoRuntimeRepository<'_> {
    fn provider_configured(&self) -> bool {
        std::env::var("VOYAGE_API_KEY").is_ok()
    }

    fn retrieval_required(&self) -> bool {
        std::env::var("RETRIEVAL_CAPABILITY_REQUIRED")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
    }

    async fn search_source_context_collection(
        &self,
        collection: SourceContextCollection,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SourceContextChunk>, DomainError> {
        search_context_collection(collection, query, limit).await
    }
}
