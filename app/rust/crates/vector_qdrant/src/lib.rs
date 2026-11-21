use anyhow::{bail, Context, Result};
pub use qdrant_client::qdrant::{
    Condition, CountPointsBuilder, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter,
    PointStruct, PointsIdsList, ScoredPoint, SearchPointsBuilder, UpsertPointsBuilder,
    VectorParamsBuilder,
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
    _client: &Qdrant,
    collection_name: &str,
    point_id: &str,
    vector: Vec<f32>,
    payload_value: Value,
) -> Result<()> {
    if vector.is_empty() {
        bail!("vector is empty");
    }
    let rest_url = qdrant_rest_url_from_env();
    let endpoint = format!(
        "{}/collections/{}/points?wait=true",
        rest_url.trim_end_matches('/'),
        collection_name
    );
    let body = serde_json::json!({
        "points": [{
            "id": point_id,
            "vector": vector,
            "payload": payload_value,
        }]
    });
    let response = reqwest::Client::new()
        .put(&endpoint)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed qdrant REST upsert request: {endpoint}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!(
            "failed qdrant REST upsert for collection={}, point_id={}, status={}, body={}",
            collection_name,
            point_id,
            status,
            text
        );
    }
    Ok(())
}

fn qdrant_rest_url_from_env() -> String {
    env::var("QDRANT_REST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let url =
                env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6333".to_string());
            if url.contains(":6334") {
                url.replace(":6334", ":6333")
            } else {
                url
            }
        })
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
    // Callers consume entity_key/retrieval_text to map neighbors and rerank;
    // Qdrant omits point payloads unless they are explicitly requested.
    let mut builder = SearchPointsBuilder::new(collection, vector, limit).with_payload(true);
    if let Some(f) = filter {
        builder = builder.filter(f);
    }
    let resp = client
        .search_points(builder)
        .await
        .with_context(|| format!("qdrant search failed for collection={collection}"))?;
    Ok(resp.result)
}

/// Expected immutable source evidence for one physical Qdrant point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunkReadbackPoint {
    pub point_id: String,
    pub section_id: String,
    pub content_hash: String,
}

fn raw_chunk_points_match(expected: &[RawChunkReadbackPoint], actual: &[Value]) -> bool {
    use std::collections::{BTreeMap, BTreeSet};
    if expected.is_empty() || expected.len() != actual.len() {
        return false;
    }
    let expected_by_id = expected
        .iter()
        .map(|point| (point.point_id.as_str(), point))
        .collect::<BTreeMap<_, _>>();
    if expected_by_id.len() != expected.len() {
        return false;
    }
    let mut seen = BTreeSet::new();
    for point in actual {
        let Some(id) = point.get("id").and_then(Value::as_str) else {
            return false;
        };
        let Some(want) = expected_by_id.get(id) else {
            return false;
        };
        if !seen.insert(id)
            || point.pointer("/payload/section_id").and_then(Value::as_str)
                != Some(want.section_id.as_str())
            || point
                .pointer("/payload/content_hash")
                .and_then(Value::as_str)
                != Some(want.content_hash.as_str())
        {
            return false;
        }
    }
    true
}

