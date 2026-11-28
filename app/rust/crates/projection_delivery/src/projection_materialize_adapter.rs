use std::env;

use contracts::generated::alegria::sync::v1::{
    Neo4jRuleUpsertPayload, QdrantDeleteCommand, QdrantEntityPayload, QdrantUpsertCommand,
    SeoCmsEventPayload, SeoGraphProjectionPayload,
};
use postgres_store::sqlx_outbox_adapter::{
    dispatch_verified_rule_with_fence, OutboxEvent, VerifiedRuleDispatchError,
};
use primitives::errors::{DomainError, ErrorClass};
use primitives::qdrant_point_id::{is_valid_qdrant_point_id, qdrant_point_id_v1};
use prost::Message;
use sqlx::PgPool;

use super::neo4j_materialization_adapter;
use vector_qdrant::projection_commands::{
    delete_projection, upsert_projection, ProjectionDelete, ProjectionError, ProjectionIdentity,
    ProjectionUpsert,
};
use voyage_api::{VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype};

const DEFAULT_VOYAGE_MODEL: &str = "voyage-4-large";
const VOYAGE4_COLLECTIONS: [&str; 3] = [
    "kb_canonical_4",
    "editorial_topics_4",
    "seo_keyword_clusters_4",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchStage {
    Contract,
    Embedding,
    Registry,
    /// External outcome may still complete late; never automatically retry.
    Quarantine,
    Qdrant,
    Neo4j,
}

#[derive(Debug)]
pub struct DispatchError {
    pub class: ErrorClass,
    pub stage: DispatchStage,
    pub detail: String,
}

impl DispatchError {
    fn contract(detail: impl std::fmt::Display) -> Self {
        Self {
            class: ErrorClass::ContractViolation,
            stage: DispatchStage::Contract,
            detail: detail.to_string(),
        }
    }

    fn external(stage: DispatchStage, detail: impl std::fmt::Display) -> Self {
        Self {
            class: ErrorClass::InfraUnavailable,
            stage,
            // Preserve the source chain from anyhow adapters (for example the
            // HTTP connector and OS error), not just the outer context.
            detail: format!("{detail:#}"),
        }
    }

    pub fn into_domain_error(self) -> DomainError {
        match self.class {
            ErrorClass::ContractViolation => DomainError::ContractViolation {
                message: self.to_string(),
            },
            _ => DomainError::InfraUnavailable {
                message: self.to_string(),
            },
        }
    }
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dispatch {:?} ({}) failed: {}",
            self.stage,
            self.class.as_str(),
            self.detail
        )
    }
}

impl std::error::Error for DispatchError {}

fn qdrant_error(error: ProjectionError) -> DispatchError {
    match error {
        ProjectionError::Contract(detail) => DispatchError::contract(detail),
        ProjectionError::Provider(error) => DispatchError::external(DispatchStage::Qdrant, error),
    }
}

fn requires_voyage4_projection(collection_name: &str) -> bool {
    VOYAGE4_COLLECTIONS.contains(&collection_name)
}

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

