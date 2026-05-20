use contracts::generated::alegria::temporal::v1::{FreshnessReport, StepContractMeta};
use infrastructure::adapters::raw_crawl_adapter;
use infrastructure::adapters::sqlx_freshness_adapter::load_freshness_snapshot;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter::RuntimeProtoPayload;
use infrastructure::adapters::sqlx_reconcile_adapter;
use primitives::errors::DomainError;
use primitives::hash::content_hash_v1;
use runtime_models::ReconcileTargetReportRecord;
use serde::{Deserialize, Serialize};

use super::AlegriaActivities;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteInput {
    pub target_system: String,
    pub dry_run: bool,
    pub max_retry_count: Option<i32>,
    pub batch_limit: Option<i64>,
    pub requeue_base_delay_sec: Option<i64>,
    pub requeue_jitter_sec: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteOutput {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleOutput {
    pub section_id: String,
    pub page_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub section_type: String,
    pub raw_text: String,
}

impl RuntimeProtoPayload for SemanticSectionSampleInput {
    fn payload_type() -> &'static str {
        "alegria.runtime.json.SemanticSectionSampleInput"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        serde_json::to_vec(self).map_err(|e| DomainError::ContractViolation {
            message: e.to_string(),
        })
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        serde_json::from_slice(payload_bytes).map_err(|e| DomainError::ContractViolation {
            message: e.to_string(),
        })
    }
}

impl RuntimeProtoPayload for SemanticSectionSampleOutput {
    fn payload_type() -> &'static str {
        "alegria.runtime.json.SemanticSectionSampleOutput"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        serde_json::to_vec(self).map_err(|e| DomainError::ContractViolation {
            message: e.to_string(),
        })
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        serde_json::from_slice(payload_bytes).map_err(|e| DomainError::ContractViolation {
            message: e.to_string(),
        })
    }
}

pub(crate) fn test_step_prepare_impl(workflow_id: &str) -> String {
    format!("prepared:{workflow_id}")
}

pub(crate) fn test_step_finalize_impl(prepared_token: &str) -> String {
    format!("completed:{prepared_token}")
}

pub(crate) async fn check_data_freshness_impl(
    acts: &AlegriaActivities,
    threshold_input: &str,
) -> Result<String, DomainError> {
    let threshold_hours: i64 = threshold_input
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(24);

    let snapshot = load_freshness_snapshot(&acts.pool, threshold_hours)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let report = FreshnessReport {
        meta: Some(StepContractMeta {
            run_id: "operational:freshness".to_string(),
            step_name: "check_data_freshness".to_string(),
            schema_version: 1,
            input_hash: content_hash_v1(threshold_input),
            output_hash: String::new(),
            idempotency_key: content_hash_v1(&format!("operational:freshness|{}", threshold_input)),
            requires_hitl: false,
            prompt_version: String::new(),
            model_version: String::new(),
            registry_version: String::new(),
            error_class: String::new(),
            retry_class: "transient".to_string(),
            executor_version: AlegriaActivities::current_build_id(),
            derivation_version: "check_data_freshness@1".to_string(),
            scope_signature: String::new(),
            max_retries: 3,
        }),
        threshold_hours,
        stale_count: snapshot.stale_count,
        max_lag_hours: snapshot.max_lag_hours,
        status: if snapshot.stale_count > 0 {
            "stale"
        } else {
            "ok"
        }
        .to_string(),
    };

    serde_json::to_string(&report).map_err(AlegriaActivities::classify_error)
}

pub(crate) async fn neo4j_backwrite_impl(
    input: &Neo4jBackwriteInput,
) -> Result<Neo4jBackwriteOutput, DomainError> {
    let mut opts = sqlx_reconcile_adapter::load_default_reconcile_options();
    opts.dry_run = input.dry_run;
    if let Some(v) = input.max_retry_count {
        opts.max_retry_count = v;
    }
    if let Some(v) = input.batch_limit {
        opts.batch_limit = v;
    }
    if let Some(v) = input.requeue_base_delay_sec {
        opts.requeue_base_delay_sec = v;
    }
    if let Some(v) = input.requeue_jitter_sec {
        opts.requeue_jitter_sec = v;
    }

    let target = if input.target_system.trim().is_empty() {
        "neo4j"
    } else {
        input.target_system.as_str()
    };
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(target, &opts)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    Ok(Neo4jBackwriteOutput {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_reconcile_impl(
    target_system: &str,
    dry_run: bool,
    max_retry_count: i32,
    batch_limit: i64,
    requeue_base_delay_sec: i64,
    requeue_jitter_sec: i64,
) -> Result<ReconcileTargetReportRecord, DomainError> {
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(
        target_system,
        &sqlx_reconcile_adapter::ReconcileOptionsRecord {
            max_retry_count,
            batch_limit,
            dry_run,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        },
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(ReconcileTargetReportRecord {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn load_semantic_section_sample_impl(
    acts: &AlegriaActivities,
    input: &SemanticSectionSampleInput,
) -> Result<SemanticSectionSampleOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let section = sections
        .into_iter()
        .find(|section| !section.content_md.trim().is_empty())
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "no non-empty raw section available for semantic slice".to_string(),
        })?;

    Ok(SemanticSectionSampleOutput {
        section_id: section.id.to_string(),
        page_id: section.page_id,
        source_url: section.source_url,
        source_domain: section.source_domain,
        heading_path: section.heading_path,
        section_type: section.section_type,
        raw_text: section.content_md,
    })
}
