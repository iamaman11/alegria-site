//! Only JSON wire boundary for the private Acquisition source-issuer CLI.

use anyhow::{ensure, Context, Result};
use operation_protocol::ArtifactId;
use uuid::Uuid;

use super::SourceIssuanceIntentV1;

pub(super) fn decode_issuance_intent(bytes: &[u8]) -> Result<SourceIssuanceIntentV1> {
    let intent: SourceIssuanceIntentV1 = serde_json::from_slice(bytes)
        .context("invalid typed Acquisition source issuance request")?;
    ensure!(
        serde_json::to_vec(&intent)? == bytes,
        "source issuance requires canonical typed JSON request"
    );
    Ok(intent)
}

pub(super) fn issued_report(
    run_id: &str,
    record_id: Uuid,
    artifact_id: &ArtifactId,
    payload_hash: &str,
) -> String {
    serde_json::json!({
        "operation":"acquisition_issue_source_v1",
        "status":"source_issued_from_pinned_raw_html",
        "run_id":run_id,
        "record_id":record_id,
        "artifact_id":artifact_id,
        "payload_hash":payload_hash,
        "execution":"not_started",
    })
    .to_string()
}

pub(super) fn unconfirmed_report() -> String {
    serde_json::json!({
        "operation":"acquisition_issue_source_v1",
        "status":"refused_or_outcome_unknown",
        "source_issuance":"not_confirmed",
        "execution":"not_started",
        "next_action":"Inspect the exact record_id in tenant SQL before retry; a lost commit acknowledgement is not a refusal.",
    }).to_string()
}
