use anyhow::Result;
use chrono::{DateTime, Utc};
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::sqlx_verified_rule_instances_adapter;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RuleInstanceRow {
    pub rule_instance_id: Uuid,
    pub context_key: String,
    pub rule_type_key: String,
    pub title: String,
    pub description: Option<String>,
    pub params_json_utf8: Vec<u8>,
    pub status: String,
    pub updated_at: DateTime<Utc>,
}

pub async fn read_verified_rules(
    pool: &AlegriaPgPool,
    context_key: &str,
) -> Result<Vec<RuleInstanceRow>> {
    let rows = sqlx_verified_rule_instances_adapter::read_verified_rules(pool, context_key).await?;

    Ok(rows
        .into_iter()
        .map(|row| RuleInstanceRow {
            rule_instance_id: row.rule_instance_id,
            context_key: row.context_key,
            rule_type_key: row.rule_type_key,
            title: row.title,
            description: row.description,
            params_json_utf8: row.params_json_utf8,
            status: row.status,
            updated_at: row.updated_at,
        })
        .collect())
}
