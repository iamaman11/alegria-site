use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

use super::qdrant_client_adapter as qdrant;
use super::voyage_api_adapter::VoyageClient;

#[derive(Debug, Clone, Default)]
pub struct SearchResultRecord {
    pub entity_key: String,
    pub score: f32,
    pub payload: BTreeMap<String, String>,
}

pub async fn search_by_text(query: &str, collection: &str, limit: u64) -> Result<Vec<SearchResultRecord>> {
    let qdrant_url = std::env::var("QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());
    let voyage_api_key = std::env::var("VOYAGE_API_KEY")
        .map_err(|_| anyhow!("VOYAGE_API_KEY is not set"))?;
    let voyage_model = std::env::var("VOYAGE_MODEL")
        .unwrap_or_else(|_| "voyage-3-large".to_string());

    let client = qdrant::connect_qdrant(&qdrant_url).await?;
    let voyage = VoyageClient::new(voyage_api_key, voyage_model);

    let vectors = voyage.embed_batch(&[query]).await?;
    let vector = vectors
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("empty embedding from Voyage"))?;

    let points = qdrant::search_dense(&client, collection, vector, limit, None).await?;
    let mut out = Vec::with_capacity(points.len());
    for p in points {
        let payload = qdrant::scored_point_payload_map(&p);
        let entity_key = payload.get("entity_key").cloned().unwrap_or_default();
        out.push(SearchResultRecord {
            entity_key,
            score: p.score,
            payload,
        });
    }
    Ok(out)
}
