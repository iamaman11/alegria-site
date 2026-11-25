//! Typed ownership of one Qdrant outbox projection, independent of SQLx and
//! the protobuf delivery envelope. PostgreSQL retains ordering, fencing and
//! commit authority; this module validates and performs vector provider I/O.

use std::env;
use std::fmt;

use anyhow::Error;
use serde_json::Value;

use super::{
    connect_qdrant, delete_point_ids, ensure_dense_collection, normalize_payload, parse_distance,
    upsert_dense_point, validate_dense_vector,
};

const DEFAULT_QDRANT_URL: &str = "http://localhost:6334";

/// Keep contract violations separate from retriable provider failures. The
/// outbox dispatcher maps these variants to its existing error classes.
#[derive(Debug)]
pub enum ProjectionError {
    Contract(String),
    Provider(Error),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(detail) => write!(f, "Qdrant projection contract: {detail}"),
            Self::Provider(error) => write!(f, "Qdrant projection provider: {error:#}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionIdentity {
    collection_name: String,
    entity_type: String,
    entity_key: String,
}

impl ProjectionIdentity {
    pub fn new(
        collection_name: &str,
        entity_type: &str,
        entity_key: &str,
    ) -> Result<Self, ProjectionError> {
        let collection_name = collection_name.trim();
        let entity_type = entity_type.trim();
        let entity_key = entity_key.trim();
        if collection_name.is_empty() {
            return Err(ProjectionError::Contract(
                "QdrantUpsertCommand.collection_name is empty".into(),
            ));
        }
        if entity_type.is_empty() {
            return Err(ProjectionError::Contract(
                "QdrantUpsertCommand.entity_type is empty".into(),
            ));
        }
        if entity_key.is_empty() {
            return Err(ProjectionError::Contract(
                "QdrantUpsertCommand.entity_key is empty".into(),
            ));
        }
        Ok(Self {
            collection_name: collection_name.to_string(),
            entity_type: entity_type.to_string(),
            entity_key: entity_key.to_string(),
        })
    }

    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    pub fn entity_type(&self) -> &str {
        &self.entity_type
    }

    pub fn entity_key(&self) -> &str {
        &self.entity_key
    }
}

#[derive(Debug)]
pub struct ProjectionUpsert {
    identity: ProjectionIdentity,
    point_id: String,
    vector: Vec<f32>,
    vector_size: u64,
    distance: super::Distance,
    payload: Value,
}

impl ProjectionUpsert {
    pub fn new(
        identity: ProjectionIdentity,
        point_id: String,
        vector: Vec<f32>,
        declared_vector_size: u64,
        distance: &str,
        payload: Value,
    ) -> Result<Self, ProjectionError> {
        if point_id.trim().is_empty() {
            return Err(ProjectionError::Contract(
                "QdrantUpsertCommand.point_id is empty".into(),
            ));
        }
        validate_dense_vector(&vector)
            .map_err(|error| ProjectionError::Contract(error.to_string()))?;
        let vector_size = if declared_vector_size == 0 {
            vector.len() as u64
        } else {
            declared_vector_size
        };
        if vector_size != vector.len() as u64 {
            return Err(ProjectionError::Contract(format!(
                "QdrantUpsertCommand.vector_size mismatch: declared={}, actual={}",
                vector_size,
                vector.len()
            )));
        }
        let distance = parse_distance(if distance.is_empty() {
            "cosine"
        } else {
            distance
        })
        .map_err(|error| ProjectionError::Contract(error.to_string()))?;
        Ok(Self {
            identity,
            point_id,
            vector,
            vector_size,
            distance,
            payload,
        })
    }
}

/// The Outbox's PG transaction/fence is resolved before entering this owner.
pub async fn upsert_projection(command: ProjectionUpsert) -> Result<(), ProjectionError> {
    let url = env::var("QDRANT_URL").unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
    let client = connect_qdrant(&url)
        .await
        .map_err(ProjectionError::Provider)?;
    ensure_dense_collection(
        &client,
        &command.identity.collection_name,
        command.vector_size,
        command.distance,
    )
    .await
    .map_err(ProjectionError::Provider)?;
    let payload = normalize_payload(
        &command.identity.entity_type,
        &command.identity.entity_key,
        Some(&command.payload),
    );
    upsert_dense_point(
        &client,
        &command.identity.collection_name,
        &command.point_id,
        command.vector,
        payload,
    )
    .await
    .map_err(ProjectionError::Provider)
}

/// Read the *complete* externally stored business projection, not merely a
/// successful HTTP upsert response or a SQL receipt. This is also the only
/// allowed recovery check after the RPC outcome becomes uncertain (for
/// example Qdrant accepted a write but the PostgreSQL receipt failed).
///
/// A missing/mismatched point is NOT permission to send a second RPC: a
/// delayed first write could still arrive after this read.
fn cosine_vector_matches(expected: &[f32], actual: &[Value]) -> bool {
    // Qdrant stores normalized unit vectors for Cosine collections. Comparing
    // to the submitted unnormalized f32 values would reject every legitimate
    // readback. The approved command SHA-256 stays in the *exact* payload;
    // all coordinates must independently match the provider's canonical
    // normalized direction within its f32 arithmetic tolerance.
    let norm_squared: f64 = expected.iter().map(|v| (*v as f64).powi(2)).sum();
    let norm = norm_squared.sqrt();
    norm.is_finite()
        && norm > 0.0
        && actual.len() == expected.len()
        && actual.iter().zip(expected.iter()).all(|(got, want)| {
            let normalized = *want as f64 / norm;
            got.as_f64().map(|value| (value - normalized).abs() <= 0.00001)
                == Some(true)
        })
}

pub async fn projection_readback_exact(
    collection: &str,
    point_id: &str,
    entity_type: &str,
    entity_key: &str,
    vector: &[f32],
    metadata: &Value,
) -> Result<bool, ProjectionError> {
    let base = env::var("QDRANT_REST_URL")
        .or_else(|_| env::var("QDRANT_URL").map(|url| url.replace(":6334", ":6333")))
        .map_err(|_| ProjectionError::Contract("QDRANT_REST_URL is required for exact readback".into()))?;
    let url = format!(
        "{}/collections/{collection}/points",
        base.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(10))
        .json(&serde_json::json!({
            "ids": [point_id], "with_payload": true, "with_vector": true
        }))
        .send()
        .await
        .map_err(|err| ProjectionError::Provider(err.into()))?;
    if !response.status().is_success() {
        return Err(ProjectionError::Provider(anyhow::anyhow!(
            "Qdrant exact projection readback HTTP {}",
            response.status()
        )));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|err| ProjectionError::Provider(err.into()))?;
    let points = body
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| ProjectionError::Provider(anyhow::anyhow!(
            "Qdrant exact readback omitted result array"
        )))?;
    if points.len() != 1 || points[0].get("id").and_then(Value::as_str) != Some(point_id) {
        return Ok(false);
    }
    let expected_payload = normalize_payload(entity_type, entity_key, Some(metadata));
    if points[0].get("payload") != Some(&expected_payload) {
        return Ok(false);
    }
    let Some(actual_vector) = points[0].get("vector").and_then(Value::as_array) else {
        return Ok(false);
    };
    Ok(cosine_vector_matches(vector, actual_vector))
}

