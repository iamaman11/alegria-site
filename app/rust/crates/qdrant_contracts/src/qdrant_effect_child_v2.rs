//! Versioned, provider-free V6-DAG transport for the Qdrant effect owner.
//!
//! Unlike the legacy Qdrant-only V1 parent, this invocation binds the
//! independently admitted child to a checked, immutable operation-DAG node.
//! A self-consistent invocation is *not* permission to perform provider I/O.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use operation_protocol::{
    canonical_effect_dispatch_input_bytes, checked_execution_key, ArtifactSchema, ArtifactTypeId,
    BoundEffectInputV4, CheckedDispatch, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId, TemporalExecutionIdentityV4,
};

use super::qdrant_effect_child_v1::{QdrantChildOutcomeV1, QdrantKeywordProjectionCommandV1};

pub const QDRANT_OWNER_CHILD_TYPE_V2: &str = "QdrantKeywordProjectionV2Workflow";
pub const QDRANT_OWNER_COMMAND_QUEUE_V2: &str = "alegria-qdrant-command-v2";
pub const QDRANT_OWNER_ACTIVITY_QUEUE_V2: &str = "alegria-qdrant-effects-v2";
pub const QDRANT_CHILD_SCHEMA_REVISION_V2: u16 = 2;
pub const QDRANT_EFFECT_OPERATION_ID_V2: &str = "qdrant.keyword_projection.upsert";
pub const QDRANT_EFFECT_OWNER_ID_V2: &str = "qdrant_effects_core";
pub const QDRANT_EFFECT_INPUT_PORT_V2: &str = "keyword_command";
pub const QDRANT_EFFECT_OUTPUT_PORT_V2: &str = "projection_receipt";

/// A provider-free owner contract: a Qdrant addition does not change DAG
/// validation, the PostgreSQL contract, or the temporal orchestration kernel.
pub struct QdrantKeywordProjectionContractV2;

