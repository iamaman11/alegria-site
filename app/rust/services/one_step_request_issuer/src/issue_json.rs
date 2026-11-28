//! Exactly one canonical JSON ingress from a private, operator-owned file.
//! The request is NOT a direct MCP Start or authorization approval: its
//! identifiers only select a separately preapproved tenant SQL event.

use anyhow::{ensure, Context, Result};
use operation_protocol::{OperationDagRunV6, SiteArchitectExpectedStartV1};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OneStepIssuanceIntentV1 {
    pub schema_revision: u16,
    pub request_id: Uuid,
    pub authorization_event_id: Uuid,
    pub run: OperationDagRunV6,
    pub expected: SiteArchitectExpectedStartV1,
    pub context_key: String,
    pub expires_at_unix_seconds: i64,
}

pub(crate) fn decode_issuance_intent(bytes: &[u8]) -> Result<OneStepIssuanceIntentV1> {
    let intent: OneStepIssuanceIntentV1 =
        serde_json::from_slice(bytes).context("invalid private one-step issuance intent")?;
    ensure!(
        serde_json::to_vec(&intent)? == bytes,
        "private one-step issuance requires canonical typed JSON"
    );
    ensure!(
        intent.schema_revision == 1
            && !intent.request_id.is_nil()
            && !intent.authorization_event_id.is_nil(),
        "one-step issuance requires canonical nonnil request and approval UUIDs"
    );
    Ok(intent)
}

pub(crate) fn issued_report(request_id: Uuid, canonical_run_id: &str) -> String {
    serde_json::json!({
        "schema_revision": 1,
        "operation": "one_step_request_issue_v1",
        "status": "one_step_request_issued_not_started",
        "request_id": request_id,
        "canonical_run_id": canonical_run_id,
        "execution": "not_started",
        "remote_chatgpt_identity": "not_authenticated_by_this_issuer",
        "production_user_start": "not_enabled",
        "next_action": "Use only the separate tenant-authenticated atomic one-use Start bridge; issuance alone is not execution authorization.",
    })
    .to_string()
}

pub(crate) fn unknown_report() -> String {
    serde_json::json!({
        "schema_revision": 1,
        "operation": "one_step_request_issue_v1",
        "status": "refused_or_commit_outcome_unknown",
        "execution": "not_started",
        "next_action": "Inspect the exact request_id in tenant SQL before any new issuance; a lost commit acknowledgement is not proof of rollback.",
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_wire_denies_extra_fields_and_noncanonical_bytes() {
        // These require no process credentials or running database. The full
        // canonical V6 fixture is exercised by the dedicated native E2E.
        let with_unknown = br#"{"schema_revision":1,"request_id":"e0813cac-2277-454e-980a-2e08110c1a08","authorization_event_id":"0e2dddf2-0f45-46ed-89dd-3dce7e8b247d","run":{},"expected":{},"context_key":"x","expires_at_unix_seconds":1890000100,"override_start":true}"#;
        assert!(decode_issuance_intent(with_unknown).is_err());
        assert!(decode_issuance_intent(b"\n").is_err());
        assert!(decode_issuance_intent(b"{}").is_err());
        let report = unknown_report();
        assert!(!report.contains("database_url"));
        assert!(!report.contains("tenant_id"));
    }
}
