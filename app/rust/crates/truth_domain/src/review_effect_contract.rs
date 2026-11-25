//! Provider-free, versioned contract for the existing Truth review queue effect.
//!
//! This is the canonical declaration of *what* the PostgreSQL owner can do,
//! not authority to perform it. An independently issued, transactionally
//! consumed PostgreSQL admission and an attested Temporal Activity identity
//! are still mandatory; a self-consistent plan is never an admission.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId, RunScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const OPERATION_ID: &str = "pg.truth.reconcile_candidate_reviews";
pub const OPERATION_REVISION: u32 = 1;
pub const OWNER_ID: &str = "postgres_store";
pub const EFFECT_ID: &str = "pg.truth.review_queue.reconcile";
pub const INPUT_PORT: &str = "truth_review_scope";
pub const OUTPUT_PORT: &str = "review_queue_receipt";

/// Exact JSON payload shape of the existing V3 owner command. The provider
/// still independently verifies all fields against the PG admission and the
/// actual server-issued Workflow Run ID; none is a bearer capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileTruthReviewsInput {
    pub admission_id: Uuid,
    pub run_id: Uuid,
    pub scope: RunScope,
    pub context_key: String,
    pub operation_instance: String,
    pub plan_hash: String,
}

/// The versioned *data* consumed by the effect graph, distinct from the
/// operator-issued command. This envelope is still caller-supplied; PG must
/// verify source provenance and revision under its own lock before effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TruthReviewScopeArtifactV1 {
    pub run_id: Uuid,
    pub context_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileTruthReviewsReceipt {
    pub receipt_id: Uuid,
    pub admission_id: Uuid,
    pub execution_key: String,
    pub run_id: Uuid,
    pub context_key: String,
    pub candidate_set_hash: String,
    pub created_review_tasks: i64,
    pub open_review_tasks: i64,
}

/// Contract-only typed marker. Business execution remains exclusively in the
/// PostgreSQL owner; the provider-free orchestration package never calls SQL.
pub struct ReconcileTruthReviewsContractV1;

impl Operation for ReconcileTruthReviewsContractV1 {
    type Input = ReconcileTruthReviewsInput;
    type Output = ReconcileTruthReviewsReceipt;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        operation_contract()
    }
}

pub fn operation_contract() -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId(OPERATION_ID.to_owned()),
            revision: OPERATION_REVISION,
        },
        owner: OperationOwnerId(OWNER_ID.to_owned()),
        inputs: BTreeMap::from([(
            PortId(INPUT_PORT.to_owned()),
            ArtifactSchema {
                id: ArtifactTypeId("truth.candidate_review_scope".to_owned()),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId(OUTPUT_PORT.to_owned()),
            ArtifactSchema {
                id: ArtifactTypeId("truth.review_queue_receipt".to_owned()),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::from([ArtifactTypeId("truth.review_queue_snapshot".to_owned())]),
        effects: BTreeSet::from([EffectId(EFFECT_ID.to_owned())]),
        idempotency: IdempotencyPolicy::Idempotent,
    }
}

#[cfg(test)]
#[path = "review_effect_contract/tests.rs"]
mod tests;