impl Operation for QdrantKeywordProjectionContractV2 {
    type Input = QdrantKeywordProjectionCommandV1;
    type Output = QdrantKeywordChildResultV2;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(QDRANT_EFFECT_OPERATION_ID_V2.to_owned()),
                revision: 2,
            },
            owner: OperationOwnerId(QDRANT_EFFECT_OWNER_ID_V2.to_owned()),
            inputs: BTreeMap::from([(
                PortId(QDRANT_EFFECT_INPUT_PORT_V2.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId("qdrant.keyword_projection_command".to_owned()),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId(QDRANT_EFFECT_OUTPUT_PORT_V2.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId("qdrant.keyword_projection_receipt".to_owned()),
                    revision: 2,
                },
            )]),
            invalidates: BTreeSet::from([ArtifactTypeId(
                "qdrant.keyword_projection_snapshot".to_owned(),
            )]),
            effects: BTreeSet::from([EffectId(QDRANT_EFFECT_OPERATION_ID_V2.to_owned())]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantKeywordChildInvocationV2 {
    pub schema_revision: u16,
    pub parent: TemporalExecutionIdentityV4,
    pub canonical_run_id: Uuid,
    pub checked_dispatch: CheckedDispatch,
    pub bound_input: BoundEffectInputV4,
    pub command: QdrantKeywordProjectionCommandV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantKeywordChildResultV2 {
    pub schema_revision: u16,
    pub admission_id: Uuid,
    pub dispatch_execution_key: String,
    pub outcome: QdrantChildOutcomeV1,
}

#[derive(Debug, Error)]
pub enum QdrantChildProtocolErrorV2 {
    #[error("unsupported Qdrant child revision {0}")]
    SchemaRevision(u16),
    #[error("invalid Qdrant child field: {0}")]
    InvalidField(&'static str),
    #[error("Qdrant child artifact or command differs from checked dispatch")]
    InputMismatch,
    #[error("Qdrant child scope differs from checked dispatch")]
    ScopeMismatch,
    #[error("Qdrant child cannot serialize its typed command: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Qdrant child dispatch integrity failed: {0}")]
    Dispatch(#[from] operation_protocol::EffectChildProtocolError),
}

impl QdrantKeywordChildInvocationV2 {
    pub fn canonical_dispatch_input_bytes(&self) -> Result<Vec<u8>, QdrantChildProtocolErrorV2> {
        let command = serde_json::to_vec(&self.command)?;
        Ok(canonical_effect_dispatch_input_bytes(
            &self.bound_input,
            &command,
        )?)
    }

    /// Identity and canonical byte-level self-consistency only. The provider
    /// separately validates both server histories, operator admission,
    /// canonical source revision and the exact durable effect receipt.
    pub fn verify_integrity(&self) -> Result<(), QdrantChildProtocolErrorV2> {
        if self.schema_revision != QDRANT_CHILD_SCHEMA_REVISION_V2 {
            return Err(QdrantChildProtocolErrorV2::SchemaRevision(
                self.schema_revision,
            ));
        }
        if self.canonical_run_id.is_nil()
            || self.parent.workflow_id.trim().is_empty()
            || self.parent.run_id.trim().is_empty()
            || self.checked_dispatch.execution_key.trim().is_empty()
        {
            return Err(QdrantChildProtocolErrorV2::InvalidField(
                "execution identity",
            ));
        }
        self.command
            .validate()
            .map_err(QdrantChildProtocolErrorV2::InvalidField)?;
        // The payload hash alone cannot constrain what the caller *calls*
        // this effect. Check the independently pinned owner contract, not
        // merely a self-consistent execution key supplied by the parent.
        let contract = QdrantKeywordProjectionContractV2::contract();
        if self.checked_dispatch.operation != contract.reference
            || self.checked_dispatch.owner != contract.owner
            || self.checked_dispatch.effects != contract.effects
            || self.checked_dispatch.idempotency != contract.idempotency
            || self.bound_input.schema
                != contract.inputs[&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2)]
        {
            return Err(QdrantChildProtocolErrorV2::InputMismatch);
        }
        if self.checked_dispatch.scope != self.bound_input.scope
            || self.checked_dispatch.scope.tenant_id != self.command.tenant_id
            || self.checked_dispatch.scope.run_id != self.canonical_run_id.to_string()
        {
            return Err(QdrantChildProtocolErrorV2::ScopeMismatch);
        }
        let parsed: QdrantKeywordProjectionCommandV1 =
            serde_json::from_slice(&self.bound_input.payload)?;
        if parsed != self.command {
            return Err(QdrantChildProtocolErrorV2::InputMismatch);
        }
        let bytes = self.canonical_dispatch_input_bytes()?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if hash != self.checked_dispatch.input_hash
            || checked_execution_key(&self.checked_dispatch, &hash)?
                != self.checked_dispatch.execution_key
        {
            return Err(QdrantChildProtocolErrorV2::InputMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{ArtifactId, OperationInstanceId, PlanHash, RunScope};

    fn valid_invocation() -> QdrantKeywordChildInvocationV2 {
        let canonical_run_id = Uuid::new_v4();
        let command = QdrantKeywordProjectionCommandV1 {
            schema_revision: 1,
            admission_id: Uuid::new_v4(),
            tenant_id: "tenant-a".into(),
            context_key: "context-a".into(),
            keyword_key: "keyword-a".into(),
            vector: vec![0.25; 1024],
        };
        let contract = QdrantKeywordProjectionContractV2::contract();
        let scope = RunScope {
            tenant_id: command.tenant_id.clone(),
            run_id: canonical_run_id.to_string(),
        };
        let bound_input = BoundEffectInputV4 {
            artifact_id: ArtifactId::from("command-a"),
            schema: contract.inputs[&PortId::from(QDRANT_EFFECT_INPUT_PORT_V2)].clone(),
            scope: scope.clone(),
            provenance: "test-admitted-command".into(),
            payload: serde_json::to_vec(&command).unwrap(),
        };
        let mut invocation = QdrantKeywordChildInvocationV2 {
            schema_revision: QDRANT_CHILD_SCHEMA_REVISION_V2,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: canonical_run_id.to_string(),
                run_id: "actual-temporal-parent-run".into(),
            },
            canonical_run_id,
            checked_dispatch: CheckedDispatch {
                plan_hash: PlanHash::from("plan-a"),
                scope,
                instance: OperationInstanceId::from("qdrant-1"),
                operation: contract.reference,
                owner: contract.owner,
                effects: contract.effects,
                idempotency: contract.idempotency,
                input_hash: String::new(),
                execution_key: String::new(),
            },
            bound_input,
            command,
        };
        let bytes = invocation.canonical_dispatch_input_bytes().unwrap();
        invocation.checked_dispatch.input_hash = blake3::hash(&bytes).to_hex().to_string();
        invocation.checked_dispatch.execution_key = checked_execution_key(
            &invocation.checked_dispatch,
            &invocation.checked_dispatch.input_hash,
        )
        .unwrap();
        invocation
    }

    #[test]
    fn valid_qdrant_v2_contract_and_byte_binding() {
        valid_invocation().verify_integrity().unwrap();
    }

    #[test]
    fn rejects_forged_owner_operation_and_effect_even_with_matching_execution_key() {
        let mut actual = valid_invocation();
        actual.checked_dispatch.owner =
            operation_protocol::OperationOwnerId::from("postgres_store");
        actual.checked_dispatch.execution_key = checked_execution_key(
            &actual.checked_dispatch,
            &actual.checked_dispatch.input_hash,
        )
        .unwrap();
        assert!(actual.verify_integrity().is_err());

        let mut actual = valid_invocation();
        actual.checked_dispatch.operation.revision += 1;
        actual.checked_dispatch.execution_key = checked_execution_key(
            &actual.checked_dispatch,
            &actual.checked_dispatch.input_hash,
        )
        .unwrap();
        assert!(actual.verify_integrity().is_err());

        let mut actual = valid_invocation();
        actual.checked_dispatch.effects.clear();
        actual.checked_dispatch.execution_key = checked_execution_key(
            &actual.checked_dispatch,
            &actual.checked_dispatch.input_hash,
        )
        .unwrap();
        assert!(actual.verify_integrity().is_err());
    }

    #[test]
    fn rejects_changed_qdrant_command_or_artifact() {
        let mut actual = valid_invocation();
        actual.command.keyword_key = "other-key".into();
        assert!(actual.verify_integrity().is_err());

        let mut actual = valid_invocation();
        actual.bound_input.schema.revision += 1;
        assert!(actual.verify_integrity().is_err());
    }
}