#[derive(Debug)]
pub struct ProjectionDelete {
    collection_name: String,
    point_id: String,
}

impl ProjectionDelete {
    pub fn new(collection_name: String, point_id: String) -> Result<Self, ProjectionError> {
        if collection_name.trim().is_empty() || point_id.trim().is_empty() {
            return Err(ProjectionError::Contract(
                "Qdrant delete requires collection and point ID".into(),
            ));
        }
        Ok(Self {
            collection_name,
            point_id,
        })
    }
}

pub async fn delete_projection(command: ProjectionDelete) -> Result<(), ProjectionError> {
    let url = env::var("QDRANT_URL").unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
    let client = connect_qdrant(&url)
        .await
        .map_err(ProjectionError::Provider)?;
    // No collection after a retry/demotion is a valid idempotent tombstone.
    if !client
        .collection_exists(&command.collection_name)
        .await
        .map_err(|error| ProjectionError::Provider(error.into()))?
    {
        return Ok(());
    }
    delete_point_ids(&client, &command.collection_name, vec![command.point_id])
        .await
        .map_err(ProjectionError::Provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_readback_checks_every_normalized_coordinate() {
        let submitted = [3.0_f32, 4.0_f32];
        assert!(cosine_vector_matches(&submitted, &[
            serde_json::json!(0.6), serde_json::json!(0.8)
        ]));
        assert!(!cosine_vector_matches(&submitted, &[
            serde_json::json!(0.6), serde_json::json!(0.79)
        ]));
        assert!(!cosine_vector_matches(&submitted, &[
            serde_json::json!(0.6)
        ]));
        assert!(!cosine_vector_matches(&[0.0, 0.0], &[
            serde_json::json!(0.0), serde_json::json!(0.0)
        ]));
    }

    fn identity() -> ProjectionIdentity {
        ProjectionIdentity::new(" verified_rules_4 ", " rule_instance ", " rule-1 ").unwrap()
    }

    #[test]
    fn identity_validated_and_normalized_without_provider_io() {
        let good = identity();
        assert_eq!(good.collection_name, "verified_rules_4");
        assert_eq!(good.entity_type, "rule_instance");
        assert_eq!(good.entity_key, "rule-1");
        for (collection, kind, key) in [
            ("", "rule_instance", "id"),
            ("kb", "", "id"),
            ("kb", "kind", " "),
        ] {
            assert!(matches!(
                ProjectionIdentity::new(collection, kind, key),
                Err(ProjectionError::Contract(_))
            ));
        }
    }

    #[test]
    fn no_invalid_vector_reaches_qdrant_or_voyage() {
        let make = |vector, size, distance| {
            ProjectionUpsert::new(
                identity(),
                "11111111-1111-4111-8111-111111111111".to_string(),
                vector,
                size,
                distance,
                serde_json::json!({"rule_instance_id":"rule-1"}),
            )
        };
        assert!(matches!(
            make(vec![], 0, "cosine"),
            Err(ProjectionError::Contract(_))
        ));
        assert!(matches!(
            make(vec![0.1, 0.2], 3, "cosine"),
            Err(ProjectionError::Contract(_))
        ));
        assert!(matches!(
            make(vec![0.1, 0.2], 2, "unsupported"),
            Err(ProjectionError::Contract(_))
        ));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                make(vec![0.1, bad], 2, "cosine"),
                Err(ProjectionError::Contract(_))
            ));
        }
        assert!(make(vec![0.1, 0.2], 0, "").is_ok());
    }

    #[test]
    fn empty_tombstone_identity_rejected_before_io() {
        assert!(matches!(
            ProjectionDelete::new("".into(), "id".into()),
            Err(ProjectionError::Contract(_))
        ));
        assert!(matches!(
            ProjectionDelete::new("collection".into(), " ".into()),
            Err(ProjectionError::Contract(_))
        ));
    }
}
