use infrastructure::adapters::temporalio_sdk_adapter::ActivityOptions;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) fn fast_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions {
        start_to_close_timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
}

pub(crate) fn db_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions {
        start_to_close_timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
}

pub(crate) fn llm_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions {
        start_to_close_timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
}

pub(crate) fn test_opts(timeout_secs: u64) -> ActivityOptions {
    ActivityOptions {
        start_to_close_timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkflowStatus {
    pub phase: String,
    pub paused: bool,
    pub waiting_hitl: bool,
    pub hitl_task_id: Option<i64>,
    pub has_hitl_resolution: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BasicWorkflowStatus {
    pub phase: String,
    pub paused: bool,
}
