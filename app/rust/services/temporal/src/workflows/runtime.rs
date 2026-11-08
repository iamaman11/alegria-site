use infrastructure::adapters::temporalio_sdk_adapter::{
    ActivityOptions, ApplicationFailure, WorkflowTermination,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) fn fast_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(timeout_secs))
}

pub(crate) fn db_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(timeout_secs))
}

pub(crate) fn llm_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(timeout_secs))
}

pub(crate) fn test_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(timeout_secs))
}

/// A deterministic input/contract violation must fail this workflow execution
/// without retrying the same invalid payload indefinitely.
pub(crate) fn workflow_contract_failure(message: impl Into<String>) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(message.into()),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BasicWorkflowStatus {
    pub phase: String,
    pub paused: bool,
}
