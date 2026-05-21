use std::env;

use anyhow::{bail, Context, Result};
use contracts::generated::alegria::sync::v1::{
    Neo4jRuleUpsertPayload, QdrantEntityPayload, QdrantUpsertCommand, SeoCmsEventPayload,
    SeoGraphProjectionPayload,
};
use prost::Message;
use primitives::qdrant_point_id::{is_valid_qdrant_point_id, qdrant_point_id_v1};

use super::neo4j_materialization_adapter;
use super::qdrant_client_adapter::{
    connect_qdrant, ensure_dense_collection, normalize_payload, parse_distance, upsert_dense_point,
};

const DEFAULT_QDRANT_URL: &str = "http://localhost:6334";

fn qdrant_payload_value(
    payload: Option<QdrantEntityPayload>,
    metadata: std::collections::HashMap<String, String>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(payload) = payload {
        map.insert(
            "rule_instance_id".to_string(),
            serde_json::Value::String(payload.rule_instance_id),
        );
        map.insert(
            "context_key".to_string(),
            serde_json::Value::String(payload.context_key),
        );
        map.insert(
            "rule_type_key".to_string(),
            serde_json::Value::String(payload.rule_type_key),
        );
        map.insert(
            "concept_key".to_string(),
            serde_json::Value::String(payload.concept_key),
        );
        map.insert(
            "role_type".to_string(),
            serde_json::json!(payload.role_type),
        );
        map.insert(
            "source_key".to_string(),
            serde_json::Value::String(payload.source_key),
        );
    }
    for (key, value) in metadata {
        map.insert(key, serde_json::Value::String(value));
    }
    serde_json::Value::Object(map)
}

async fn dispatch_qdrant_upsert(payload_bytes: &[u8]) -> Result<()> {
    let cmd = QdrantUpsertCommand::decode(payload_bytes)
        .context("invalid protobuf payload for QdrantUpsertCommand")?;

    let collection_name = cmd.collection_name.trim();
    if collection_name.is_empty() {
        bail!("QdrantUpsertCommand.collection_name is empty");
    }
    let entity_type = cmd.entity_type.trim();
    if entity_type.is_empty() {
        bail!("QdrantUpsertCommand.entity_type is empty");
    }
    let entity_key = cmd.entity_key.trim();
    if entity_key.is_empty() {
        bail!("QdrantUpsertCommand.entity_key is empty");
    }
    if cmd.vector.is_empty() {
        bail!("QdrantUpsertCommand.vector is empty");
    }

    let vector_size = if cmd.vector_size == 0 {
        cmd.vector.len() as u64
    } else {
        cmd.vector_size as u64
    };
    if vector_size != cmd.vector.len() as u64 {
        bail!(
            "QdrantUpsertCommand.vector_size mismatch: declared={}, actual={}",
            vector_size,
            cmd.vector.len()
        );
    }
    let distance = parse_distance(if cmd.distance.is_empty() {
        "cosine"
    } else {
        &cmd.distance
    })?;
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
    let point_id = if cmd.point_id.is_empty() {
        qdrant_point_id_v1(collection_name, entity_type, entity_key)
    } else if is_valid_qdrant_point_id(&cmd.point_id) {
        cmd.point_id.clone()
    } else {
        qdrant_point_id_v1(collection_name, entity_type, entity_key)
    };

    let client = connect_qdrant(&qdrant_url)
        .await
        .with_context(|| format!("failed to connect qdrant: {qdrant_url}"))?;
    ensure_dense_collection(&client, collection_name, vector_size, distance).await?;
    let entity_payload_value = qdrant_payload_value(cmd.payload, cmd.metadata);
    let merged_payload = normalize_payload(entity_type, entity_key, Some(&entity_payload_value));
    upsert_dense_point(
        &client,
        collection_name,
        &point_id,
        cmd.vector,
        merged_payload,
    )
    .await?;
    Ok(())
}

async fn dispatch_seo_graph_projection(aggregate_key: &str, payload_bytes: &[u8]) -> Result<()> {
    let payload = SeoGraphProjectionPayload::decode(payload_bytes)
        .context("invalid protobuf payload for SeoGraphProjectionPayload")?;
    let artifact_key = if payload.artifact_key.is_empty() {
        aggregate_key
    } else {
        &payload.artifact_key
    };
    neo4j_materialization_adapter::materialize_seo_artifact(&payload.artifact_type, artifact_key)
        .await
}

async fn dispatch_neo4j_rule_upsert(aggregate_key: &str, payload_bytes: &[u8]) -> Result<()> {
    let payload = Neo4jRuleUpsertPayload::decode(payload_bytes)
        .context("invalid protobuf payload for RuleInstanceUpserted")?;
    let rule_instance_id = if payload.rule_instance_id.is_empty() {
        aggregate_key
    } else {
        &payload.rule_instance_id
    };
    neo4j_materialization_adapter::materialize_rule_instance(rule_instance_id).await
}

fn dispatch_cms_event(payload_bytes: &[u8]) -> Result<()> {
    let payload = SeoCmsEventPayload::decode(payload_bytes)
        .context("invalid protobuf payload for SeoCmsEventPayload")?;
    if payload.event_key.trim().is_empty() {
        bail!("SeoCmsEventPayload.event_key is empty");
    }
    if payload.page_node_key.trim().is_empty() {
        bail!("SeoCmsEventPayload.page_node_key is empty");
    }
    if payload.revision_id.trim().is_empty() {
        bail!("SeoCmsEventPayload.revision_id is empty");
    }
    Ok(())
}

pub async fn dispatch_event(
    target_system: &str,
    event_type: &str,
    aggregate_key: &str,
    payload_type: &str,
    payload_bytes: &[u8],
) -> Result<()> {
    if target_system == "qdrant" {
        return match event_type {
            "QdrantUpsertCommand" => dispatch_qdrant_upsert(payload_bytes).await,
            _ => bail!("unsupported qdrant event_type: {event_type}"),
        };
    }
    if target_system == "cms" {
        return match event_type {
            "seo_page_review_requested"
            | "seo_page_approved"
            | "seo_page_publish_blocked"
            | "seo_page_published"
            | "seo_page_deprecated"
            | "seo_page_rollback_requested"
            | "seo_page_rolled_back"
            | "seo_page_rebuild_requested"
            | "seo_page_canonical_changed" => dispatch_cms_event(payload_bytes),
            _ => bail!("unsupported cms event_type: {event_type}"),
        };
    }

    match event_type {
        "RuleInstanceUpserted" => {
            if payload_type == "alegria.outbox.neo4j_rule_upserted.v1" {
                dispatch_neo4j_rule_upsert(aggregate_key, payload_bytes).await
            } else {
                neo4j_materialization_adapter::materialize_rule_instance(aggregate_key).await
            }
        }
        "ConceptApproved" => {
            neo4j_materialization_adapter::materialize_concept(aggregate_key).await
        }
        "PageContextUpserted" => {
            neo4j_materialization_adapter::materialize_page_context(aggregate_key).await
        }
        "SeoGraphProjectionUpserted" => {
            if payload_type == "alegria.outbox.seo_graph_projection.v1" {
                dispatch_seo_graph_projection(aggregate_key, payload_bytes).await
            } else {
                bail!("unsupported SEO graph payload_type: {payload_type}")
            }
        }
        _ => bail!("unsupported event_type: {event_type}"),
    }
}