/// Compare exact point IDs, section IDs and content hashes against live Qdrant.
/// A collection's mere existence or old SQL timestamp cannot certify evidence.
pub async fn verify_raw_chunk_readback(
    collection: &str,
    expected: &[RawChunkReadbackPoint],
) -> Result<bool> {
    if expected.is_empty() {
        return Ok(false);
    }
    let url = format!(
        "{}/collections/{}/points",
        qdrant_rest_url_from_env().trim_end_matches('/'),
        collection
    );
    let client = reqwest::Client::new();
    // A retained corpus can span thousands of sections. Bound each provider
    // request, while still requiring an exact match for every expected point.
    for chunk in expected.chunks(128) {
        let ids = chunk
            .iter()
            .map(|point| point.point_id.as_str())
            .collect::<Vec<_>>();
        let response = client
            .post(&url)
            .timeout(std::time::Duration::from_secs(8))
            .json(&serde_json::json!({
                "ids": ids,
                "with_payload": ["section_id", "content_hash"],
                "with_vector": false
            }))
            .send()
            .await
            .with_context(|| format!("Qdrant raw evidence readback failed: {collection}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("Qdrant raw evidence readback HTTP {status}: {collection}");
        }
        let body: Value = response.json().await?;
        let actual = body
            .get("result")
            .and_then(Value::as_array)
            .context("Qdrant raw evidence readback missing points array")?;
        if !raw_chunk_points_match(chunk, actual) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod raw_chunk_readback_tests {
    use super::*;

    #[test]
    fn immutable_raw_points_require_exact_ids_and_source_hashes() {
        let expected = vec![RawChunkReadbackPoint {
            point_id: "point-1".into(),
            section_id: "1223".into(),
            content_hash: "source-hash".into(),
        }];
        let actual = serde_json::json!([{
            "id": "point-1",
            "payload": {"section_id": "1223", "content_hash": "source-hash"}
        }]);
        let good = actual.as_array().unwrap();
        assert!(raw_chunk_points_match(&expected, good));
        assert!(!raw_chunk_points_match(&[], good));
        assert!(!raw_chunk_points_match(&expected, &[]));
        assert!(!raw_chunk_points_match(
            &expected,
            &[serde_json::json!({
                "id": "point-1",
                "payload": {"section_id": "1223", "content_hash": "stale"}
            })]
        ));
        assert!(!raw_chunk_points_match(
            &expected,
            &[serde_json::json!({
                "id": "point-other",
                "payload": {"section_id": "1223", "content_hash": "source-hash"}
            })]
        ));
        assert!(!raw_chunk_points_match(
            &expected,
            &[serde_json::json!({
                "id": "point-1",
                "payload": {"section_id": "older", "content_hash": "source-hash"}
            })]
        ));
        let duplicated = vec![expected[0].clone(), expected[0].clone()];
        assert!(!raw_chunk_points_match(&duplicated, good));
        let two = vec![
            expected[0].clone(),
            RawChunkReadbackPoint {
                point_id: "point-2".into(),
                section_id: "1224".into(),
                content_hash: "other-hash".into(),
            },
        ];
        let both = serde_json::json!([
            {"id": "point-2", "payload": {"section_id": "1224", "content_hash": "other-hash"}},
            {"id": "point-1", "payload": {"section_id": "1223", "content_hash": "source-hash"}}
        ]);
        assert!(raw_chunk_points_match(&two, both.as_array().unwrap()));
        assert!(!raw_chunk_points_match(&two, good));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRuleReadbackPoint {
    pub point_id: String,
    pub rule_instance_id: String,
    pub context_key: String,
    pub concept_key: String,
    pub source_key: String,
}

fn verified_rule_points_match(expected: &[VerifiedRuleReadbackPoint], actual: &[Value]) -> bool {
    use std::collections::{BTreeMap, BTreeSet};
    if expected.is_empty() || expected.len() != actual.len() {
        return false;
    }
    let by_id = expected
        .iter()
        .map(|point| (point.point_id.as_str(), point))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != expected.len() {
        return false;
    }
    let mut seen = BTreeSet::new();
    for point in actual {
        let Some(id) = point.get("id").and_then(Value::as_str) else {
            return false;
        };
        let Some(want) = by_id.get(id) else {
            return false;
        };
        if !seen.insert(id)
            || point
                .pointer("/payload/rule_instance_id")
                .and_then(Value::as_str)
                != Some(want.rule_instance_id.as_str())
            || point
                .pointer("/payload/context_key")
                .and_then(Value::as_str)
                != Some(want.context_key.as_str())
            || point
                .pointer("/payload/concept_key")
                .and_then(Value::as_str)
                != Some(want.concept_key.as_str())
            || point.pointer("/payload/source_key").and_then(Value::as_str)
                != Some(want.source_key.as_str())
        {
            return false;
        }
    }
    true
}

/// Check the actual Qdrant fact point against the verified canonical SQL fact.
pub async fn verify_verified_rule_readback(expected: &[VerifiedRuleReadbackPoint]) -> Result<bool> {
    if expected.is_empty() {
        return Ok(false);
    }
    let url = format!(
        "{}/collections/verified_rules_4/points",
        qdrant_rest_url_from_env().trim_end_matches('/')
    );
    for chunk in expected.chunks(128) {
        let ids = chunk
            .iter()
            .map(|point| point.point_id.as_str())
            .collect::<Vec<_>>();
        let response = reqwest::Client::new()
            .post(&url)
            .timeout(std::time::Duration::from_secs(8))
            .json(&serde_json::json!({
                "ids": ids,
                "with_payload": ["rule_instance_id", "context_key", "concept_key", "source_key"],
                "with_vector": false
            }))
            .send()
            .await
            .context("Qdrant verified rule readback failed")?;
        let status = response.status();
        if !status.is_success() {
            bail!("Qdrant verified rule readback HTTP {status}");
        }
        let body: Value = response.json().await?;
        let actual = body
            .get("result")
            .and_then(Value::as_array)
            .context("Qdrant verified rule readback missing result array")?;
        if !verified_rule_points_match(chunk, actual) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod verified_rule_readback_tests {
    use super::*;
    #[test]
    fn verified_fact_requires_matching_live_source_context_and_id() {
        let want = vec![VerifiedRuleReadbackPoint {
            point_id: "point-a".into(),
            rule_instance_id: "fact-a".into(),
            context_key: "ES|tourist||BY".into(),
            concept_key: "standard_tourist_eligibility".into(),
            source_key: "https://official.example/rule".into(),
        }];
        let good = serde_json::json!([{"id":"point-a","payload":{
            "rule_instance_id":"fact-a", "context_key":"ES|tourist||BY",
            "concept_key":"standard_tourist_eligibility",
            "source_key":"https://official.example/rule"}}]);
        let actual = good.as_array().unwrap();
        assert!(verified_rule_points_match(&want, actual));
        assert!(!verified_rule_points_match(&[], actual));
        assert!(!verified_rule_points_match(&want, &[]));
        for field in [
            "id",
            "rule_instance_id",
            "context_key",
            "concept_key",
            "source_key",
        ] {
            let mut bad = actual.clone();
            let obj = bad[0].as_object_mut().unwrap();
            if field == "id" {
                obj.insert(field.into(), Value::String("wrong".into()));
            } else {
                obj.get_mut("payload")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(field.into(), Value::String("wrong".into()));
            }
            assert!(!verified_rule_points_match(&want, &bad), "{field}");
        }
        assert!(!verified_rule_points_match(
            &[want[0].clone(), want[0].clone()],
            actual
        ));
    }
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
