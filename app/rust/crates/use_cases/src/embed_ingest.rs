//! Координатор: читает entity из DB → получает эмбеддинги Voyage → upsert в Qdrant.
use anyhow::Result;
use std::collections::BTreeMap;

use infrastructure::adapters::qdrant_client_adapter::{
    self as qdrant, AlegriaQdrantClient, DenseEmbeddingPoint,
};
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::sqlx_verified_rule_instances_adapter;
use infrastructure::adapters::voyage_api_adapter::VoyageClient;

/// Ingest canonical KB items в Qdrant.
/// Читает kb.rule_instances + kb.facts из PG, получает текст, встраивает, upsert.
pub async fn ingest_canonical(
    pool: &AlegriaPgPool,
    voyage: &VoyageClient,
    qdrant_client: &AlegriaQdrantClient,
    context_key: &str,
    collection: &str,
) -> Result<usize> {
    // Читаем тексты для embedding из DB
    let rows =
        sqlx_verified_rule_instances_adapter::load_embedding_text_rows(pool, context_key).await?;

    if rows.is_empty() {
        return Ok(0);
    }

    let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
    let texts: Vec<String> = rows.iter().map(|(_, t)| t.clone()).collect();

    qdrant::ensure_default_dense_collection(qdrant_client, collection, 1024).await?;

    let embeddings = voyage.embed_all(&texts).await?;

    let points: Vec<DenseEmbeddingPoint> = ids
        .into_iter()
        .zip(embeddings)
        .map(|(id, vec)| {
            let mut payload = BTreeMap::new();
            payload.insert("rule_instance_id".to_string(), id.clone());
            payload.insert("context_key".to_string(), context_key.to_string());
            DenseEmbeddingPoint {
                point_id: id,
                vector: vec,
                payload,
            }
        })
        .collect();

    let count = points.len();
    qdrant::upsert_embedding_points(qdrant_client, collection, points).await?;
    Ok(count)
}
