use std::env;

use anyhow::{bail, Context, Result};
use contracts::generated::alegria::sync::v1::{Neo4jRuleUpsertPayload, QdrantEntityPayload, QdrantUpsertCommand};
use prost::Message;

use infrastructure::adapters::qdrant_client_adapter::{
    connect_qdrant, ensure_dense_collection, normalize_payload, parse_distance, upsert_dense_point,
};

const DEFAULT_QDRANT_URL: &str = "http://localhost:6334";

fn qdrant_entity_payload_value(payload: Option<QdrantEntityPayload>) -> serde_json::Value {
    let Some(payload) = payload else {
        return serde_json::json!({});
    };
    serde_json::json!({
        "rule_instance_id": payload.rule_instance_id,
        "context_key": payload.context_key,
        "rule_type_key": payload.rule_type_key,
        "concept_key": payload.concept_key,
        "role_type": payload.role_type,
        "source_key": payload.source_key,
    })
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
    let distance = parse_distance(if cmd.distance.is_empty() { "cosine" } else { &cmd.distance })?;
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
    let point_id = if cmd.point_id.is_empty() {
        entity_key.to_string()
    } else {
        cmd.point_id.clone()
    };

    let client = connect_qdrant(&qdrant_url)
        .await
        .with_context(|| format!("failed to connect qdrant: {qdrant_url}"))?;
    ensure_dense_collection(&client, collection_name, vector_size, distance).await?;
    let entity_payload_value = qdrant_entity_payload_value(cmd.payload);
    let merged_payload = normalize_payload(
        entity_type,
        entity_key,
        Some(&entity_payload_value),
    );
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

async fn dispatch_neo4j_rule_upsert(aggregate_key: &str, payload_bytes: &[u8]) -> Result<()> {
    let payload = Neo4jRuleUpsertPayload::decode(payload_bytes)
        .context("invalid protobuf payload for RuleInstanceUpserted")?;
    let rule_instance_id = if payload.rule_instance_id.is_empty() {
        aggregate_key
    } else {
        &payload.rule_instance_id
    };
    use_cases::materialize_rule_instance::materialize_rule_instance(rule_instance_id).await
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

    match event_type {
        "RuleInstanceUpserted" => {
            if payload_type == "alegria.outbox.neo4j_rule_upserted.v1" {
                dispatch_neo4j_rule_upsert(aggregate_key, payload_bytes).await
            } else {
                use_cases::materialize_rule_instance::materialize_rule_instance(aggregate_key).await
            }
        }
        "ConceptApproved" => {
            use_cases::materialize_concept::materialize_concept(aggregate_key).await
        }
        "PageContextUpserted" => {
            use_cases::materialize_page_context::materialize_page_context(aggregate_key).await
        }
        _ => bail!("unsupported event_type: {event_type}"),
    }
}