async fn dispatch_qdrant_upsert(payload_bytes: &[u8]) -> Result<(), DispatchError> {
    let mut cmd = QdrantUpsertCommand::decode(payload_bytes).map_err(DispatchError::contract)?;
    // Provider identity and vector shape belong to Qdrant's typed boundary;
    // SQLx/Outbox still own the durable ordering and stale-event fences.
    let identity = ProjectionIdentity::new(&cmd.collection_name, &cmd.entity_type, &cmd.entity_key)
        .map_err(qdrant_error)?;
    let collection_name = identity.collection_name();
    if requires_voyage4_projection(collection_name)
        && cmd
            .metadata
            .get("embedding_model")
            .map(|value| value == "deterministic-fingerprint")
            .unwrap_or(false)
    {
        let retrieval_text = cmd
            .metadata
            .get("retrieval_text")
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if retrieval_text.is_empty() {
            return Err(DispatchError::contract(format!(
                "{collection_name} projection requires retrieval_text for Voyage materialization"
            )));
        }
        let api_key = env::var("VOYAGE_API_KEY").map_err(DispatchError::contract)?;
        let model = env::var("VOYAGE_MODEL").unwrap_or_else(|_| DEFAULT_VOYAGE_MODEL.to_string());
        let voyage = VoyageClient::new(api_key, model.clone());
        let vector = voyage
            .embed_batch_with_settings(
                &[retrieval_text],
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Document),
                    output_dimension: Some(1024),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(false),
                },
            )
            .await
            .map_err(|err| DispatchError::external(DispatchStage::Embedding, err))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                DispatchError::external(DispatchStage::Embedding, "Voyage returned no embedding")
            })?;
        cmd.vector = vector;
        cmd.vector_size = 1024;
        cmd.metadata.insert("embedding_model".to_string(), model);
        cmd.metadata.insert(
            "embedding_version".to_string(),
            format!("{collection_name}@voyage4_document_1024_float"),
        );
    }
    if cmd.vector.is_empty() {
        let retrieval_text = cmd
            .metadata
            .get("retrieval_text")
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if retrieval_text.is_empty() {
            return Err(DispatchError::contract(
                "QdrantUpsertCommand.vector is empty",
            ));
        }
        let api_key = env::var("VOYAGE_API_KEY").map_err(DispatchError::contract)?;
        let model = env::var("VOYAGE_MODEL").unwrap_or_else(|_| DEFAULT_VOYAGE_MODEL.to_string());
        let voyage = VoyageClient::new(api_key, model.clone());
        let vector = voyage
            .embed_batch_with_settings(
                &[retrieval_text],
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Document),
                    output_dimension: Some(1024),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(false),
                },
            )
            .await
            .map_err(|err| DispatchError::external(DispatchStage::Embedding, err))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                DispatchError::external(
                    DispatchStage::Embedding,
                    "Voyage returned no backfill embedding",
                )
            })?;
        cmd.vector = vector;
        cmd.vector_size = 1024;
        cmd.metadata.insert("embedding_model".to_string(), model);
        cmd.metadata.insert(
            "embedding_version".to_string(),
            format!("{collection_name}@voyage4_document_1024_float_backfill"),
        );
    }

    let point_id = if cmd.point_id.is_empty() {
        qdrant_point_id_v1(
            collection_name,
            identity.entity_type(),
            identity.entity_key(),
        )
    } else if is_valid_qdrant_point_id(&cmd.point_id) {
        cmd.point_id.clone()
    } else {
        qdrant_point_id_v1(
            collection_name,
            identity.entity_type(),
            identity.entity_key(),
        )
    };
    let entity_payload_value = qdrant_payload_value(cmd.payload, cmd.metadata);
    let projection = ProjectionUpsert::new(
        identity,
        point_id,
        cmd.vector,
        cmd.vector_size as u64,
        &cmd.distance,
        entity_payload_value,
    )
    .map_err(qdrant_error)?;
    upsert_projection(projection).await.map_err(qdrant_error)?;
    Ok(())
}

async fn dispatch_qdrant_delete(payload_bytes: &[u8]) -> Result<(), DispatchError> {
    let cmd = QdrantDeleteCommand::decode(payload_bytes).map_err(DispatchError::contract)?;
    if cmd.collection_name != "verified_rules_4" || cmd.entity_type != "rule_instance" {
        return Err(DispatchError::contract(
            "QdrantDeleteCommand only supports verified rule projection tombstones",
        ));
    }
    if cmd.entity_key.trim().is_empty() {
        return Err(DispatchError::contract(
            "QdrantDeleteCommand.entity_key is empty",
        ));
    }
    let expected_point_id =
        qdrant_point_id_v1(&cmd.collection_name, &cmd.entity_type, &cmd.entity_key);
    if cmd.point_id != expected_point_id {
        return Err(DispatchError::contract(
            "QdrantDeleteCommand.point_id does not match canonical identity",
        ));
    }
    let projection =
        ProjectionDelete::new(cmd.collection_name, expected_point_id).map_err(qdrant_error)?;
    delete_projection(projection).await.map_err(qdrant_error)
}

/// An ordinary provider event needs a caller-owned Outbox ACK. Verified-rule
/// events instead commit their ACK inside the PostgreSQL provider fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDisposition {
    NeedsAck,
    AcknowledgedByFence,
}

