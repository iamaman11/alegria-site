use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::read_api::v1::{citation_fact, RuleRoleTypeV1};
use contracts::generated::alegria::temporal::v1::{
    ValidationDiagnostic as TemporalValidationDiagnostic, ValidationReport,
};
use primitives::errors::{DomainError, ErrorClass};
use use_cases::pipeline_runtime as pipeline_storage;

use super::AlegriaActivities;

pub(crate) async fn generate_content_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let context_key = run.context_key();
    let bundle =
        use_cases::assemble_context_bundle::assemble_context_bundle_model(&acts.pool, context_key)
            .await
            .map_err(AlegriaActivities::classify_error)?;

    let mut rendered_blocks: BTreeMap<String, String> = BTreeMap::new();
    let citation_rules: Vec<primitives::writer::CitationRule> = bundle
        .citation_facts
        .iter()
        .map(|fact| {
            let mut role = match RuleRoleTypeV1::try_from(fact.role_type).ok() {
                Some(RuleRoleTypeV1::DocumentRequired | RuleRoleTypeV1::MustProvide) => {
                    primitives::writer::RuleRole::DocumentRequired
                }
                Some(RuleRoleTypeV1::FormRequired) => primitives::writer::RuleRole::FormRequired,
                Some(RuleRoleTypeV1::FeeItem | RuleRoleTypeV1::MustPay) => {
                    primitives::writer::RuleRole::FeeItem
                }
                Some(RuleRoleTypeV1::TimelineItem | RuleRoleTypeV1::Timeline) => {
                    primitives::writer::RuleRole::TimelineItem
                }
                Some(_) => primitives::writer::RuleRole::Generic,
                None => primitives::writer::RuleRole::Unknown,
            };
            let mut params = primitives::writer::RuleParamsView::None;
            match fact.params.as_ref() {
                Some(citation_fact::Params::DocParams(v)) => {
                    role = primitives::writer::RuleRole::DocumentRequired;
                    params = primitives::writer::RuleParamsView::Document {
                        severity: v.severity.clone(),
                        subtype: if v.subtype.is_empty() {
                            None
                        } else {
                            Some(v.subtype.clone())
                        },
                        notarization_required: v.notarization_required,
                    };
                }
                Some(citation_fact::Params::FeeParams(v)) => {
                    role = primitives::writer::RuleRole::FeeItem;
                    params = primitives::writer::RuleParamsView::Fee {
                        amount: v.amount,
                        currency: if v.currency.is_empty() {
                            "EUR".to_string()
                        } else {
                            v.currency.clone()
                        },
                    };
                }
                Some(citation_fact::Params::TimelineParams(v)) => {
                    role = primitives::writer::RuleRole::TimelineItem;
                    params = primitives::writer::RuleParamsView::Timeline { days: v.days };
                }
                None => {}
                _ => role = primitives::writer::RuleRole::Generic,
            }

            primitives::writer::CitationRule {
                rule_instance_id: fact.rule_instance_id.clone(),
                rule_type_key: fact.rule_type_key.clone(),
                role,
                params,
                status: fact.status.clone(),
                effective_from: fact.effective_from.clone(),
                effective_to: fact.effective_to.clone(),
            }
        })
        .collect();
    let faq_items: Vec<primitives::writer::FaqItem> = Vec::new();
    for block_key in primitives::writer::plan_blocks(&citation_rules, &faq_items) {
        let html = primitives::writer::render_block(&block_key, &citation_rules, &faq_items);
        rendered_blocks.insert(block_key, html);
    }

    pipeline_storage::advance_execution_run_typed(
        &acts.pool,
        run_id,
        "generating",
        Some("generation_result"),
        Some(&rendered_blocks),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(run_id.to_string())
}

