use prost::Message;
use contracts::generated::alegria::read_api::v1::{
    citation_fact, AppointmentRuleParams, CitationFact, ContextBundle, DocumentRequiredParams,
    EligibilityRuleParams, FeeItemParams, FormRequiredParams, RuleRoleTypeV1, SourceCitation,
    StepParams, TimelineItemParams, WhereToApplyParams,
};
use infrastructure::adapters::sqlx_context_bundle_adapter;
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use primitives::errors::DomainError;
use runtime_models::RuleParams;

fn read_role_type(role_type: &str) -> i32 {
    match role_type {
        "must_provide" => RuleRoleTypeV1::MustProvide as i32,
        "must_pay" => RuleRoleTypeV1::MustPay as i32,
        "must_satisfy" => RuleRoleTypeV1::MustSatisfy as i32,
        "allows" => RuleRoleTypeV1::Allows as i32,
        "forbids" => RuleRoleTypeV1::Forbids as i32,
        "timeline" => RuleRoleTypeV1::Timeline as i32,
        "document_required" => RuleRoleTypeV1::DocumentRequired as i32,
        "eligibility_rule" => RuleRoleTypeV1::EligibilityRule as i32,
        "fee_item" => RuleRoleTypeV1::FeeItem as i32,
        "timeline_item" => RuleRoleTypeV1::TimelineItem as i32,
        "where_to_apply" => RuleRoleTypeV1::WhereToApply as i32,
        "appointment_rule" => RuleRoleTypeV1::AppointmentRule as i32,
        "form_required" => RuleRoleTypeV1::FormRequired as i32,
        "step" => RuleRoleTypeV1::Step as i32,
        _ => RuleRoleTypeV1::Unspecified as i32,
    }
}

fn map_params(params: &RuleParams) -> Option<citation_fact::Params> {
    match params {
        RuleParams::None => None,
        RuleParams::Document {
            severity,
            subtype,
            notarization_required,
            translation_required,
            accepts_alternatives,
            conditions_key,
        } => Some(citation_fact::Params::DocParams(DocumentRequiredParams {
            severity: severity.clone(),
            subtype: subtype.clone().unwrap_or_default(),
            notarization_required: *notarization_required,
            translation_required: *translation_required,
            accepts_alternatives: *accepts_alternatives,
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Fee {
            amount,
            currency,
            severity,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::FeeParams(FeeItemParams {
            amount: *amount,
            currency: currency.clone(),
            severity: severity.clone(),
            channel: channel.clone().unwrap_or_default(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Timeline {
            days,
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::TimelineParams(TimelineItemParams {
            days: *days,
            subtype: subtype.clone().unwrap_or_default(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::WhereToApply {
            location_key,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::WhereParams(WhereToApplyParams {
            location_key: location_key.clone(),
            channel: channel.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::EligibilityRule {
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::EligibilityParams(EligibilityRuleParams {
            subtype: subtype.clone(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::AppointmentRule {
            subtype,
            advance_days,
            conditions_key,
        } => Some(citation_fact::Params::AppointmentParams(AppointmentRuleParams {
            subtype: subtype.clone(),
            advance_days: *advance_days,
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::FormRequired {
            form_id,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::FormParams(FormRequiredParams {
            form_id: form_id.clone(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Step {
            step_index,
            subtype,
            conditions_key,
        } => Some(citation_fact::Params::StepParams(StepParams {
            step_index: *step_index,
            subtype: subtype.clone(),
            conditions_key: conditions_key.clone(),
        })),
    }
}

pub async fn assemble_context_bundle_model(
    pool: &AlegriaPgPool,
    context_key: &str,
) -> std::result::Result<ContextBundle, DomainError> {
    let Some(ctx) = sqlx_context_bundle_adapter::load_context_bundle_base(pool, context_key).await? else {
        return Ok(ContextBundle {
            context_key: context_key.to_string(),
            country_code: "".to_string(),
            visa_family: "".to_string(),
            visa_subtype: "".to_string(),
            citizenship_code: "".to_string(),
            ontology_rules: vec![],
            citation_facts: vec![],
            related_links: vec![],
        });
    };


    let rules_rows = sqlx_context_bundle_adapter::load_context_bundle_rules(pool, context_key).await?;

    let mut facts = Vec::new();
    for r in rules_rows {
        let citation = if let Some(sk) = r.source_key.clone() {
            Some(SourceCitation {
                source_key: sk,
                source_label: r.source_label.unwrap_or_default(),
                base_url: r.base_url.unwrap_or_default(),
            })
        } else {
            None
        };

        facts.push(CitationFact {
            rule_instance_id: r.rule_instance_id,
            rule_type_key: r.rule_type_key,
            status: r.status,
            effective_from: r.effective_from,
            effective_to: r.effective_to,
            citation,
            role_type: read_role_type(&r.role_type),
            params: map_params(&r.params),
        });
    }

    let bundle = ContextBundle {
        context_key: context_key.to_string(),
        country_code: ctx.country_code,
        visa_family: ctx.visa_family,
        visa_subtype: ctx.visa_subtype,
        citizenship_code: ctx.citizenship_code,
        ontology_rules: vec![],
        citation_facts: facts,
        related_links: vec![],
    };

    Ok(bundle)
}


pub async fn assemble_context_bundle(
    pool: &AlegriaPgPool,
    context_key: &str,
) -> std::result::Result<Vec<u8>, DomainError> {
    let bundle = assemble_context_bundle_model(pool, context_key).await?;
    let mut buf = Vec::new();
    bundle
        .encode(&mut buf)
        .map_err(|e| DomainError::ContractViolation { message: e.to_string() })?;
    Ok(buf)
}
