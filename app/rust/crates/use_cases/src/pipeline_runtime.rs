use std::collections::BTreeMap;

use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter as adapter;
use primitives::errors::DomainError;
use runtime_models::ExecutionRun as DomainExecutionRun;
pub use runtime_models::{
    ExtractedPayload, FactCandidateValue, PersistPipelineState, ReconcileSummary,
    ReconcileTargetReportRecord, RuleInstanceCandidate, RuleParams, SourceRegistryRecord,
    ValidationInputRecord,
};

pub struct ExecutionRun(DomainExecutionRun);

impl ExecutionRun {
    pub fn context_key(&self) -> &str {
        &self.0.context_key
    }

    pub fn status(&self) -> &str {
        &self.0.status
    }
}

pub fn decode_extracted_payload_from_run(run: &ExecutionRun) -> ExtractedPayload {
    adapter::decode_extracted_payload(run.0.extracted_payload.as_ref())
}

pub fn build_extracted_payload_from_typed(
    extracted: primitives::facts_extractor::ExtractedFactsEnvelope,
) -> ExtractedPayload {
    adapter::build_extracted_payload_from_typed(extracted)
}

pub fn decode_verify_report_from_run(
    run: &ExecutionRun,
) -> Option<contracts::generated::alegria::temporal::v1::VerifyReport> {
    adapter::decode_verify_report(run.0.verify_report.as_ref())
}

pub fn extracted_payload_rules_json(payload: &ExtractedPayload) -> String {
    adapter::extracted_payload_rules_json(payload)
}

pub fn extracted_payload_facts_json(payload: &ExtractedPayload) -> String {
    adapter::extracted_payload_facts_json(payload)
}

pub fn extract_sections_json_from_run(run: &ExecutionRun) -> String {
    adapter::extract_sections_json(run.0.input_payload.as_ref())
}

pub fn decode_validation_input_from_run(run: &ExecutionRun) -> ValidationInputRecord {
    adapter::decode_validation_input(run.0.input_payload.as_ref())
}

pub fn decode_generation_result_from_run(run: &ExecutionRun) -> BTreeMap<String, String> {
    adapter::decode_generation_result(run.0.generation_result.as_ref())
}

pub async fn read_execution_run(
    pool: &AlegriaPgPool,
    run_id: &str,
) -> std::result::Result<ExecutionRun, DomainError> {
    adapter::read_execution_run(pool, run_id)
        .await
        .map(ExecutionRun)
}

pub async fn advance_execution_run_status(
    pool: &AlegriaPgPool,
    run_id: &str,
    new_status: &str,
) -> std::result::Result<(), DomainError> {
    adapter::advance_execution_run(pool, run_id, new_status, None, None, None, None, None).await
}

pub async fn advance_execution_run_typed<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    new_status: &str,
    json_field: Option<&str>,
    payload: Option<&T>,
) -> std::result::Result<(), DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::advance_execution_run_typed(pool, run_id, new_status, json_field, payload).await
}

pub async fn begin_step_execution(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input_hash: &str,
    idempotency_key: &str,
) -> std::result::Result<bool, DomainError> {
    adapter::begin_step_execution(
        pool,
        run_id,
        step_name,
        schema_version,
        input_hash,
        idempotency_key,
    )
    .await
}

pub async fn read_step_execution_id(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
) -> std::result::Result<Option<i64>, DomainError> {
    adapter::read_step_execution_id(pool, run_id, step_name, idempotency_key).await
}

pub async fn begin_step_attempt(
    pool: &AlegriaPgPool,
    step_execution_id: i64,
) -> std::result::Result<i32, DomainError> {
    adapter::begin_step_attempt(pool, step_execution_id).await
}

pub async fn finish_step_attempt(
    pool: &AlegriaPgPool,
    step_execution_id: i64,
    attempt_no: i32,
    status: &str,
    error_class: Option<&str>,
    error_message: Option<&str>,
) -> std::result::Result<(), DomainError> {
    adapter::finish_step_attempt(
        pool,
        step_execution_id,
        attempt_no,
        status,
        error_class,
        error_message,
    )
    .await
}

pub async fn load_completed_step_result<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
) -> std::result::Result<Option<T>, DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::load_completed_step_result(pool, run_id, step_name, idempotency_key).await
}