pub(crate) async fn validate_blocks_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let generation_result = pipeline_storage::decode_generation_result_from_run(&run);
    let extracted_payload = pipeline_storage::decode_extracted_payload_from_run(&run);
    let rules_json = pipeline_storage::extracted_payload_rules_json(&extracted_payload);
    let facts_json = pipeline_storage::extracted_payload_facts_json(&extracted_payload);

    let input_payload = pipeline_storage::decode_validation_input_from_run(&run);
    let required_links_json = input_payload.required_links_json;
    let required_keys_json = input_payload.required_keys_json;
    let used_rule_keys_json = input_payload.used_rule_keys_json;
    let used_fact_keys_json = input_payload.used_fact_keys_json;
    let url_norm = input_payload.url_norm;

    let blocks = AlegriaActivities::extract_block_htmls(&generation_result);
    if blocks.is_empty() {
        let (input_hash, idempotency_key) =
            pipeline_storage::derive_step_keys(run_id, "validate_blocks", 1, &run_id)
                .map_err(AlegriaActivities::classify_error)?;
        let report = ValidationReport {
            meta: Some(AlegriaActivities::build_meta(
                run_id,
                "validate_blocks",
                1,
                &input_hash,
                None,
                &idempotency_key,
                Some(ErrorClass::ValidationFailure),
            )),
            ok: false,
            critical_count: 1,
            warning_count: 0,
            info_count: 0,
            blocks_total: 0,
            diagnostics: vec![TemporalValidationDiagnostic {
                severity: "critical".to_string(),
                gate: "html_structure".to_string(),
                block_key: "__all__".to_string(),
                message: "generation_result is empty or has no renderable blocks".to_string(),
                context_json_utf8: b"{}".to_vec(),
            }],
        };
        let _ = pipeline_storage::advance_execution_run_typed(
            &acts.pool,
            run_id,
            "failed",
            Some("verify_report"),
            Some(&report),
        )
        .await;
        return Err(DomainError::ValidationFailure {
            message: "validate_blocks: empty generation_result".to_string(),
        });
    }

    let mut diagnostics: Vec<TemporalValidationDiagnostic> = Vec::new();
    for (block_key, html) in blocks {
        let diag_envelope = primitives::block_validator_json::validate_block_boundary_typed(
            &html,
            &rules_json,
            &facts_json,
            &required_links_json,
            &required_keys_json,
            &used_rule_keys_json,
            &used_fact_keys_json,
            &block_key,
            &url_norm,
        );
        if diag_envelope.diagnostics.is_empty() {
            diagnostics.push(TemporalValidationDiagnostic {
                severity: "warning".to_string(),
                gate: "validator_runtime".to_string(),
                block_key,
                message: "validator returned payload without diagnostics[]".to_string(),
                context_json_utf8: b"{}".to_vec(),
            });
        } else {
            diagnostics.extend(diag_envelope.diagnostics.into_iter().map(|d| {
                TemporalValidationDiagnostic {
                    severity: d.severity,
                    gate: d.gate,
                    block_key: d.block_key,
                    message: d.message,
                    context_json_utf8: d.context_json_utf8,
                }
            }));
        }
    }

    let mut critical_count = 0usize;
    let mut warning_count = 0usize;
    let mut info_count = 0usize;
    for d in &diagnostics {
        match d.severity.as_str() {
            "critical" => critical_count += 1,
            "warning" => warning_count += 1,
            _ => info_count += 1,
        }
    }

    let (input_hash, idempotency_key) =
        pipeline_storage::derive_step_keys(run_id, "validate_blocks", 1, &run_id)
            .map_err(AlegriaActivities::classify_error)?;
    let report = ValidationReport {
        meta: Some(AlegriaActivities::build_meta(
            run_id,
            "validate_blocks",
            1,
            &input_hash,
            None,
            &idempotency_key,
            None,
        )),
        ok: critical_count == 0,
        critical_count: critical_count as u32,
        warning_count: warning_count as u32,
        info_count: info_count as u32,
        blocks_total: diagnostics
            .iter()
            .map(|d| d.block_key.as_str())
            .collect::<BTreeSet<_>>()
            .len() as u32,
        diagnostics,
    };
    let status = if report.ok { "validating" } else { "failed" };

    pipeline_storage::advance_execution_run_typed(
        &acts.pool,
        run_id,
        status,
        Some("verify_report"),
        Some(&report),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    if !report.ok {
        return Err(DomainError::ValidationFailure {
            message: "validate_blocks: critical diagnostics present".to_string(),
        });
    }

    Ok(run_id.to_string())
}

pub(crate) async fn finalize_run_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    pipeline_storage::advance_execution_run_status(&acts.pool, run_id, "done")
        .await
        .map_err(AlegriaActivities::classify_error)?;
    Ok(run_id.to_string())
}
