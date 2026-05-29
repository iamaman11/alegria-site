use anyhow::{bail, Context, Result};
pub use qdrant_client::qdrant::{
    CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointStruct, PointsIdsList,
    ScoredPoint, SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
};
pub use qdrant_client::{Payload, Qdrant};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;

pub type AlegriaQdrantClient = Qdrant;

#[derive(Debug, Clone)]
pub struct DenseEmbeddingPoint {
    pub point_id: String,
    pub vector: Vec<f32>,
    pub payload: BTreeMap<String, String>,
}

pub async fn connect_qdrant(url: &str) -> Result<Qdrant> {
    let skip_compatibility = env::var("QDRANT_SKIP_COMPATIBILITY_CHECK")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let builder = if skip_compatibility {
        Qdrant::from_url(url).skip_compatibility_check()
    } else {
        Qdrant::from_url(url)
    };
    Ok(builder.build()?)
}

pub fn parse_distance(distance: &str) -> Result<Distance> {
    let d = distance.trim().to_ascii_lowercase();
    match d.as_str() {
        "cosine" => Ok(Distance::Cosine),
        "dot" => Ok(Distance::Dot),
        "euclid" => Ok(Distance::Euclid),
        "manhattan" => Ok(Distance::Manhattan),
        _ => bail!("unsupported qdrant distance: {distance}"),
    }
}

pub fn normalize_payload(
    entity_type: &str,
    entity_key: &str,
    payload_value: Option<&Value>,
) -> Value {
    let mut merged = Map::new();
    merged.insert(
        "entity_type".to_string(),
        Value::String(entity_type.to_string()),
    );
    merged.insert(
        "entity_key".to_string(),
        Value::String(entity_key.to_string()),
    );
    if let Some(v) = payload_value {
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                merged.insert(k.clone(), val.clone());
            }
        } else {
            merged.insert("source_payload".to_string(), v.clone());
        }
    }
    Value::Object(merged)
}

pub async fn ensure_dense_collection(
    client: &Qdrant,
    collection_name: &str,
    vector_size: u64,
    distance: Distance,
) -> Result<()> {
    if vector_size == 0 {
        bail!("vector_size must be > 0");
    }
    if client.collection_exists(collection_name).await? {
        return Ok(());
    }

    client
        .create_collection(
            CreateCollectionBuilder::new(collection_name)
                .vectors_config(VectorParamsBuilder::new(vector_size, distance)),
        )
        .await
        .with_context(|| format!("failed to create qdrant collection: {collection_name}"))?;
    Ok(())
}

pub async fn ensure_default_dense_collection(
    client: &AlegriaQdrantClient,
    collection_name: &str,
    vector_size: u64,
) -> Result<()> {
    ensure_dense_collection(client, collection_name, vector_size, Distance::Cosine).await
}

pub async fn upsert_dense_point(
    client: &Qdrant,
    collection_name: &str,
    point_id: &str,
    vector: Vec<f32>,
    payload_value: Value,
) -> Result<()> {
    if vector.is_empty() {
        bail!("vector is empty");
    }
    let payload: Payload = payload_value
        .try_into()
        .context("failed to convert payload_value into qdrant payload")?;
    let point = PointStruct::new(point_id.to_string(), vector, payload);
    client
        .upsert_points(UpsertPointsBuilder::new(collection_name, vec![point]).wait(true))
        .await
        .with_context(|| {
            format!("failed qdrant upsert for collection={collection_name}, point_id={point_id}")
        })?;
    Ok(())
}

pub async fn upsert_batch(
    client: &Qdrant,
    collection_name: &str,
    points: Vec<(String, Vec<f32>, Value)>,
) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    let pts: Result<Vec<PointStruct>> = points
        .into_iter()
        .map(|(id, vec, payload)| {
            let p: Payload = payload.try_into().context("failed to convert payload")?;
            Ok(PointStruct::new(id, vec, p))
        })
        .collect();
    client
        .upsert_points(UpsertPointsBuilder::new(collection_name, pts?).wait(true))
        .await?;
    Ok(())
}

pub async fn upsert_batch_map(
    client: &Qdrant,
    collection_name: &str,
    points: Vec<(String, Vec<f32>, BTreeMap<String, String>)>,
) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    let pts: Result<Vec<PointStruct>> = points
        .into_iter()
        .map(|(id, vec, payload_map)| {
            let payload_value = serde_json::to_value(payload_map)
                .context("failed to serialize payload_map into json")?;
            let p: Payload = payload_value
                .try_into()
                .context("failed to convert payload_map")?;
            Ok(PointStruct::new(id, vec, p))
        })
        .collect();
    client
        .upsert_points(UpsertPointsBuilder::new(collection_name, pts?).wait(true))
        .await?;
    Ok(())
}

pub async fn upsert_embedding_points(
    client: &AlegriaQdrantClient,
    collection_name: &str,
    points: Vec<DenseEmbeddingPoint>,
) -> Result<()> {
    let mapped = points
        .into_iter()
        .map(|point| (point.point_id, point.vector, point.payload))
        .collect();
    upsert_batch_map(client, collection_name, mapped).await
}

pub async fn delete_point_ids(
    client: &AlegriaQdrantClient,
    collection_name: &str,
    point_ids: Vec<String>,
) -> Result<()> {
    if point_ids.is_empty() {
        return Ok(());
    }
    client
        .delete_points(
            DeletePointsBuilder::new(collection_name)
                .points(PointsIdsList {
                    ids: point_ids.into_iter().map(Into::into).collect(),
                })
                .wait(true),
        )
        .await
        .with_context(|| format!("failed qdrant delete for collection={collection_name}"))?;
    Ok(())
}

pub async fn search_dense(
    client: &Qdrant,
    collection: &str,
    vector: Vec<f32>,
    limit: u64,
    filter: Option<Filter>,
) -> Result<Vec<ScoredPoint>> {
    if vector.is_empty() {
        bail!("search vector is empty");
    }
    let mut builder = SearchPointsBuilder::new(collection, vector, limit);
    if let Some(f) = filter {
        builder = builder.filter(f);
    }
    let resp = client
        .search_points(builder)
        .await
        .with_context(|| format!("qdrant search failed for collection={collection}"))?;
    Ok(resp.result)
}

pub fn scored_point_payload_value(point: &ScoredPoint) -> Value {
    let payload: Payload = point.payload.clone().into();
    payload.into()
}

pub fn scored_point_payload_map(point: &ScoredPoint) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let payload = scored_point_payload_value(point);
    if let Some(obj) = payload.as_object() {
        for (k, v) in obj {
            let value = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.insert(k.clone(), value);
        }
    }
    out
}
