//! Concrete Voyage/Qdrant/PostgreSQL/Outbox effects for the Knowledge
//! projection of globally verified Truth. SQL metadata and delivery intent
//! must commit atomically; Qdrant stays a rebuildable projection.

use async_trait::async_trait;
use primitives::{errors::DomainError, qdrant_point_id::qdrant_point_id_v1};
use seo_ports::{
    knowledge::{VerifiedRuleEmbeddingBatch, VerifiedRuleProjectionChange},
    VerifiedRuleProjectionPort,
};
use sqlx::PgPool;

use super::{
    qdrant_client_adapter, sqlx_outbox_adapter, sqlx_runtime_outbox_adapter, sqlx_seo_adapter,
    voyage_api_adapter::{
        VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype,
    },
};

const COLLECTION: &str = "verified_rules_4";

pub struct VerifiedRuleProjectionAdapter<'a> {
    pool: &'a PgPool,
}

impl<'a> VerifiedRuleProjectionAdapter<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }
}

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

#[async_trait]
impl VerifiedRuleProjectionPort for VerifiedRuleProjectionAdapter<'_> {
    fn retrieval_required(&self) -> bool {
        env_flag("RETRIEVAL_CAPABILITY_REQUIRED")
    }

    async fn delete_rule_points(
        &self,
        run_id: &str,
        rule_ids: &[String],
    ) -> Result<(), DomainError> {
        if rule_ids.is_empty() {
            return Ok(());
        }
        if self.retrieval_required() {
            let url =
                std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
            let client = qdrant_client_adapter::connect_qdrant(&url)
                .await
                .map_err(|err| stage_error("connect_qdrant_delete", run_id, err))?;
            let point_ids = rule_ids
                .iter()
                .map(|id| qdrant_point_id_v1(COLLECTION, "rule_instance", id))
                .collect();
            qdrant_client_adapter::delete_point_ids(&client, COLLECTION, point_ids)
                .await
                .map_err(|err| stage_error("delete_qdrant", run_id, err))?;
        }
        sqlx_seo_adapter::delete_qdrant_rule_points(self.pool, COLLECTION, rule_ids)
            .await
            .map_err(|err| stage_error("delete_registry", run_id, err))
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

    async fn stage_rule_upserts(
        &self,
        change: &VerifiedRuleProjectionChange,
        embeddings: VerifiedRuleEmbeddingBatch,
    ) -> Result<(), DomainError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| stage_error("begin", &change.run_id, err))?;
        let mut events = Vec::with_capacity(change.upserts.len());
        for (candidate, vector) in change.upserts.iter().zip(embeddings.vectors) {
            let event = seo_steps::outbox_builder::qdrant_rule_upsert(
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
            let point_id =
                qdrant_point_id_v1(COLLECTION, "rule_instance", &candidate.rule_instance_id);
            sqlx_seo_adapter::upsert_qdrant_rule_point_in_tx(
                &mut tx,
                &point_id,
                &candidate.rule_instance_id,
                COLLECTION,
                &embeddings.model,
                "verified_rules_4@1",
            )
            .await?;
        }
        sqlx_runtime_outbox_adapter::outbox_emit_many_in_tx(&mut tx, &events).await?;
        tx.commit()
            .await
            .map_err(|err| stage_error("commit", &change.run_id, err))?;
        Ok(())
    }
}
