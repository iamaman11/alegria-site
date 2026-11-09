use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct VerifiedRuleRow {
    pub rule_instance_id: String,
    pub context_key: String,
    pub rule_type_key: String,
    pub title: String,
    pub description: Option<String>,
    pub params_json_utf8: Vec<u8>,
    pub status: String,
    pub updated_at: DateTime<Utc>,
}

pub async fn read_verified_rules(pool: &PgPool, context_key: &str) -> Result<Vec<VerifiedRuleRow>> {
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id::text AS rule_instance_id,
            r.context_key,
            r.rule_type_key,
            COALESCE(c.label_ru, c.concept_key, r.concept_key) AS title,
            concat_ws(
                ' ',
                r.role_type,
                r.params::text,
                COALESCE(s.source_label, '')
            ) AS description,
            r.params::text AS params_text,
            r.status,
            r.updated_at
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1 AND r.status = 'verified'
          AND r.publish_admissibility = 'admissible'
          AND NOT EXISTS (
              SELECT 1 FROM verified.rule_instance_profiles rp
              WHERE rp.rule_instance_id = r.rule_instance_id
          )
          AND (
              r.rule_candidate_id IS NULL
              OR EXISTS (
                  SELECT 1 FROM extracted.rule_candidates c
                  WHERE c.rule_candidate_id = r.rule_candidate_id
                    AND c.scope = '{}'::jsonb
                    AND c.applies_to_profiles = '[]'::jsonb
                    AND btrim(c.exceptions_raw) = ''
                    AND btrim(c.conditions_raw) = ''
                    AND c.alternatives = '[]'::jsonb
                    AND c.epistemic_status IN ('verified', 'structured')
              )
          )
        "#,
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
        r#"
        SELECT
            r.rule_instance_id::text,
            concat_ws(
                ' ',
                COALESCE(c.label_ru, c.concept_key, r.concept_key),
                r.role_type,
                r.params::text,
                COALESCE(s.source_label, '')
            )
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1 AND r.status = 'verified'
          AND r.publish_admissibility = 'admissible'
          AND NOT EXISTS (
              SELECT 1 FROM verified.rule_instance_profiles rp
              WHERE rp.rule_instance_id = r.rule_instance_id
          )
          AND (
              r.rule_candidate_id IS NULL
              OR EXISTS (
                  SELECT 1 FROM extracted.rule_candidates c
                  WHERE c.rule_candidate_id = r.rule_candidate_id
                    AND c.scope = '{}'::jsonb
                    AND c.applies_to_profiles = '[]'::jsonb
                    AND btrim(c.exceptions_raw) = ''
                    AND btrim(c.conditions_raw) = ''
                    AND c.alternatives = '[]'::jsonb
                    AND c.epistemic_status IN ('verified', 'structured')
              )
          )
        "#,
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .context("load embedding text rows")
}
