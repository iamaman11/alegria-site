//! Canonical protobuf wire mapping for PostgreSQL reconciliation reports.
//!
//! Both the PostgreSQL persistence owner and the general runtime payload
//! catalog use these functions. The wire format is defined exactly once;
//! changing a database owner must not create a second serializer.

use contracts::generated::alegria::temporal::v1::{
    ReconcileSummaryPayload, ReconcileTargetReportPayload,
};
use prost::{DecodeError, Message};

use crate::{ReconcileSummary, ReconcileTargetReportRecord};

pub const RECONCILE_SUMMARY_PAYLOAD_TYPE: &str = "alegria.temporal.v1.ReconcileSummaryPayload";
pub const RECONCILE_TARGET_PAYLOAD_TYPE: &str = "alegria.temporal.v1.ReconcileTargetReportPayload";
pub const RECONCILE_SCHEMA_VERSION: i32 = 1;

pub fn encode_summary(value: &ReconcileSummary) -> Vec<u8> {
    ReconcileSummaryPayload {
        open_dlq: value.open_dlq,
        stale_runs: value.stale_runs,
        pending_hitl_runs: value.pending_hitl_runs,
        stuck_steps: value.stuck_steps,
    }
    .encode_to_vec()
}

pub fn decode_summary(bytes: &[u8]) -> Result<ReconcileSummary, DecodeError> {
    let payload = ReconcileSummaryPayload::decode(bytes)?;
    Ok(ReconcileSummary {
        open_dlq: payload.open_dlq,
        stale_runs: payload.stale_runs,
        pending_hitl_runs: payload.pending_hitl_runs,
        stuck_steps: payload.stuck_steps,
    })
}

pub fn encode_target_report(value: &ReconcileTargetReportRecord) -> Vec<u8> {
    ReconcileTargetReportPayload {
        target_system: value.target_system.clone(),
        dry_run: value.dry_run,
        stale_candidates: value.stale_candidates,
        failed_candidates: value.failed_candidates,
        reset_stale_processing: value.reset_stale_processing,
        requeued_failed: value.requeued_failed,
    }
    .encode_to_vec()
}

pub fn decode_target_report(bytes: &[u8]) -> Result<ReconcileTargetReportRecord, DecodeError> {
    let payload = ReconcileTargetReportPayload::decode(bytes)?;
    Ok(ReconcileTargetReportRecord {
        target_system: payload.target_system,
        dry_run: payload.dry_run,
        stale_candidates: payload.stale_candidates,
        failed_candidates: payload.failed_candidates,
        reset_stale_processing: payload.reset_stale_processing,
        requeued_failed: payload.requeued_failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_summary_retains_original_protobuf_fields() {
        let value = ReconcileSummary {
            open_dlq: 1,
            stale_runs: 2,
            pending_hitl_runs: 3,
            stuck_steps: 4,
        };
        let wire = encode_summary(&value);
        let decoded = decode_summary(&wire).unwrap();
        assert_eq!(decoded.open_dlq, value.open_dlq);
        assert_eq!(decoded.stale_runs, value.stale_runs);
        assert_eq!(decoded.pending_hitl_runs, value.pending_hitl_runs);
        assert_eq!(decoded.stuck_steps, value.stuck_steps);
        assert_eq!(
            ReconcileSummaryPayload::decode(wire.as_slice())
                .unwrap()
                .open_dlq,
            1
        );
    }

    #[test]
    fn reconcile_target_report_retains_original_protobuf_fields() {
        let value = ReconcileTargetReportRecord {
            target_system: "neo4j".to_string(),
            dry_run: true,
            stale_candidates: 2,
            failed_candidates: 3,
            reset_stale_processing: 4,
            requeued_failed: 5,
        };
        let wire = encode_target_report(&value);
        let decoded = decode_target_report(&wire).unwrap();
        assert_eq!(decoded.target_system, value.target_system);
        assert_eq!(decoded.dry_run, value.dry_run);
        assert_eq!(decoded.stale_candidates, value.stale_candidates);
        assert_eq!(decoded.failed_candidates, value.failed_candidates);
        assert_eq!(decoded.reset_stale_processing, value.reset_stale_processing);
        assert_eq!(decoded.requeued_failed, value.requeued_failed);
        assert_eq!(
            ReconcileTargetReportPayload::decode(wire.as_slice())
                .unwrap()
                .target_system,
            "neo4j"
        );
    }
}
