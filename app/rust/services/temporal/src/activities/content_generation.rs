use std::collections::BTreeSet;

use contracts::generated::alegria::temporal::v1::{
    ValidationDiagnostic as TemporalValidationDiagnostic, ValidationReport,
};
use infrastructure::adapters::editorial_block_validation_adapter;
use infrastructure::adapters::seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter as pipeline_storage;
use primitives::errors::{DomainError, ErrorClass};
use seo_application::knowledge;
use seo_ports::knowledge::VerifiedRulesQuery;

use super::AlegriaActivities;

pub(crate) async fn generate_content_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let context_key = &run.context_key;
    let repo = SqlxSeoRuntimeRepository::new(&acts.pool);
    let rendered_blocks = knowledge::generate_verified_blocks(
        &repo,
        &VerifiedRulesQuery {
            run_id: run_id.to_string(),
            context_key: context_key.clone(),
        },
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

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

    let generation_result =
        pipeline_storage::decode_generation_result(run.generation_result.as_ref());
    let extracted_payload =
        pipeline_storage::decode_extracted_payload(run.extracted_payload.as_ref());
    let rules_json = pipeline_storage::extracted_payload_rules_json(&extracted_payload);
    let facts_json = pipeline_storage::extracted_payload_facts_json(&extracted_payload);

    let input_payload = pipeline_storage::decode_validation_input(run.input_payload.as_ref());
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
        let diag_envelope = editorial_block_validation_adapter::validate_block_boundary_typed(
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
    pipeline_storage::advance_execution_run(
        &acts.pool, run_id, "done", None, None, None, None, None,
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;
    Ok(run_id.to_string())
}
