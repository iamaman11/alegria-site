use std::collections::BTreeMap;

use sqlx::PgPool;

use super::proto_runtime_payload_store::{classify_sqlx, neo4j_rule_upserted, validation_failure};
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;
use primitives::concept_key::normalize_concept_key;
use primitives::errors::DomainError;
use primitives::stable_id::stable_rule_instance_id;
use runtime_models::{PersistPipelineState, SourceRegistryRecord};

mod rows {
    #[derive(Debug)]
    pub(super) struct SourceRegistryRow {
        pub(super) source_key: String,
        pub(super) source_type: String,
        pub(super) authority_class: String,
        pub(super) independence_group_key: String,
        pub(super) trust_level: i32,
        pub(super) freshness_ttl_days: i32,
        pub(super) override_eligible: bool,
    }
}

mod queries {
    use super::rows::SourceRegistryRow;
    use sqlx::{PgPool, Row};

    pub(super) async fn fetch_active_sources(
        pool: &PgPool,
    ) -> Result<Vec<SourceRegistryRow>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT source_key, source_type, authority_class, independence_group_key,
                   trust_level, freshness_ttl_days, override_eligible
            FROM kb.sources
            WHERE status = 'active'
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SourceRegistryRow {
                source_key: r.get("source_key"),
                source_type: r.get("source_type"),
                authority_class: r.get("authority_class"),
                independence_group_key: r.get("independence_group_key"),
                trust_level: r.get("trust_level"),
                freshness_ttl_days: r.get("freshness_ttl_days"),
                override_eligible: r.get("override_eligible"),
            })
            .collect())
    }
}

mod commands {
    use sqlx::types::Json;
    use sqlx::PgPool;

    pub(super) async fn upsert_rule_instance(
        pool: &PgPool,
        rule_instance_id: &str,
        context_key: &str,
        rule_type_key: &str,
        concept_key: &str,
        role_type: &str,
        params: serde_json::Value,
        status: &str,
        source_key: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO verified.rule_instances
            (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (rule_instance_id) DO UPDATE
            SET rule_type_key = EXCLUDED.rule_type_key,
                concept_key   = EXCLUDED.concept_key,
                role_type     = EXCLUDED.role_type,
                params        = EXCLUDED.params,
                status        = EXCLUDED.status,
                source_key    = EXCLUDED.source_key,
                updated_at    = now()
            "#,
            rule_instance_id,
            context_key,
            rule_type_key,
            concept_key,
            role_type,
            Json(params) as _,
            status,
            source_key
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub async fn load_source_registry_entries(
    pool: &PgPool,
) -> std::result::Result<BTreeMap<String, SourceRegistryRecord>, DomainError> {
    let rows = queries::fetch_active_sources(pool)
        .await
        .map_err(classify_sqlx)?;

    let mut registry = BTreeMap::new();
    for row in rows {
        let key = row.source_key;
        let source_type = row.source_type;
        let authority_class = row.authority_class;
        let independence_group_key = row.independence_group_key;
        let trust_level = row.trust_level;
        let freshness_ttl_days = row.freshness_ttl_days;
        let override_eligible = row.override_eligible;
        let effective_trust = if source_type.eq_ignore_ascii_case("government") {
            10
        } else {
            trust_level
        };
        registry.insert(
            key,
            SourceRegistryRecord {
                source_type,
                trust_level: effective_trust as i64,
                authority_class,
                independence_group_key,
                freshness_ttl_days,
                override_eligible,
            },
        );
    }
    Ok(registry)
}

pub async fn persist_from_pipeline_state(
    pool: &PgPool,
    run_id: &str,
    state: &PersistPipelineState,
) -> std::result::Result<(usize, usize), DomainError> {
    let context_key = state.context_key.clone();
    let rule_instances = &state.extracted_payload.rule_instances;

    let mut written = 0usize;
    let mut outbox_events: Vec<OutboxEnvelope> = Vec::with_capacity(rule_instances.len());

    for rule in rule_instances {
        let rule_type_key = rule.rule_type_key.clone();
        let concept_key_raw = rule.concept_key.as_str();
        let concept_key = normalize_concept_key(concept_key_raw).map_err(|e| {
            validation_failure(format!("invalid concept_key in rule {:?}: {e}", rule))
        })?;
        let role_type = rule.role_type.as_str().to_string();
        let params = rule.params.as_json_value();
        let status = rule.status.clone();
        let source_key = rule.source_key.clone();
        if status == "verified" && source_key.is_none() {
            return Err(validation_failure(format!(
                "verified rule `{rule_type_key}` in context `{context_key}` is missing source provenance"
            )));
        }

        let rule_instance_id =
            stable_rule_instance_id(&[&context_key, &rule_type_key, &concept_key, &role_type]);

        commands::upsert_rule_instance(
            pool,
            &rule_instance_id,
            &context_key,
            &rule_type_key,
            &concept_key,
            &role_type,
            params,
            &status,
            source_key.as_deref(),
        )
        .await
        .map_err(classify_sqlx)?;

        written += 1;
        let mut event = neo4j_rule_upserted(&rule_instance_id, &context_key);
        event.run_id = run_id.to_string();
        outbox_events.push(event);
    }

    let outbox_count = outbox_emit_many(pool, &outbox_events).await? as usize;
    Ok((written, outbox_count))
}
