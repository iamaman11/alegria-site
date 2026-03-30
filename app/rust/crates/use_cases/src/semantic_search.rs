use anyhow::Result;
use infrastructure::adapters::semantic_search_adapter;
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct SearchResult {
    pub entity_key: String,
    pub score: f32,
    pub payload: BTreeMap<String, String>,
}

pub async fn search_by_text(
    _pool: &AlegriaPgPool,
    query: &str,
    collection: &str,
    limit: u64,
) -> Result<Vec<SearchResult>> {
    semantic_search_adapter::search_by_text(query, collection, limit)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| SearchResult {
                    entity_key: row.entity_key,
                    score: row.score,
                    payload: row.payload,
                })
                .collect()
        })
}
