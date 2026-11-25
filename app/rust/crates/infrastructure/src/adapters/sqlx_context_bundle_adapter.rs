use primitives::errors::DomainError;
use runtime_models::RuleParams;
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct AdmissibleVerifiedRuleRow {
    pub rule_instance_id: String,
    pub rule_type_key: String,
    pub role_type: String,
    pub params: RuleParams,
    pub effective_from: String,
    pub effective_to: String,
    pub source_key: Option<String>,
}

fn infra_unavailable(message: impl Into<String>) -> DomainError {
    DomainError::InfraUnavailable {
        message: message.into(),
    }
}

fn incomplete_verified_param(rule_id: &str, field: &str) -> DomainError {
    DomainError::ContractViolation {
        message: format!(
            "knowledge.read_verified_rules stage=required_params rule_id={rule_id} missing {field}"
        ),
    }
}

pub async fn active_context_exists(
    pool: &PgPool,
    context_key: &str,
) -> std::result::Result<bool, DomainError> {
    sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM kb.visa_contexts
            WHERE context_key = $1 AND status = 'active'
        )
        "#,
    )
    .bind(context_key)
    .fetch_one(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))
}

pub async fn load_admissible_verified_rules(
    pool: &PgPool,
    context_key: &str,
) -> std::result::Result<Vec<AdmissibleVerifiedRuleRow>, DomainError> {
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
            r.effective_from::text as effective_from,
            r.effective_to::text as effective_to,
            r.source_key
        FROM verified.rule_instances r
        WHERE r.context_key = $1
          AND r.status = 'verified'
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
                    AND COALESCE(c.params->>'conditions_key', '') = ''
                    AND COALESCE(r.params->>'conditions_key', '') = ''
                    AND c.scope = '{}'::jsonb
                    AND c.applies_to_profiles = '[]'::jsonb
                    AND btrim(c.conditions_raw) = ''
                    AND btrim(c.exceptions_raw) = ''
                    AND c.alternatives = '[]'::jsonb
                    AND c.epistemic_status IN ('verified', 'structured')
              )
          )
        ORDER BY r.rule_instance_id
        "#,
    )
    .bind(context_key)
    .fetch_all(pool)
    .await
    .map_err(|e| infra_unavailable(e.to_string()))?;

    rows.into_iter()
        .map(|r| -> Result<AdmissibleVerifiedRuleRow, DomainError> {
            let rule_id: String = r.get("rule_instance_id");
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
                    amount: r
                        .get::<Option<f64>, _>("amount")
                        .ok_or_else(|| incomplete_verified_param(&rule_id, "amount"))?,
                    currency: r
                        .get::<Option<String>, _>("currency")
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| incomplete_verified_param(&rule_id, "currency"))?,
                    severity,
                    channel: r.get::<Option<String>, _>("channel"),
                    conditions_key,
                },
                "timeline_item" | "timeline" => RuleParams::Timeline {
                    days: r
                        .get::<Option<i64>, _>("days")
                        .ok_or_else(|| incomplete_verified_param(&rule_id, "days"))?,
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

            Ok(AdmissibleVerifiedRuleRow {
                rule_instance_id: rule_id,
                rule_type_key: r.get("rule_type_key"),
                role_type,
                params,
                effective_from: r
                    .get::<Option<String>, _>("effective_from")
                    .unwrap_or_default(),
                effective_to: r
                    .get::<Option<String>, _>("effective_to")
                    .unwrap_or_default(),
                source_key: r.get::<Option<String>, _>("source_key"),
            })
        })
        .collect::<Result<Vec<_>, DomainError>>()
}
