use std::collections::BTreeMap;

use contracts::generated::alegria::temporal::v1::{
    HitlPauseInfo, HitlResolutionInput, HitlTaskContext, VerifyReport,
};
use primitives::errors::DomainError;
use use_cases::pipeline_runtime as pipeline_storage;

use super::AlegriaActivities;

pub(crate) async fn extract_facts_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let sections_json_owned = pipeline_storage::extract_sections_json_from_run(&run);
    let extracted_typed = primitives::facts_extractor::extract_facts_typed(
        &sections_json_owned,
        &primitives::facts_extractor::ExtractionContext::default(),
    );
    let extracted_payload = pipeline_storage::build_extracted_payload_from_typed(extracted_typed);

    pipeline_storage::advance_execution_run_typed(
        &acts.pool,
        run_id,
        "extracting",
        Some("extracted_payload"),
        Some(&extracted_payload),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(run_id.to_string())
}

pub(crate) async fn verify_rules_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let extracted = pipeline_storage::decode_extracted_payload_from_run(&run);
    let rules = extracted.rule_instances;

    let mut invalid = 0usize;
    for rule in &rules {
        if let Some(cond) = rule.condition_expr.as_ref() {
            if policies::condition_schema::validate_condition(cond).is_err() {
                invalid += 1;
            }
        }
    }

    if invalid > 0 {
        return Err(DomainError::ValidationFailure {
            message: format!("verify_rules: {invalid} invalid condition_expr in rule_instances"),
        });
    }

    let source_registry = pipeline_storage::load_source_registry_entries(&acts.pool)
        .await?
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                policies::conflict_resolution::SourceRegistryEntry {
                    source_type: value.source_type,
                    trust_level: value.trust_level,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let rules_for_conflict: Vec<policies::conflict_resolution::ConflictRuleCandidate> = rules
        .iter()
        .map(|r| policies::conflict_resolution::ConflictRuleCandidate {
            source_key: r.source_key.clone(),
            fact_value: r.params.primary_value_text(),
        })
        .collect();

    let conflict_resolution = match policies::conflict_resolution::resolve_source_conflict_typed(
        &rules_for_conflict,
        &source_registry,
    ) {
        policies::conflict_resolution::ConflictResolution::GovernmentWins => "government_wins",
        policies::conflict_resolution::ConflictResolution::ConsensusWins => "consensus_wins",
        policies::conflict_resolution::ConflictResolution::Disputed => "disputed",
    };

    let (input_hash, idempotency_key) =
        pipeline_storage::derive_step_keys(run_id, "verify_rules", 1, &run_id)
            .map_err(AlegriaActivities::classify_error)?;
    let verify_report = VerifyReport {
        meta: Some(AlegriaActivities::build_meta(
            run_id,
            "verify_rules",
            1,
            &input_hash,
            None,
            &idempotency_key,
            None,
        )),
        rules_total: rules.len() as u32,
        invalid_conditions: 0,
        conflict_resolution: conflict_resolution.to_string(),
    };

    pipeline_storage::advance_execution_run_typed(
        &acts.pool,
        run_id,
        "verifying",
        Some("verify_report"),
        Some(&verify_report),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(run_id.to_string())
}

pub(crate) async fn prepare_hitl_pause_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<HitlPauseInfo, DomainError> {
    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let verify_report_typed = pipeline_storage::decode_verify_report_from_run(&run);
    let conflict_resolution = verify_report_typed
        .as_ref()
        .map(|report| report.conflict_resolution.as_str())
        .unwrap_or_default();

    if conflict_resolution != "disputed" {
        return Ok(HitlPauseInfo {
            requires_hitl: false,
            task_id: 0,
            reason: String::new(),
        });
    }

    let diagnostics = HitlTaskContext {
        run_id: run_id.to_string(),
        reason: "conflict_disputed".to_string(),
        verify_report: verify_report_typed,
    };

    let task_id =
        use_cases::hitl_queue::enqueue_hitl_task(&acts.pool, "fact_conflict", &diagnostics, 1)
            .await
            .map_err(AlegriaActivities::classify_error)?;

    Ok(HitlPauseInfo {
        requires_hitl: true,
        task_id,
        reason: "conflict_disputed".to_string(),
    })
}

pub(crate) async fn apply_hitl_resolution_impl(
    acts: &AlegriaActivities,
    input: &HitlResolutionInput,
) -> Result<String, DomainError> {
    let run_id = input
        .meta
        .as_ref()
        .map(|m| m.run_id.clone())
        .unwrap_or_default();
    let resolution = input
        .resolution
        .clone()
        .ok_or_else(|| DomainError::ContractViolation {
            message: "missing HITL resolution payload".to_string(),
        })?;

    use_cases::hitl_queue::resolve_hitl_task(&acts.pool, input.task_id, &resolution)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    pipeline_storage::write_hitl_decision_typed(
        &acts.pool,
        &run_id,
        "apply_hitl_resolution",
        input.task_id,
        &resolution.decision,
        &resolution,
        Some(&resolution.actor),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    let decision = resolution.decision.to_lowercase();
    let approved = matches!(decision.as_str(), "approve" | "approved" | "accept");
    let next_status = if approved { "verifying" } else { "failed" };

    pipeline_storage::advance_execution_run_status(&acts.pool, &run_id, next_status)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    if !approved {
        return Err(DomainError::ValidationFailure {
            message: format!("HITL rejected run {}", run_id),
        });
    }

    Ok(run_id)
}

pub(crate) async fn persist_and_emit_impl(
    acts: &AlegriaActivities,
    run_id: &str,
) -> Result<String, DomainError> {
    use contracts::generated::alegria::temporal::v1::PersistReport;

    let run = pipeline_storage::read_execution_run(&acts.pool, run_id)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let state = pipeline_storage::PersistPipelineState {
        context_key: run.context_key().to_string(),
        extracted_payload: pipeline_storage::decode_extracted_payload_from_run(&run),
    };

    let (written, outbox_count) = pipeline_storage::persist_from_pipeline_state(&acts.pool, &state)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let (input_hash, idempotency_key) =
        pipeline_storage::derive_step_keys(run_id, "persist_and_emit", 1, &run_id)
            .map_err(AlegriaActivities::classify_error)?;
    let persist_report = PersistReport {
        meta: Some(AlegriaActivities::build_meta(
            run_id,
            "persist_and_emit",
            1,
            &input_hash,
            None,
            &idempotency_key,
            None,
        )),
        rules_written: written as u32,
        outbox_emitted: outbox_count as u32,
    };

    pipeline_storage::advance_execution_run_typed(
        &acts.pool,
        run_id,
        "persisting",
        Some("persist_report"),
        Some(&persist_report),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(run_id.to_string())
}
