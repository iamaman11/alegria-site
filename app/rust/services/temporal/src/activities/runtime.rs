use std::collections::BTreeMap;
use std::io::Error as IoError;

use contracts::generated::alegria::temporal::v1::{RuntimeErrorPayload, StepContractMeta};
use futures::Future;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter::{
    self as pipeline_storage, RuntimeProtoPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::ActivityError;
use primitives::errors::{DomainError, ErrorClass};
use primitives::hash::blake3_hex;

use super::AlegriaActivities;
use crate::metrics::ActivityTimer;

impl AlegriaActivities {
    pub(crate) fn current_build_id() -> String {
        std::env::var("WORKER_BUILD_ID").unwrap_or_else(|_| "dev-local".to_string())
    }

    pub(crate) fn build_meta(
        run_id: &str,
        step_name: &str,
        schema_version: i32,
        input_hash: &str,
        output_hash: Option<&str>,
        idempotency_key: &str,
        error_class: Option<ErrorClass>,
    ) -> StepContractMeta {
        StepContractMeta {
            run_id: run_id.to_string(),
            step_name: step_name.to_string(),
            schema_version: schema_version as u32,
            input_hash: input_hash.to_string(),
            output_hash: output_hash.unwrap_or_default().to_string(),
            idempotency_key: idempotency_key.to_string(),
            requires_hitl: false,
            prompt_version: String::new(),
            model_version: String::new(),
            registry_version: String::new(),
            error_class: error_class
                .map(|c| c.as_str().to_string())
                .unwrap_or_default(),
            retry_class: "safe".to_string(),
            executor_version: Self::current_build_id(),
            derivation_version: format!("{step_name}@{schema_version}"),
            scope_signature: String::new(),
            max_retries: 3,
        }
    }

    pub(crate) fn classify_error_message(msg: &str) -> DomainError {
        let lower = msg.to_lowercase();
        if lower.contains("violates foreign key constraint") || lower.contains("empty concept_key")
        {
            DomainError::ForeignKeyViolation {
                message: msg.to_string(),
            }
        } else if lower.contains("violates check constraint")
            || lower.contains("invalid input value")
            || lower.contains("invalid run_id uuid")
            || lower.contains("contract violation")
        {
            DomainError::ContractViolation {
                message: msg.to_string(),
            }
        } else if lower.contains("validation failure") || lower.contains("invalid condition_expr") {
            DomainError::ValidationFailure {
                message: msg.to_string(),
            }
        } else if lower.contains("timeout") {
            DomainError::TransportTimeout {
                message: msg.to_string(),
            }
        } else if lower.contains("rate limit") || lower.contains("too many requests") {
            DomainError::RemoteRateLimit {
                message: msg.to_string(),
            }
        } else if lower.contains("connection refused")
            || lower.contains("pool timed out")
            || lower.contains("connection reset")
            || lower.contains("database is closed")
        {
            DomainError::InfraUnavailable {
                message: msg.to_string(),
            }
        } else {
            DomainError::UnexpectedBug {
                message: msg.to_string(),
            }
        }
    }

    fn boxed_error(message: String) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(IoError::other(message))
    }

    pub(crate) fn classify_error<E: std::fmt::Display>(err: E) -> DomainError {
        Self::classify_error_message(&err.to_string())
    }

    pub(crate) fn into_activity_error<E: std::fmt::Display>(err: E) -> ActivityError {
        let message = err.to_string();
        let classified = Self::classify_error_message(&message);
        match classified {
            DomainError::ContractViolation { .. }
            | DomainError::ValidationFailure { .. }
            | DomainError::ForeignKeyViolation { .. }
            | DomainError::ConflictViolation { .. } => {
                ActivityError::NonRetryable(Self::boxed_error(message))
            }
            DomainError::TransportTimeout { .. }
            | DomainError::RemoteRateLimit { .. }
            | DomainError::Remote5xx { .. }
            | DomainError::InfraUnavailable { .. }
            | DomainError::UnexpectedBug { .. } => ActivityError::Retryable {
                source: Self::boxed_error(message),
                explicit_delay: None,
            },
        }
    }

    pub(crate) fn activity_error_from_domain(err: &DomainError) -> ActivityError {
        let boxed = Self::boxed_error(err.to_string());
        match err {
            DomainError::ContractViolation { .. }
            | DomainError::ValidationFailure { .. }
            | DomainError::ForeignKeyViolation { .. }
            | DomainError::ConflictViolation { .. } => ActivityError::NonRetryable(boxed),
            DomainError::TransportTimeout { .. }
            | DomainError::RemoteRateLimit { .. }
            | DomainError::Remote5xx { .. }
            | DomainError::InfraUnavailable { .. }
            | DomainError::UnexpectedBug { .. } => ActivityError::Retryable {
                source: boxed,
                explicit_delay: None,
            },
        }
    }

    pub(crate) fn extract_block_htmls(
        generation_result: &BTreeMap<String, String>,
    ) -> Vec<(String, String)> {
        let mut out = generation_result
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub(crate) async fn execute_step<I, O, F, Fut>(
        &self,
        run_id: &str,
        step_name: &str,
        schema_version: i32,
        input: &I,
        op: F,
    ) -> Result<O, ActivityError>
    where
        I: RuntimeProtoPayload,
        O: RuntimeProtoPayload + Clone,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<O, DomainError>>,
    {
        let (input_hash, idempotency_key) =
            pipeline_storage::derive_step_keys(run_id, step_name, schema_version, input)
                .map_err(Self::into_activity_error)?;
        let timer = ActivityTimer::start(step_name);

        let started = pipeline_storage::begin_step_execution(
            &self.pool,
            run_id,
            step_name,
            schema_version,
            &input_hash,
            &idempotency_key,
        )
        .await
        .map_err(Self::into_activity_error)?;

        if !started {
            crate::metrics::global()
                .step_execution_reused_total
                .with_label_values(&[step_name])
                .inc();
            if let Some(existing) = pipeline_storage::load_completed_step_result::<O>(
                &self.pool,
                run_id,
                step_name,
                &idempotency_key,
            )
            .await
            .map_err(Self::into_activity_error)?
            {
                timer.record_success();
                return Ok(existing);
            }
        }

        let step_execution_id = pipeline_storage::read_step_execution_id(
            &self.pool,
            run_id,
            step_name,
            &idempotency_key,
        )
        .await
        .map_err(Self::into_activity_error)?
        .ok_or_else(|| {
            Self::into_activity_error("missing step_execution row after begin_step_execution")
        })?;

        let attempt_no = pipeline_storage::begin_step_attempt(&self.pool, step_execution_id)
            .await
            .map_err(Self::into_activity_error)?;

        let _ = pipeline_storage::write_step_payload_blob_typed(
            &self.pool,
            run_id,
            step_name,
            &idempotency_key,
            "input",
            schema_version,
            input,
        )
        .await;

        match op().await {
            Ok(output) => {
                let output_hash = blake3_hex(
                    &output
                        .encode_payload_bytes()
                        .map_err(Self::into_activity_error)?,
                );
                let _ = pipeline_storage::write_step_payload_blob_typed(
                    &self.pool,
                    run_id,
                    step_name,
                    &idempotency_key,
                    "output",
                    schema_version,
                    &output,
                )
                .await;
                pipeline_storage::complete_step_execution_typed(
                    &self.pool,
                    run_id,
                    step_name,
                    &idempotency_key,
                    &output_hash,
                    Some(&output),
                )
                .await
                .map_err(Self::into_activity_error)?;
                let _ = pipeline_storage::finish_step_attempt(
                    &self.pool,
                    step_execution_id,
                    attempt_no,
                    "done",
                    None,
                    None,
                )
                .await;
                timer.record_success();
                Ok(output)
            }
            Err(err) => {
                let domain = Self::classify_error_message(&err.to_string());
                let error_class = domain.class();
                let _ = pipeline_storage::fail_step_execution(
                    &self.pool,
                    run_id,
                    step_name,
                    &idempotency_key,
                    error_class.as_str(),
                    &err.to_string(),
                    "failed",
                )
                .await;
                let _ = pipeline_storage::write_step_payload_blob_typed(
                    &self.pool,
                    run_id,
                    step_name,
                    &idempotency_key,
                    "error",
                    schema_version,
                    &RuntimeErrorPayload {
                        error_class: error_class.as_str().to_string(),
                        error_message: err.to_string(),
                    },
                )
                .await;
                let _ = pipeline_storage::finish_step_attempt(
                    &self.pool,
                    step_execution_id,
                    attempt_no,
                    "failed",
                    Some(error_class.as_str()),
                    Some(&err.to_string()),
                )
                .await;
                if matches!(
                    error_class,
                    ErrorClass::ContractViolation
                        | ErrorClass::ValidationFailure
                        | ErrorClass::ForeignKeyViolation
                        | ErrorClass::ConflictViolation
                ) {
                    let _ = pipeline_storage::write_dead_letter_typed(
                        &self.pool,
                        run_id,
                        step_name,
                        run_id,
                        error_class,
                        input,
                        &input_hash,
                        &idempotency_key,
                        &Self::current_build_id(),
                        &err.to_string(),
                    )
                    .await;
                }
                timer.record_failure(error_class.as_str());
                Err(Self::activity_error_from_domain(&domain))
            }
        }
    }
}
