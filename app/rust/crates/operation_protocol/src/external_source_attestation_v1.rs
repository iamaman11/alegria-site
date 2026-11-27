//! Provider-free verification of a separately issued, persisted external
//! source record against the exact V6 Temporal start payload.
//!
//! `TrustedExternalSourceRecordV1` is **not** a bearer token or a signature:
//! its authenticity comes only from an independently credentialed read of the
//! immutable source-issuer store. Never deserialize this DTO from a caller's
//! proposed plan and then treat it as trusted. This module performs the
//! deterministic second half of that proof, without SQL or provider I/O.
//! The generic DAG's historical wire formats remain unchanged.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    owner_content_hash, ArtifactBinding, ArtifactId, ArtifactSchema, OperationDagRunV6, PlanHash,
    OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
};

pub const EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1: u16 = 1;
const PROVENANCE_PREFIX: &str = "source-record-v1:";

/// A row loaded from the restricted, durable source issuer. The caller must
/// already have authenticated the database principal and tenant context.
/// `record_id` is a canonical lowercase UUID; `payload_hash` is a canonical
/// BLAKE3 digest of the exact *admitted* byte payload (not of a URL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedExternalSourceRecordV1 {
    pub schema_revision: u16,
    pub record_id: String,
    pub tenant_id: String,
    pub context_key: String,
    pub run_id: String,
    pub plan_hash: PlanHash,
    pub artifact_id: ArtifactId,
    pub schema: ArtifactSchema,
    pub payload_hash: String,
    pub issued_at_unix_seconds: i64,
    pub expires_at_unix_seconds: i64,
    pub revoked_at_unix_seconds: Option<i64>,
}

/// Locally verified equality evidence, not an authorization credential. The
/// private fields prevent accidental serialization as an external grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExternalSourceV1 {
    record_id: String,
    artifact_id: ArtifactId,
    payload_hash: String,
}

