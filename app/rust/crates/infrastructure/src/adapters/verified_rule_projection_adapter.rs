//! Concrete Voyage/Qdrant/PostgreSQL/Outbox effects for the Knowledge
//! projection of globally verified Truth. SQL metadata and delivery intent
//! must commit atomically; Qdrant stays a rebuildable projection.

use async_trait::async_trait;
use primitives::{errors::DomainError, qdrant_point_id::qdrant_point_id_v1};
use seo_ports::{
    knowledge::{VerifiedRuleEmbeddingBatch, VerifiedRuleUpsertBatch},
    VerifiedRuleEmbeddingPort,
};
use sqlx::{Postgres, Transaction};

use super::{
    sqlx_outbox_adapter, sqlx_runtime_outbox_adapter, sqlx_seo_adapter,
    voyage_api_adapter::{
        VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype,
    },
};

const COLLECTION: &str = "verified_rules_4";

pub struct VerifiedRuleEmbeddingAdapter;

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn stage_error(stage: &str, run_id: &str, error: impl std::fmt::Display) -> DomainError {
    DomainError::InfraUnavailable {
        message: format!("knowledge.project_verified_rules stage={stage} run_id={run_id}: {error}"),
    }
}

async fn lock_projection_keys_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &str,
    rule_ids: &[String],
) -> Result<(), DomainError> {
    for rule_id in rule_ids.iter().collect::<std::collections::BTreeSet<_>>() {
        let key = format!("qdrant|verified_rules_4|rule_instance|{rule_id}");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(&key)
            .execute(&mut **tx)
            .await
            .map_err(|err| stage_error("lock_projection", run_id, err))?;
    }
    Ok(())
}

/// Stage a complete Truth projection tombstone in the caller's SQL transaction.
/// A failure inserting the event rolls back the registry deletion (and any
/// earlier canonical Truth mutations), without touching external Qdrant.
pub async fn stage_rule_point_deletes_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &str,
    _context_key: &str,
    rule_ids: &[String],
) -> Result<(), DomainError> {
    if rule_ids.is_empty() {
        return Ok(());
    }
    let ids = rule_ids
        .iter()
        .filter(|id| !id.trim().is_empty())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let ordered_ids = ids.iter().cloned().collect::<Vec<_>>();
    lock_projection_keys_in_tx(tx, run_id, &ordered_ids).await?;
    let events = ids
        .iter()
        .map(|rule_id| {
            let event = seo_steps::outbox_builder::qdrant_rule_delete(run_id, rule_id, COLLECTION);
            sqlx_outbox_adapter::OutboxEnvelope {
                run_id: run_id.to_string(),
                aggregate_type: event.aggregate_type,
                aggregate_key: event.aggregate_key,
                target_system: event.target_system,
                event_type: event.event_type,
                payload_type: event.payload_type,
                schema_version: event.schema_version,
                idempotency_key: event.idempotency_key,
                payload_bytes: event.payload_bytes,
            }
        })
        .collect::<Vec<_>>();
    sqlx::query(
        "DELETE FROM kb.qdrant_points
         WHERE collection_name = $1
           AND entity_type = 'rule_instance'
           AND entity_key = ANY($2)",
    )
    .bind(COLLECTION)
    .bind(&ordered_ids)
    .execute(&mut **tx)
    .await
    .map_err(|err| stage_error("delete_registry", run_id, err))?;
    sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(tx, &events).await?;
    Ok(())
}

/// Called after Voyage embeddings have been prepared, but while the same
/// canonical Truth transaction is still open. Outbox and registry are written
/// together; the external Qdrant write remains asynchronous after commit.
pub async fn stage_rule_upserts_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    change: &VerifiedRuleUpsertBatch,
    embeddings: VerifiedRuleEmbeddingBatch,
) -> Result<(), DomainError> {
    let ids = change
        .upserts
        .iter()
        .map(|candidate| candidate.rule_instance_id.clone())
        .collect::<Vec<_>>();
    lock_projection_keys_in_tx(tx, &change.run_id, &ids).await?;
    let mut events = Vec::with_capacity(change.upserts.len());
    for (candidate, vector) in change.upserts.iter().zip(embeddings.vectors) {
        let event = seo_steps::outbox_builder::qdrant_rule_upsert(
            &change.run_id,
            &candidate.rule_instance_id,
            &change.context_key,
            &candidate.role_type,
            &candidate.concept_key,
            &candidate.role_type,
            &candidate.source_key,
            COLLECTION,
            vector.iter().map(|value| *value as f64).collect(),
        );
        events.push(sqlx_outbox_adapter::OutboxEnvelope {
            run_id: change.run_id.clone(),
            aggregate_type: event.aggregate_type,
            aggregate_key: event.aggregate_key,
            target_system: event.target_system,
            event_type: event.event_type,
            payload_type: event.payload_type,
            schema_version: event.schema_version,
            idempotency_key: event.idempotency_key,
            payload_bytes: event.payload_bytes,
        });
        let point_id = qdrant_point_id_v1(COLLECTION, "rule_instance", &candidate.rule_instance_id);
        sqlx_seo_adapter::upsert_qdrant_rule_point_in_tx(
            tx,
            &point_id,
            &candidate.rule_instance_id,
            COLLECTION,
            &embeddings.model,
            "verified_rules_4@1",
        )
        .await?;
    }
    sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(tx, &events).await?;
    Ok(())
}

#[async_trait]
impl VerifiedRuleEmbeddingPort for VerifiedRuleEmbeddingAdapter {
    fn retrieval_required(&self) -> bool {
        env_flag("RETRIEVAL_CAPABILITY_REQUIRED")
    }

    async fn embed_rule_texts(
        &self,
        run_id: &str,
        texts: &[String],
    ) -> Result<Option<VerifiedRuleEmbeddingBatch>, DomainError> {
        let key = match std::env::var("VOYAGE_API_KEY") {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let model = std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
        let voyage = VoyageClient::new(key, model.clone());
        let vectors = voyage
            .embed_all_with_settings(
                texts,
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Document),
                    output_dimension: Some(1024),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(false),
                },
            )
            .await
            .map_err(|err| stage_error("embed", run_id, err))?;
        Ok(Some(VerifiedRuleEmbeddingBatch { model, vectors }))
    }
}
