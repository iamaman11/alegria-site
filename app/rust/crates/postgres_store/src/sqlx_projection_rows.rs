//! PostgreSQL-owned read models for Neo4j materialization. No graph, HTTP,
//! browser or application dependency belongs on this side of the boundary.

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};

pub async fn load_rule_instance_materialization_row(
    pool: &PgPool,
    rule_instance_id: &str,
) -> Result<Option<(String, String, String, String, String, String)>> {
    let row = sqlx::query(
        r#"
        SELECT
          rule_instance_id,
          context_key,
          rule_type_key,
          concept_key,
          role_type,
          status
        FROM verified.rule_instances
        WHERE rule_instance_id = $1
        "#,
    )
    .bind(rule_instance_id)
    .fetch_optional(pool)
    .await
    .context("load materialization rule row")?;

    Ok(row.map(|r| {
        (
            r.get("rule_instance_id"),
            r.get("context_key"),
            r.get("rule_type_key"),
            r.get("concept_key"),
            r.get("role_type"),
            r.get("status"),
        )
    }))
}

pub async fn load_concept_materialization_row(
    pool: &PgPool,
    concept_key: &str,
) -> Result<Option<(String, String, String, String)>> {
    let row = sqlx::query(
        r#"
        SELECT concept_key, concept_type, COALESCE(label_ru, concept_key) AS label_ru, status
        FROM kb.concepts
        WHERE concept_key = $1
        "#,
    )
    .bind(concept_key)
    .fetch_optional(pool)
    .await
    .context("load materialization concept row")?;
    Ok(row.map(|r| {
        (
            r.get("concept_key"),
            r.get("concept_type"),
            r.get("label_ru"),
            r.get("status"),
        )
    }))
}

pub async fn load_page_context_materialization_row(
    pool: &PgPool,
    url_path: &str,
) -> Result<Option<(String, String, String)>> {
    let row = sqlx::query(
        r#"
        SELECT url_path, context_key, mapping_status
        FROM site.page_context_map
        WHERE url_path = $1
        "#,
    )
    .bind(url_path)
    .fetch_optional(pool)
    .await
    .context("load materialization page context row")?;
    Ok(row.map(|r| {
        (
            r.get("url_path"),
            r.get("context_key"),
            r.get("mapping_status"),
        )
    }))
}
