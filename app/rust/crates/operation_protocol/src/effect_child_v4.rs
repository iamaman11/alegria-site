//! Canonical provider-free Temporal child invocation / checked dispatch ABI.
//!
//! A checked invocation is intentionally NOT an admission. An effect owner
//! must independently prove the *actual* Temporal parent/child Run IDs, the
//! admitted source revision, operation contract, command and durable receipt.

use serde::{Deserialize, Serialize};

use crate::{ArtifactId, ArtifactSchema, CheckedDispatch, RunScope};

pub const EFFECT_CHILD_SCHEMA_REVISION: u16 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalExecutionIdentityV4 {
    pub workflow_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundEffectInputV4 {
    pub artifact_id: ArtifactId,
    pub schema: ArtifactSchema,
    pub scope: RunScope,
    pub provenance: String,
    /// Exact immutable producer bytes captured in the parent History. The
    /// source revision MUST still be independently checked by the owner.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectChildInvocationV4 {
    pub schema_revision: u16,
    /// Immutable declaration from parent History; separately compare both
    /// Temporal Histories before privileged operator admission.
    pub parent: TemporalExecutionIdentityV4,
    pub checked_dispatch: CheckedDispatch,
    pub bound_input: BoundEffectInputV4,
    /// Exact JSON serialization of a versioned *typed owner command*. For
    /// Truth reconciliation it is `ReconcileTruthReviewsInput`, including
    /// admission_id. Do not interpret as freeform SQL or generic RPC data.
    pub owner_command_bytes: Vec<u8>,
}

/// Typed owner result envelope. The child Workflow returns this as a JSON
/// string in its Temporal completion History; `pg_receipt` is the genuine
/// durable owner receipt, *not* a caller-supplied success bit. The parent
/// verifies schema, dispatch key and all receipt fields before continuing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectChildResultV4<T> {
    pub schema_revision: u16,
    pub dispatch_execution_key: String,
    pub pg_receipt: T,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EffectChildProtocolError {
    #[error("unsupported effect child invocation revision {0}")]
    UnsupportedSchemaRevision(u16),
    #[error("invalid Temporal parent or owner effect field: {0}")]
    EmptyField(&'static str),
    #[error("child effect scope differs from checked dispatch scope")]
    ScopeMismatch,
    #[error("child effect input bytes have a different BLAKE3 fingerprint")]
    InputHashMismatch,
    #[error("child effect execution key differs from canonical checked dispatch")]
    ExecutionKeyMismatch,
    #[error("child effect invocation JSON serialization failed: {0}")]
    Serialization(String),
}

impl EffectChildInvocationV4 {
    /// Wire- and history-stable SHA input in THIS order. Include the schema
    /// revision, complete input envelope and typed command bytes: neither a
    /// claimed digest nor mutable admission state may replace these bytes.
    pub fn canonical_dispatch_input_bytes(&self) -> Result<Vec<u8>, EffectChildProtocolError> {
        canonical_effect_dispatch_input_bytes(&self.bound_input, &self.owner_command_bytes)
    }

    /// Self-consistency ONLY. It proves that the dispatch key describes these
    /// exact bytes; it does NOT prove operator admission or source provenance.
    pub fn verify_integrity(&self) -> Result<(), EffectChildProtocolError> {
        if self.schema_revision != EFFECT_CHILD_SCHEMA_REVISION {
            return Err(EffectChildProtocolError::UnsupportedSchemaRevision(
                self.schema_revision,
            ));
        }
        for (label, value) in [
            ("parent.workflow_id", self.parent.workflow_id.as_str()),
            ("parent.run_id", self.parent.run_id.as_str()),
            ("artifact.provenance", self.bound_input.provenance.as_str()),
            (
                "checked.plan_hash",
                self.checked_dispatch.plan_hash.as_str(),
            ),
            ("checked.instance", self.checked_dispatch.instance.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(EffectChildProtocolError::EmptyField(label));
            }
        }
        if self.owner_command_bytes.is_empty() {
            return Err(EffectChildProtocolError::EmptyField("owner_command_bytes"));
        }
        if self.bound_input.scope != self.checked_dispatch.scope {
            return Err(EffectChildProtocolError::ScopeMismatch);
        }
        let input_bytes = self.canonical_dispatch_input_bytes()?;
        let actual_input_hash = blake3::hash(&input_bytes).to_hex().to_string();
        if actual_input_hash != self.checked_dispatch.input_hash {
            return Err(EffectChildProtocolError::InputHashMismatch);
        }
        let expected_key = checked_execution_key(&self.checked_dispatch, &actual_input_hash)?;
        if expected_key != self.checked_dispatch.execution_key {
            return Err(EffectChildProtocolError::ExecutionKeyMismatch);
        }
        Ok(())
    }
}

/// Stable, provider-independent serializer shared by plan preflight and the
/// SQL effect owner. The attached schema/provenance is hashed but still needs
/// independently verified source attestation before it authorizes any write.
pub fn canonical_effect_dispatch_input_bytes(
    bound_input: &BoundEffectInputV4,
    owner_command_bytes: &[u8],
) -> Result<Vec<u8>, EffectChildProtocolError> {
    serde_json::to_vec(&(
        EFFECT_CHILD_SCHEMA_REVISION,
        bound_input,
        owner_command_bytes,
    ))
    .map_err(|error| EffectChildProtocolError::Serialization(error.to_string()))
}

/// Exactly matches `ValidatedPlan::execution_key`. This public verifier is
/// needed by the independent PostgreSQL effect owner; it must still compare
/// the operation identity and plan fingerprint against operator-issued facts.
pub fn checked_execution_key(
    dispatch: &CheckedDispatch,
    input_hash: &str,
) -> Result<String, EffectChildProtocolError> {
    let encoded = serde_json::to_vec(&(
        &dispatch.scope,
        &dispatch.plan_hash,
        &dispatch.instance,
        &dispatch.operation,
        input_hash,
    ))
    .map_err(|error| EffectChildProtocolError::Serialization(error.to_string()))?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        EffectId, IdempotencyPolicy, OperationId, OperationInstanceId, OperationOwnerId,
        OperationRef, PlanHash,
    };

    fn invocation() -> EffectChildInvocationV4 {
        let bound_input = BoundEffectInputV4 {
            artifact_id: "verified_scope".into(),
            schema: ArtifactSchema {
                id: "truth.candidate_review_scope".into(),
                revision: 1,
            },
            scope: RunScope {
                tenant_id: "untrusted-tenant-label".into(),
                run_id: "canonical-run-id".into(),
            },
            provenance: "source:verified-revision".into(),
            payload: br#"{"run_id":"canonical-run-id","context_key":"ctx"}"#.to_vec(),
        };
        let checked_dispatch = CheckedDispatch {
            plan_hash: PlanHash("planhash".into()),
            scope: bound_input.scope.clone(),
            instance: OperationInstanceId::from("review"),
            operation: OperationRef {
                id: OperationId::from("pg.truth.reconcile_candidate_reviews"),
                revision: 1,
            },
            owner: OperationOwnerId::from("postgres_store"),
            effects: BTreeSet::from([EffectId::from("pg.truth.review_queue.reconcile")]),
            idempotency: IdempotencyPolicy::Idempotent,
            input_hash: String::new(),
            execution_key: String::new(),
        };
        let mut invocation = EffectChildInvocationV4 {
            schema_revision: EFFECT_CHILD_SCHEMA_REVISION,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: "canonical-run-id".into(),
                run_id: "server-parent-run-id".into(),
            },
            checked_dispatch,
            bound_input,
            owner_command_bytes: br#"{"operation_instance":"review"}"#.to_vec(),
        };
        invocation.checked_dispatch.input_hash =
            blake3::hash(&invocation.canonical_dispatch_input_bytes().unwrap())
                .to_hex()
                .to_string();
        invocation.checked_dispatch.execution_key = checked_execution_key(
            &invocation.checked_dispatch,
            &invocation.checked_dispatch.input_hash,
        )
        .unwrap();
        invocation
    }

    #[test]
    fn exact_payload_digest_is_reproducible_and_detects_all_mutations() {
        let original = invocation();
        original.verify_integrity().unwrap();

        let mut changed = original.clone();
        changed.bound_input.payload.push(b' ');
        assert_eq!(
            changed.verify_integrity(),
            Err(EffectChildProtocolError::InputHashMismatch)
        );
        let mut changed = original.clone();
        changed.owner_command_bytes.push(b' ');
        assert_eq!(
            changed.verify_integrity(),
            Err(EffectChildProtocolError::InputHashMismatch)
        );
        let mut changed = original.clone();
        changed.bound_input.provenance.push_str("/forged");
        assert_eq!(
            changed.verify_integrity(),
            Err(EffectChildProtocolError::InputHashMismatch)
        );
        let mut changed = original.clone();
        changed.checked_dispatch.execution_key.push('f');
        assert_eq!(
            changed.verify_integrity(),
            Err(EffectChildProtocolError::ExecutionKeyMismatch)
        );
        let mut changed = original;
        changed.parent.run_id.clear();
        assert_eq!(
            changed.verify_integrity(),
            Err(EffectChildProtocolError::EmptyField("parent.run_id"))
        );
    }
}
