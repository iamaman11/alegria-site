use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

use super::qdrant_client_adapter as qdrant;
use super::voyage_api_adapter::{
    VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageRerankOptions,
};

#[derive(Debug, Clone, Default)]
pub struct SearchResultRecord {
    pub entity_key: String,
    pub score: f32,
    pub payload: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoyageSearchSurface {
    Standard,
    Contextualized,
}

impl VoyageSearchSurface {
    fn default_model(self) -> &'static str {
        match self {
            Self::Standard => "voyage-4-large",
            Self::Contextualized => "voyage-context-3",
        }
    }
}

fn env_voyage_model(surface: VoyageSearchSurface) -> String {
    match surface {
        VoyageSearchSurface::Standard => {
            std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| surface.default_model().to_string())
        }
        VoyageSearchSurface::Contextualized => std::env::var("VOYAGE_CONTEXT_MODEL")
            .unwrap_or_else(|_| surface.default_model().to_string()),
    }
}

pub async fn rerank_records(
    query: &str,
    records: Vec<SearchResultRecord>,
    top_k: Option<usize>,
) -> Result<Vec<SearchResultRecord>> {
    if records.len() <= 1 {
        return Ok(records);
    }
    let voyage_api_key =
        std::env::var("VOYAGE_API_KEY").map_err(|_| anyhow!("VOYAGE_API_KEY is not set"))?;
    let voyage = VoyageClient::new(voyage_api_key, env_voyage_model(VoyageSearchSurface::Standard));
    let documents = records
        .iter()
        .map(|record| {
            record
                .payload
                .get("retrieval_text")
                .or_else(|| record.payload.get("label_ru"))
                .or_else(|| record.payload.get("aliases"))
                .or_else(|| record.payload.get("heading_path"))
                .or_else(|| record.payload.get("entity_key"))
                .map(String::as_str)
                .unwrap_or("")
        })
        .collect::<Vec<_>>();
    let reranked = voyage
        .rerank(
            query,
            &documents,
            &VoyageRerankOptions {
                top_k,
                truncation: Some(false),
                return_documents: false,
            },
            None,
        )
        .await?;
    if reranked.is_empty() {
        return Ok(records);
    }
    let mut reordered = Vec::with_capacity(reranked.len());
    for entry in reranked {
        if let Some(mut record) = records.get(entry.index).cloned() {
            record.score = entry.relevance_score;
            reordered.push(record);
        }
    }
    if reordered.is_empty() {
        Ok(records)
    } else {
        Ok(reordered)
    }
}

pub async fn search_by_text(
    query: &str,
    collection: &str,
    limit: u64,
) -> Result<Vec<SearchResultRecord>> {
    search_by_text_with_surface(query, collection, limit, VoyageSearchSurface::Standard).await
}

pub async fn search_by_text_with_surface(
    query: &str,
    collection: &str,
    limit: u64,
    surface: VoyageSearchSurface,
) -> Result<Vec<SearchResultRecord>> {
    let qdrant_url =
        std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let voyage_api_key =
        std::env::var("VOYAGE_API_KEY").map_err(|_| anyhow!("VOYAGE_API_KEY is not set"))?;
    let voyage_model = env_voyage_model(surface);

    let client = qdrant::connect_qdrant(&qdrant_url).await?;
    let voyage = VoyageClient::new(voyage_api_key, voyage_model);

    let vector = match surface {
        VoyageSearchSurface::Standard => voyage
            .embed_batch_with_settings(
                &[query],
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Query),
                    output_dimension: Some(1024),
                    output_dtype: None,
                    truncation: Some(false),
                },
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("empty embedding from Voyage"))?,
        VoyageSearchSurface::Contextualized => voyage
            .contextualized_embed(
                &[vec![query]],
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Query),
                    output_dimension: Some(1024),
                    output_dtype: None,
                    truncation: Some(false),
                },
                None,
            )
            .await?
            .into_iter()
            .next()
            .and_then(|group| group.into_iter().next())
            .ok_or_else(|| anyhow!("empty contextualized embedding from Voyage"))?,
    };

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