pub async fn complete_step_execution_typed<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    output_hash: &str,
    payload: Option<&T>,
) -> std::result::Result<(), DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::complete_step_execution_typed(
        pool,
        run_id,
        step_name,
        idempotency_key,
        output_hash,
        payload,
    )
    .await
}

pub async fn fail_step_execution(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    error_class: &str,
    error_message: &str,
    terminal_status: &str,
) -> std::result::Result<(), DomainError> {
    adapter::fail_step_execution(
        pool,
        run_id,
        step_name,
        idempotency_key,
        error_class,
        error_message,
        terminal_status,
    )
    .await
}

pub fn derive_step_keys<T>(
    run_id: &str,
    step_name: &str,
    schema_version: i32,
    input: &T,
) -> std::result::Result<(String, String), DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::derive_step_keys(run_id, step_name, schema_version, input)
}

pub async fn write_dead_letter_typed<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    workflow_id: &str,
    error_class: primitives::errors::ErrorClass,
    payload: &T,
    input_hash: &str,
    idempotency_key: &str,
    build_id: &str,
    error_message: &str,
) -> std::result::Result<(), DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::write_dead_letter_typed(
        pool,
        run_id,
        step_name,
        workflow_id,
        error_class,
        payload,
        input_hash,
        idempotency_key,
        build_id,
        error_message,
    )
    .await
}

pub async fn write_step_payload_blob_typed<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    idempotency_key: &str,
    payload_kind: &str,
    schema_version: i32,
    payload: &T,
) -> std::result::Result<String, DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::write_step_payload_blob_typed(
        pool,
        run_id,
        step_name,
        idempotency_key,
        payload_kind,
        schema_version,
        payload,
    )
    .await
}

pub async fn load_source_registry_entries(
    pool: &AlegriaPgPool,
) -> std::result::Result<BTreeMap<String, SourceRegistryRecord>, DomainError> {
    adapter::load_source_registry_entries(pool).await
}

pub async fn persist_from_pipeline_state(
    pool: &AlegriaPgPool,
    state: &PersistPipelineState,
) -> std::result::Result<(usize, usize), DomainError> {
    adapter::persist_from_pipeline_state(pool, state).await
}

pub async fn write_hitl_decision_typed<T>(
    pool: &AlegriaPgPool,
    run_id: &str,
    step_name: &str,
    task_id: i64,
    decision_status: &str,
    decision_payload: &T,
    resolved_by: Option<&str>,
) -> std::result::Result<(), DomainError>
where
    T: adapter::RuntimeProtoPayload,
{
    adapter::write_hitl_decision_typed(
        pool,
        run_id,
        step_name,
        task_id,
        decision_status,
        decision_payload,
        resolved_by,
    )
    .await
}

pub async fn begin_reconcile_run(pool: &AlegriaPgPool) -> std::result::Result<i64, DomainError> {
    adapter::begin_reconcile_run(pool).await
}

pub async fn append_reconcile_target_action(
    pool: &AlegriaPgPool,
    reconcile_run_id: i64,
    action_type: &str,
    target_system: Option<&str>,
    target_key: Option<&str>,
    details: &ReconcileTargetReportRecord,
) -> std::result::Result<(), DomainError> {
    adapter::append_reconcile_target_action(
        pool,
        reconcile_run_id,
        action_type,
        target_system,
        target_key,
        details,
    )
    .await
}

pub async fn append_reconcile_summary_action(
    pool: &AlegriaPgPool,
    reconcile_run_id: i64,
    action_type: &str,
    target_system: Option<&str>,
    target_key: Option<&str>,
    details: &ReconcileSummary,
) -> std::result::Result<(), DomainError> {
    adapter::append_reconcile_summary_action(
        pool,
        reconcile_run_id,
        action_type,
        target_system,
        target_key,
        details,
    )
    .await
}

pub async fn finish_reconcile_run(
    pool: &AlegriaPgPool,
    reconcile_run_id: i64,
    status: &str,
    summary: &ReconcileSummary,
    error_message: Option<&str>,
) -> std::result::Result<(), DomainError> {
    adapter::finish_reconcile_run(pool, reconcile_run_id, status, summary, error_message).await
}
