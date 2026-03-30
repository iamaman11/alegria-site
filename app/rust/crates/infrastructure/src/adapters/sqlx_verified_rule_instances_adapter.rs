use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct VerifiedRuleRow {
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
    pool: &PgPool,
    context_key: &str,
) -> Result<Vec<VerifiedRuleRow>> {
    let rows = sqlx::query(
        "SELECT rule_instance_id, context_key, rule_type_key, title, description, params::text AS params_text, status, updated_at
         FROM verified.rule_instances
         WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .context("read verified rules")?;

    Ok(rows
        .into_iter()
        .map(|row| VerifiedRuleRow {
            rule_instance_id: row.get("rule_instance_id"),
            context_key: row.get("context_key"),
            rule_type_key: row.get("rule_type_key"),
            title: row.get("title"),
            description: row.get("description"),
            params_json_utf8: row.get::<String, _>("params_text").into_bytes(),
            status: row.get("status"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn load_embedding_text_rows(
    pool: &PgPool,
    context_key: &str,
) -> Result<Vec<(String, String)>> {
    sqlx::query_as(
        "SELECT rule_instance_id::text, title || ' ' || coalesce(description, '')
         FROM verified.rule_instances
         WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .context("load embedding text rows")
}