/// Production Outbox dispatch must be tied to an actual leased event. A
/// verified-rule provider RPC runs only after the PostgreSQL owner committed
/// its uncertainty barrier and while its advisory lock remains held. Never
/// re-enter `dispatch_event` from the fenced effect: that legacy API refuses
/// verified rules precisely to prevent bypass and self-locking.
pub async fn dispatch_leased_event(
    pool: &PgPool,
    event: &OutboxEvent,
) -> Result<DispatchDisposition, DispatchError> {
    if event.target_system == "qdrant" {
        let verified_rule = match event.event_type.as_str() {
            "QdrantUpsertCommand" => {
                let cmd = QdrantUpsertCommand::decode(event.payload_bytes.as_slice())
                    .map_err(DispatchError::contract)?;
                let any_verified_identity = cmd.collection_name == "verified_rules_4"
                    || cmd.entity_type == "rule_instance"
                    || event.aggregate_type == "rule_instance";
                if any_verified_identity
                    && !(cmd.collection_name == "verified_rules_4"
                        && cmd.entity_type == "rule_instance"
                        && event.aggregate_type == "rule_instance")
                {
                    return Err(DispatchError::contract(
                        "verified-rule Qdrant identity must match the leased Outbox aggregate",
                    ));
                }
                any_verified_identity
            }
            "QdrantDeleteCommand" => true,
            _ => false,
        };
        if verified_rule {
            let aggregate_key = &event.aggregate_key;
            let payload = event.payload_bytes.as_slice();
            // Point identity is validated before registering the effect with
            // the PostgreSQL owner. No network call is possible on mismatch.
            match event.event_type.as_str() {
                "QdrantUpsertCommand" => {
                    let cmd =
                        QdrantUpsertCommand::decode(payload).map_err(DispatchError::contract)?;
                    if cmd.entity_key.as_str() != aggregate_key {
                        return Err(DispatchError::contract(
                            "QdrantUpsertCommand aggregate_key does not match rule identity",
                        ));
                    }
                }
                "QdrantDeleteCommand" => {
                    let cmd =
                        QdrantDeleteCommand::decode(payload).map_err(DispatchError::contract)?;
                    if cmd.entity_key.as_str() != aggregate_key {
                        return Err(DispatchError::contract(
                            "QdrantDeleteCommand aggregate_key does not match rule identity",
                        ));
                    }
                }
                _ => unreachable!("verified rule type checked above"),
            }
            dispatch_verified_rule_with_fence(pool, event, || async {
                match event.event_type.as_str() {
                    "QdrantUpsertCommand" => dispatch_qdrant_upsert(payload).await,
                    "QdrantDeleteCommand" => dispatch_qdrant_delete(payload).await,
                    _ => unreachable!("verified rule type checked above"),
                }
            })
            .await
            .map_err(|err| match err {
                VerifiedRuleDispatchError::UnknownOutcome(source) => {
                    DispatchError::external(DispatchStage::Quarantine, source)
                }
                blocked @ VerifiedRuleDispatchError::BlockedUncertain { .. } => {
                    DispatchError::external(DispatchStage::Quarantine, blocked)
                }
                invalid @ VerifiedRuleDispatchError::InvalidEvent(_) => {
                    DispatchError::contract(invalid)
                }
                other => DispatchError::external(DispatchStage::Registry, other),
            })?;
            return Ok(DispatchDisposition::AcknowledgedByFence);
        }
    }

    dispatch_event(
        &event.target_system,
        &event.event_type,
        &event.aggregate_key,
        &event.payload_type,
        &event.payload_bytes,
    )
    .await?;
    Ok(DispatchDisposition::NeedsAck)
}

async fn dispatch_seo_graph_projection(
    aggregate_key: &str,
    payload_bytes: &[u8],
) -> Result<(), DispatchError> {
    let payload =
        SeoGraphProjectionPayload::decode(payload_bytes).map_err(DispatchError::contract)?;
    let artifact_key = if payload.artifact_key.is_empty() {
        aggregate_key
    } else {
        &payload.artifact_key
    };
    neo4j_materialization_adapter::materialize_seo_artifact(&payload.artifact_type, artifact_key)
        .await
        .map_err(|err| DispatchError::external(DispatchStage::Neo4j, err))
}

async fn dispatch_neo4j_rule_upsert(
    aggregate_key: &str,
    payload_bytes: &[u8],
) -> Result<(), DispatchError> {
    let payload = Neo4jRuleUpsertPayload::decode(payload_bytes).map_err(DispatchError::contract)?;
    let rule_instance_id = if payload.rule_instance_id.is_empty() {
        aggregate_key
    } else {
        &payload.rule_instance_id
    };
    neo4j_materialization_adapter::materialize_rule_instance(rule_instance_id)
        .await
        .map_err(|err| DispatchError::external(DispatchStage::Neo4j, err))
}

fn dispatch_cms_event(payload_bytes: &[u8]) -> Result<(), DispatchError> {
    let payload = SeoCmsEventPayload::decode(payload_bytes).map_err(DispatchError::contract)?;
    if payload.event_key.trim().is_empty() {
        return Err(DispatchError::contract(
            "SeoCmsEventPayload.event_key is empty",
        ));
    }
    if payload.page_node_key.trim().is_empty() {
        return Err(DispatchError::contract(
            "SeoCmsEventPayload.page_node_key is empty",
        ));
    }
    if payload.revision_id.trim().is_empty() {
        return Err(DispatchError::contract(
            "SeoCmsEventPayload.revision_id is empty",
        ));
    }
    Ok(())
}

