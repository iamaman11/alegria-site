//! Bounded, provider-free view of a completed Temporal DAG for run-status readers.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{owner_dispatch::OWNER_CHILD_WORKFLOW_ID_PREFIX_V2, OperationRef, DAG_MAX_NODES};

pub const DAG_READ_TRACE_SCHEMA_REVISION_V1: u16 = 1;
pub const DAG_READ_MAX_REPLY_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDagReadTraceV1 {
    pub schema_revision: u16,
    /// Canonical execution order of the sealed plan, with zero-based indices.
    pub steps: Vec<OperationDagReadStepV1>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDagReadStepV1 {
    pub operation: OperationRef,
    /// Producers of typed inputs, as indices into `steps`.
    pub input_steps: Vec<usize>,
    pub child_workflow_id: String,
    pub child_run_id: Uuid,
    pub receipt_blake3: String,
}

impl OperationDagReadTraceV1 {
    /// Check wire shape before rendering. This is separate from fetching and
    /// proving the corresponding server Histories.
    pub fn has_valid_shape(&self, expected_steps: usize) -> bool {
        let is_hex_digest = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let mut workflow_ids = BTreeSet::new();
        let mut run_ids = BTreeSet::new();
        self.schema_revision == DAG_READ_TRACE_SCHEMA_REVISION_V1
            && (1..=DAG_MAX_NODES).contains(&expected_steps)
            && self.steps.len() == expected_steps
            && self.steps.iter().enumerate().all(|(index, step)| {
                let operation = step.operation.id.as_str();
                !operation.is_empty()
                    && operation.len() <= 256
                    && operation
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                    && step.operation.revision > 0
                    && step.input_steps.iter().all(|&producer| producer < index)
                    && step.input_steps.windows(2).all(|pair| pair[0] < pair[1])
                    && step
                        .child_workflow_id
                        .strip_prefix(OWNER_CHILD_WORKFLOW_ID_PREFIX_V2)
                        .is_some_and(is_hex_digest)
                    && workflow_ids.insert(step.child_workflow_id.as_str())
                    && !step.child_run_id.is_nil()
                    && run_ids.insert(step.child_run_id)
                    && is_hex_digest(&step.receipt_blake3)
            })
    }
}
