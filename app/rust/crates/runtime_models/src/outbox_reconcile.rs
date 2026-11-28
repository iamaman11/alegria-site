//! Provider-independent command for the PostgreSQL Outbox lease owner.
//!
//! A target is an opaque identifier: adding another projection provider does
//! not require changing the operation catalog or orchestration protocol.
//! This command does not expose SQLx, Temporal or any projection SDK.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileOutboxOptions {
    pub max_retry_count: i32,
    pub batch_limit: i64,
    pub dry_run: bool,
    pub requeue_base_delay_sec: i64,
    pub requeue_jitter_sec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileOutboxCommand {
    pub target_system: String,
    pub options: ReconcileOutboxOptions,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_future_target_without_a_provider_enum() {
        let command = ReconcileOutboxCommand {
            target_system: "search-provider-next".to_owned(),
            options: ReconcileOutboxOptions {
                max_retry_count: 10,
                batch_limit: 100,
                dry_run: true,
                requeue_base_delay_sec: 0,
                requeue_jitter_sec: 15,
            },
        };
        let encoded = serde_json::to_vec(&command).unwrap();
        let decoded: ReconcileOutboxCommand = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, command);
    }
}