pub async fn dispatch_event(
    target_system: &str,
    event_type: &str,
    aggregate_key: &str,
    payload_type: &str,
    payload_bytes: &[u8],
) -> Result<(), DispatchError> {
    if target_system == "qdrant" {
        return match event_type {
            "QdrantUpsertCommand" => {
                let cmd =
                    QdrantUpsertCommand::decode(payload_bytes).map_err(DispatchError::contract)?;
                if cmd.collection_name == "verified_rules_4" || cmd.entity_type == "rule_instance" {
                    if cmd.entity_key != aggregate_key {
                        return Err(DispatchError::contract(
                            "QdrantUpsertCommand aggregate_key does not match rule identity",
                        ));
                    }
                    Err(DispatchError::contract(
                        "verified-rule Qdrant RPC requires a PostgreSQL leased Outbox fence",
                    ))
                } else {
                    dispatch_qdrant_upsert(payload_bytes).await
                }
            }
            "QdrantDeleteCommand" => {
                if payload_type != "alegria.outbox.qdrant_delete_command.v1" {
                    return Err(DispatchError::contract(
                        "unsupported QdrantDeleteCommand payload_type",
                    ));
                }
                let cmd =
                    QdrantDeleteCommand::decode(payload_bytes).map_err(DispatchError::contract)?;
                if cmd.entity_key != aggregate_key {
                    return Err(DispatchError::contract(
                        "QdrantDeleteCommand aggregate_key does not match rule identity",
                    ));
                }
                Err(DispatchError::contract(
                    "verified-rule Qdrant delete requires a PostgreSQL leased Outbox fence",
                ))
            }
            _ => Err(DispatchError::contract(format!(
                "unsupported qdrant event_type: {event_type}"
            ))),
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
            _ => Err(DispatchError::contract(format!(
                "unsupported cms event_type: {event_type}"
            ))),
        };
    }

    if target_system != "neo4j" {
        return Err(DispatchError::contract(format!(
            "unsupported target_system: {target_system}"
        )));
    }

    match event_type {
        "RuleInstanceUpserted" => {
            if payload_type == "alegria.outbox.neo4j_rule_upserted.v1" {
                dispatch_neo4j_rule_upsert(aggregate_key, payload_bytes).await
            } else {
                neo4j_materialization_adapter::materialize_rule_instance(aggregate_key)
                    .await
                    .map_err(|err| DispatchError::external(DispatchStage::Neo4j, err))
            }
        }
        "ConceptApproved" => neo4j_materialization_adapter::materialize_concept(aggregate_key)
            .await
            .map_err(|err| DispatchError::external(DispatchStage::Neo4j, err)),
        "PageContextUpserted" => {
            neo4j_materialization_adapter::materialize_page_context(aggregate_key)
                .await
                .map_err(|err| DispatchError::external(DispatchStage::Neo4j, err))
        }
        "SeoGraphProjectionUpserted" => {
            if payload_type == "alegria.outbox.seo_graph_projection.v1" {
                dispatch_seo_graph_projection(aggregate_key, payload_bytes).await
            } else {
                Err(DispatchError::contract(format!(
                    "unsupported SEO graph payload_type: {payload_type}"
                )))
            }
        }
        _ => Err(DispatchError::contract(format!(
            "unsupported event_type: {event_type}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_is_not_inferred_from_detail_text() {
        let invalid = DispatchError::contract("timeout connecting to remote");
        assert_eq!(invalid.class, ErrorClass::ContractViolation);
        let offline = DispatchError::external(DispatchStage::Qdrant, "invalid payload");
        assert_eq!(offline.class, ErrorClass::InfraUnavailable);
        assert_eq!(offline.stage, DispatchStage::Qdrant);
    }

    #[test]
    fn qdrant_typed_owner_preserves_contract_and_provider_error_classes() {
        let invalid = ProjectionIdentity::new("", "rule_instance", "fact-1")
            .expect_err("invalid projection must fail before provider IO");
        let mapped = qdrant_error(invalid);
        assert_eq!(mapped.class, ErrorClass::ContractViolation);
        assert_eq!(mapped.stage, DispatchStage::Contract);

        let offline = qdrant_error(ProjectionError::Provider(anyhow::anyhow!(
            "upstream request failed"
        )));
        assert_eq!(offline.class, ErrorClass::InfraUnavailable);
        assert_eq!(offline.stage, DispatchStage::Qdrant);
    }
}
