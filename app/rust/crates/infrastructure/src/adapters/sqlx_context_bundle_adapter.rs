use anyhow::Context;
use primitives::errors::DomainError;
use runtime_models::RuleParams;
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct ContextBundleBaseRow {
    pub country_code: String,
    pub visa_family: String,
    pub visa_subtype: String,
    pub citizenship_code: String,
}

#[derive(Debug, Clone)]
pub struct ContextBundleRuleRow {
    pub rule_instance_id: String,
    pub rule_type_key: String,
    pub role_type: String,
    pub params: RuleParams,
    pub status: String,
    pub effective_from: String,
    pub effective_to: String,
    pub source_key: Option<String>,
    pub source_label: Option<String>,
    pub base_url: Option<String>,
}

fn infra_unavailable(message: impl Into<String>) -> DomainError {
    DomainError::InfraUnavailable {
        message: message.into(),
    }
}

pub async fn load_context_bundle_base(
    pool: &PgPool,
    context_key: &str,
) -> std::result::Result<Option<ContextBundleBaseRow>, DomainError> {
    let ctx_row = sqlx::query(
        r#"
        SELECT country_code, visa_family, visa_subtype, citizenship_code
        FROM kb.visa_contexts
        WHERE context_key = $1 AND status = 'active'
        "#,
    )
    .bind(context_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;

    Ok(ctx_row.map(|ctx| ContextBundleBaseRow {
        country_code: ctx.get("country_code"),
        visa_family: ctx.get("visa_family"),
        visa_subtype: ctx
            .get::<Option<String>, _>("visa_subtype")
            .unwrap_or_default(),
        citizenship_code: ctx.get("citizenship_code"),
    }))
}

pub async fn load_context_bundle_rules(
    pool: &PgPool,
    context_key: &str,
) -> std::result::Result<Vec<ContextBundleRuleRow>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id,
            r.rule_type_key,
            r.role_type,
            r.params->>'severity' as severity,
            r.params->>'subtype' as subtype,
            r.params->>'conditions_key' as conditions_key,
            r.params->>'currency' as currency,
            r.params->>'channel' as channel,
            r.params->>'location_key' as location_key,
            r.params->>'form_id' as form_id,
            (r.params->>'amount')::double precision as amount,
            (r.params->>'days')::bigint as days,
            (r.params->>'advance_days')::integer as advance_days,
            (r.params->>'step_index')::integer as step_index,
            (r.params->>'notarization_required')::boolean as notarization_required,
            (r.params->>'translation_required')::boolean as translation_required,
            (r.params->>'accepts_alternatives')::boolean as accepts_alternatives,
            r.status,
            r.effective_from::text as effective_from,
            r.effective_to::text as effective_to,
            s.source_key,
            s.source_label,
            s.base_url
        FROM verified.rule_instances r
        LEFT JOIN kb.sources s ON r.source_key = s.source_key
        WHERE r.context_key = $1
        "#,
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let role_type = r.get::<Option<String>, _>("role_type").unwrap_or_default();
            let severity = r.get::<Option<String>, _>("severity").unwrap_or_default();
            let subtype = r.get::<Option<String>, _>("subtype");
            let conditions_key = r
                .get::<Option<String>, _>("conditions_key")
                .unwrap_or_default();
            let params = match role_type.as_str() {
                "document_required" | "form_required" => RuleParams::Document {
                    severity,
                    subtype,
                    notarization_required: r
                        .get::<Option<bool>, _>("notarization_required")
                        .unwrap_or(false),
                    translation_required: r
                        .get::<Option<bool>, _>("translation_required")
                        .unwrap_or(false),
                    accepts_alternatives: r
                        .get::<Option<bool>, _>("accepts_alternatives")
                        .unwrap_or(false),
                    conditions_key,
                },
                "fee_item" | "must_pay" => RuleParams::Fee {
                    amount: r.get::<Option<f64>, _>("amount").unwrap_or(0.0),
                    currency: r
                        .get::<Option<String>, _>("currency")
                        .unwrap_or_else(|| "EUR".to_string()),
                    severity,
                    channel: r.get::<Option<String>, _>("channel"),
                    conditions_key,
                },
                "timeline_item" | "timeline" => RuleParams::Timeline {
                    days: r.get::<Option<i64>, _>("days").unwrap_or_default(),
                    subtype,
                    severity,
                    conditions_key,
                },
                "where_to_apply" => RuleParams::WhereToApply {
                    location_key: r
                        .get::<Option<String>, _>("location_key")
                        .unwrap_or_default(),
                    channel: r.get::<Option<String>, _>("channel").unwrap_or_default(),
                    conditions_key,
                },
                "eligibility_rule" => RuleParams::EligibilityRule {
                    subtype: subtype.unwrap_or_default(),
                    severity,
                    conditions_key,
                },
                "appointment_rule" => RuleParams::AppointmentRule {
                    subtype: subtype.unwrap_or_default(),
                    advance_days: r.get::<Option<i32>, _>("advance_days").unwrap_or_default(),
                    conditions_key,
                },
                "step" => RuleParams::Step {
                    step_index: r.get::<Option<i32>, _>("step_index").unwrap_or_default(),
                    subtype: subtype.unwrap_or_default(),
                    conditions_key,
                },
                _ => RuleParams::None,
            };

            ContextBundleRuleRow {
                rule_instance_id: r.get("rule_instance_id"),
                rule_type_key: r.get("rule_type_key"),
                role_type,
                params,
                status: r.get("status"),
                effective_from: r
                    .get::<Option<String>, _>("effective_from")
                    .unwrap_or_default(),
                effective_to: r
                    .get::<Option<String>, _>("effective_to")
                    .unwrap_or_default(),
                source_key: r.get::<Option<String>, _>("source_key"),
                source_label: r.get::<Option<String>, _>("source_label"),
                base_url: r.get::<Option<String>, _>("base_url"),
            }
        })
        .collect())
}

pub async fn load_rule_instance_materialization_row(
    pool: &PgPool,
    rule_instance_id: &str,
) -> anyhow::Result<Option<(String, String, String, String, String, String)>> {
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
) -> anyhow::Result<Option<(String, String, String, String)>> {
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
) -> anyhow::Result<Option<(String, String, String)>> {
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