impl VerifiedExternalSourceV1 {
    pub fn record_id(&self) -> &str {
        &self.record_id
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExternalSourceAdmissionErrorV1 {
    #[error("source-issuer record has invalid revision, identity, digest or validity interval")]
    InvalidRecord,
    #[error("source-issuer record was issued in the future")]
    NotYetValid,
    #[error("source-issuer record is expired")]
    Expired,
    #[error("source-issuer record is revoked")]
    Revoked,
    #[error("source-issuer record does not match authenticated tenant, run or context")]
    ScopeMismatch,
    #[error("source-issuer record does not match the immutable DAG plan")]
    PlanMismatch,
    #[error("source-issuer record does not match a bound external artifact")]
    ArtifactMismatch,
    #[error("external artifact bytes differ from immutable source-issuer digest")]
    PayloadMismatch,
    #[error("external artifact provenance differs from immutable source-issuer identity")]
    ProvenanceMismatch,
}

fn canonical_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_non_nil_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .map(|id| !id.is_nil() && id.to_string() == value)
        .unwrap_or(false)
}

/// Verify source identity, tenant/context, scope, plan hash, typed schema,
/// provenance, exact bytes and a live, non-revoked issuer window. `now` is a
/// trusted operator clock supplied *outside* Temporal workflow replay.
///
/// The `record` MUST come from a separately authorized issuer query; a
/// matching caller-provided record has zero authority. A privileged operator
/// must also validate the business-run issuer, allowed operation/route and
/// original V6 plan independently before calling Temporal Start.
pub fn verify_external_source_record_v1(
    record: &TrustedExternalSourceRecordV1,
    run: &OperationDagRunV6,
    context_key: &str,
    artifact_id: &ArtifactId,
    now_unix_seconds: i64,
) -> Result<VerifiedExternalSourceV1, ExternalSourceAdmissionErrorV1> {
    if record.schema_revision != EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1
        || !canonical_non_nil_uuid(&record.record_id)
        || !canonical_non_nil_uuid(&record.run_id)
        || record.tenant_id.trim().is_empty()
        || record.context_key.trim().is_empty()
        || !canonical_digest(&record.payload_hash)
        || !canonical_digest(&record.plan_hash.0)
        || record.issued_at_unix_seconds < 0
        || record.expires_at_unix_seconds <= record.issued_at_unix_seconds
    {
        return Err(ExternalSourceAdmissionErrorV1::InvalidRecord);
    }
    if now_unix_seconds < record.issued_at_unix_seconds {
        return Err(ExternalSourceAdmissionErrorV1::NotYetValid);
    }
    if now_unix_seconds >= record.expires_at_unix_seconds {
        return Err(ExternalSourceAdmissionErrorV1::Expired);
    }
    // Revocation is unconditional even if the timestamp is in the future:
    // the authoritative issuer store has already marked this grant revoked.
    if record.revoked_at_unix_seconds.is_some() {
        return Err(ExternalSourceAdmissionErrorV1::Revoked);
    }
    if run.plan.scope.tenant_id != record.tenant_id
        || run.plan.scope.run_id != record.run_id
        || context_key != record.context_key
    {
        return Err(ExternalSourceAdmissionErrorV1::ScopeMismatch);
    }
    if run.schema_revision != OPERATION_DAG_RUN_SCHEMA_REVISION_V6
        || run.plan.hash != record.plan_hash
    {
        return Err(ExternalSourceAdmissionErrorV1::PlanMismatch);
    }
    if artifact_id != &record.artifact_id {
        return Err(ExternalSourceAdmissionErrorV1::ArtifactMismatch);
    }
    let planned = run
        .plan
        .external_artifacts
        .get(artifact_id)
        .ok_or(ExternalSourceAdmissionErrorV1::ArtifactMismatch)?;
    let admitted = run
        .admitted
        .get(artifact_id)
        .ok_or(ExternalSourceAdmissionErrorV1::ArtifactMismatch)?;
    let actually_bound = run.plan.nodes.iter().any(|node| {
        node.inputs.values().any(|input| {
            matches!(input, ArtifactBinding::External { artifact } if artifact == artifact_id)
        })
    });
    if !actually_bound
        || planned.schema != record.schema
        || admitted.schema != record.schema
        || planned.scope != run.plan.scope
        || admitted.scope != run.plan.scope
    {
        return Err(ExternalSourceAdmissionErrorV1::ArtifactMismatch);
    }
    let expected_provenance = format!("{PROVENANCE_PREFIX}{}", record.record_id);
    if planned.provenance != expected_provenance || admitted.provenance != expected_provenance {
        return Err(ExternalSourceAdmissionErrorV1::ProvenanceMismatch);
    }
    if owner_content_hash(&admitted.payload) != record.payload_hash {
        return Err(ExternalSourceAdmissionErrorV1::PayloadMismatch);
    }
    Ok(VerifiedExternalSourceV1 {
        record_id: record.record_id.clone(),
        artifact_id: artifact_id.clone(),
        payload_hash: record.payload_hash.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::{
        AdmittedArtifactV1, ArtifactTypeId, ContractCatalog, ExecutionPlan, ExternalArtifact,
        IdempotencyPolicy, OperationContract, OperationId, OperationInstanceId, OperationNode,
        OperationOwnerId, OperationRef, PortId, RunScope, PLAN_SCHEMA_REVISION,
    };

    const NOW: i64 = 1_900_000_100;

    fn fixture() -> (TrustedExternalSourceRecordV1, OperationDagRunV6) {
        let record_id = "0e2dddf2-0f45-46ed-89dd-3dce7e8b247d";
        let provenance = format!("source-record-v1:{record_id}");
        let scope = RunScope {
            tenant_id: "tenant-primary".into(),
            run_id: "befd8f0c-8f83-4b29-8c4b-9c5a7694a328".into(),
        };
        let schema = ArtifactSchema {
            id: ArtifactTypeId("acquisition.source_page.v1".into()),
            revision: 1,
        };
        let operation = OperationRef {
            id: OperationId("acquisition.classify_source_page".into()),
            revision: 1,
        };
        let mut catalog = ContractCatalog::default();
        catalog
            .register_contract(OperationContract {
                reference: operation.clone(),
                owner: OperationOwnerId("acquisition".into()),
                inputs: BTreeMap::from([(PortId("input".into()), schema.clone())]),
                outputs: BTreeMap::from([(
                    PortId("output".into()),
                    ArtifactSchema {
                        id: ArtifactTypeId("acquisition.classification.v1".into()),
                        revision: 1,
                    },
                )]),
                invalidates: BTreeSet::new(),
                effects: BTreeSet::new(),
                idempotency: IdempotencyPolicy::Pure,
            })
            .unwrap();
        let source_id = ArtifactId("source".into());
        let payload =
            br#"{"url":"https://example.test","title":"Trusted","raw_text":"Real source"}"#
                .to_vec();
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope.clone(),
            allowed_effects: BTreeSet::new(),
            external_artifacts: BTreeMap::from([(
                source_id.clone(),
                ExternalArtifact {
                    schema: schema.clone(),
                    scope: scope.clone(),
                    provenance: provenance.clone(),
                },
            )]),
            pinned_contracts: Vec::new(),
            nodes: vec![OperationNode {
                instance: OperationInstanceId("one_step".into()),
                operation,
                inputs: BTreeMap::from([(
                    PortId("input".into()),
                    ArtifactBinding::External {
                        artifact: source_id.clone(),
                    },
                )]),
                outputs: BTreeMap::from([(PortId("output".into()), ArtifactId("result".into()))]),
                after: BTreeSet::new(),
                permitted_effects: BTreeSet::new(),
            }],
            hash: PlanHash(String::new()),
        }
        .seal(&catalog)
        .unwrap();
        let record = TrustedExternalSourceRecordV1 {
            schema_revision: EXTERNAL_SOURCE_ADMISSION_SCHEMA_REVISION_V1,
            record_id: record_id.into(),
            tenant_id: scope.tenant_id.clone(),
            context_key: "context-authenticated".into(),
            run_id: scope.run_id.clone(),
            plan_hash: plan.hash.clone(),
            artifact_id: source_id.clone(),
            schema: schema.clone(),
            payload_hash: owner_content_hash(&payload),
            issued_at_unix_seconds: NOW - 100,
            expires_at_unix_seconds: NOW + 300,
            revoked_at_unix_seconds: None,
        };
        let run = OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::from([(
                source_id,
                AdmittedArtifactV1 {
                    schema,
                    scope,
                    provenance,
                    payload,
                },
            )]),
            owner_commands: BTreeMap::new(),
            pinned_routes: Vec::new(),
        };
        (record, run)
    }

    fn verify(
        record: &TrustedExternalSourceRecordV1,
        run: &OperationDagRunV6,
        context_key: &str,
        now: i64,
    ) -> Result<VerifiedExternalSourceV1, ExternalSourceAdmissionErrorV1> {
        verify_external_source_record_v1(record, run, context_key, &record.artifact_id, now)
    }

    #[test]
    fn exact_issuer_row_authenticates_typed_source_bytes() {
        let (record, run) = fixture();
        let checked = verify(&record, &run, "context-authenticated", NOW).unwrap();
        assert_eq!(checked.record_id(), record.record_id);
        assert_eq!(checked.artifact_id(), &record.artifact_id);
        assert_eq!(checked.payload_hash(), record.payload_hash);
    }

    #[test]
    fn changed_bytes_and_forged_provenance_fail_closed() {
        let (record, mut run) = fixture();
        run.admitted.get_mut(&record.artifact_id).unwrap().payload[0] ^= 1;
        assert_eq!(
            verify(&record, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::PayloadMismatch)
        );
        let (record, mut run) = fixture();
        run.admitted
            .get_mut(&record.artifact_id)
            .unwrap()
            .provenance = "self-attested-in-plan".into();
        assert_eq!(
            verify(&record, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ProvenanceMismatch)
        );
        let (record, mut run) = fixture();
        run.plan
            .external_artifacts
            .get_mut(&record.artifact_id)
            .unwrap()
            .provenance = "self-attested-in-plan".into();
        assert_eq!(
            verify(&record, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ProvenanceMismatch)
        );
    }

    #[test]
    fn cross_tenant_context_run_and_plan_are_denied() {
        let (record, run) = fixture();
        assert_eq!(
            verify(&record, &run, "another-context", NOW),
            Err(ExternalSourceAdmissionErrorV1::ScopeMismatch)
        );
        let mut cross_tenant = record.clone();
        cross_tenant.tenant_id = "another-tenant".into();
        assert_eq!(
            verify(&cross_tenant, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ScopeMismatch)
        );
        let mut cross_run = record.clone();
        cross_run.run_id = "different-run".into();
        assert_eq!(
            verify(&cross_run, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ScopeMismatch)
        );
        let mut other_plan = record.clone();
        other_plan.plan_hash = PlanHash("a".repeat(64));
        assert_eq!(
            verify(&other_plan, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::PlanMismatch)
        );
    }

    #[test]
    fn expiry_revocation_and_future_issue_are_denied() {
        let (record, run) = fixture();
        assert_eq!(
            verify(
                &record,
                &run,
                "context-authenticated",
                record.expires_at_unix_seconds
            ),
            Err(ExternalSourceAdmissionErrorV1::Expired)
        );
        let mut revoked = record.clone();
        revoked.revoked_at_unix_seconds = Some(NOW - 30);
        assert_eq!(
            verify(&revoked, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::Revoked)
        );
        let mut future_issued = record.clone();
        future_issued.issued_at_unix_seconds = NOW + 50;
        assert_eq!(
            verify(&future_issued, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::NotYetValid)
        );
    }

    #[test]
    fn missing_binding_and_bad_issuer_identity_are_denied() {
        let (record, run) = fixture();
        let mut forged_record = record.clone();
        forged_record.record_id = "not-a-canonical-uuid".into();
        assert_eq!(
            verify(&forged_record, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::InvalidRecord)
        );
        let mut invalid_run = record.clone();
        invalid_run.run_id = "non-uuid-run".into();
        assert_eq!(
            verify(&invalid_run, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::InvalidRecord)
        );
        let mut malformed_hash = record.clone();
        malformed_hash.payload_hash = "A".repeat(64);
        assert_eq!(
            verify(&malformed_hash, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::InvalidRecord)
        );
        let mut altered_schema = record.clone();
        altered_schema.schema.revision = 3;
        assert_eq!(
            verify(&altered_schema, &run, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ArtifactMismatch)
        );
        let mut missing = run;
        missing.plan.nodes[0].inputs.clear();
        assert_eq!(
            verify(&record, &missing, "context-authenticated", NOW),
            Err(ExternalSourceAdmissionErrorV1::ArtifactMismatch)
        );
    }
}
