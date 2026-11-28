//! Strict, independently composable **pure** Truth decisions (TR-02/TR-03).
//!
//! Existing permissive V1 DTOs remain untouched for historical replay. This
//! owner-issued v1 boundary rejects unknown fields at every nesting level;
//! source access, PostgreSQL Truth writes, HITL and Temporal admission are
//! explicitly outside these deterministic operations.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId, OperationOwnerId,
    OperationRef, PortId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{completeness, contradictions};

pub const TRUTH_PURE_OWNER: &str = "truth_domain";

/// Strict owner boundary. A future adapter may check these DTOs as single
/// Temporal DAG nodes; registration here alone grants no execution capability.
pub trait PureTruthOperation: Operation<Failure = Infallible> {
    fn evaluate(input: &Self::Input) -> Self::Output;
}

fn pure_contract(id: &str, input_type: &str, output_type: &str) -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId::from(id),
            revision: 1,
        },
        owner: OperationOwnerId::from(TRUTH_PURE_OWNER),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            ArtifactSchema {
                id: input_type.into(),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            ArtifactSchema {
                id: output_type.into(),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::new(),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompletenessInputV1 {
    pub section_id: String,
    pub raw_text: String,
    pub source_numeric_tokens: Vec<String>,
    pub extracted_numeric_tokens: Vec<String>,
    pub extracted_rule_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MissingElementV1 {
    pub loss_type: String,
    pub raw_fragment: String,
    pub reason: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompletenessOutputV1 {
    pub completeness_score: f32,
    pub missing_elements: Vec<MissingElementV1>,
    pub needs_hitl: bool,
    pub hitl_reason: Option<String>,
}

/// TR-02: completeness is a pure *assessment*, not a Truth revision update.
pub struct JudgeCompletenessV1;

impl Operation for JudgeCompletenessV1 {
    type Input = CompletenessInputV1;
    type Output = CompletenessOutputV1;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "truth.judge_completeness",
            "truth.completeness_input",
            "truth.completeness_output",
        )
    }
}

impl PureTruthOperation for JudgeCompletenessV1 {
    fn evaluate(input: &Self::Input) -> Self::Output {
        let assessed = completeness::execute(&completeness::CompletenessJudgeInput {
            section_id: input.section_id.clone(),
            raw_text: input.raw_text.clone(),
            source_numeric_tokens: input.source_numeric_tokens.clone(),
            extracted_numeric_tokens: input.extracted_numeric_tokens.clone(),
            extracted_rule_keys: input.extracted_rule_keys.clone(),
        });
        CompletenessOutputV1 {
            completeness_score: assessed.completeness_score,
            missing_elements: assessed
                .missing_elements
                .into_iter()
                .map(|missing| MissingElementV1 {
                    loss_type: missing.loss_type,
                    raw_fragment: missing.raw_fragment,
                    reason: missing.reason,
                    action: missing.action,
                })
                .collect(),
            needs_hitl: assessed.needs_hitl,
            hitl_reason: assessed.hitl_reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactAssertionV1 {
    pub subject_key: String,
    pub predicate_key: String,
    pub value_normalized: String,
    pub source_key: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContradictionInputV1 {
    pub run_id: String,
    pub facts: Vec<FactAssertionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConflictCaseV1 {
    pub subject_key: String,
    pub predicate_key: String,
    pub value_left: String,
    pub value_right: String,
    pub source_left: Option<String>,
    pub source_right: Option<String>,
    pub confidence_left: f32,
    pub confidence_right: f32,
    pub severity: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContradictionOutputV1 {
    pub is_blocked: bool,
    pub conflict_count: usize,
    pub conflicts: Vec<ConflictCaseV1>,
    pub needs_hitl: bool,
    pub hitl_reason: Option<String>,
    pub decision: String,
}

/// TR-03: read-only contradiction classification, no publish veto write.
pub struct DetectContradictionsV1;

impl Operation for DetectContradictionsV1 {
    type Input = ContradictionInputV1;
    type Output = ContradictionOutputV1;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "truth.detect_contradictions",
            "truth.contradiction_input",
            "truth.contradiction_output",
        )
    }
}

impl PureTruthOperation for DetectContradictionsV1 {
    fn evaluate(input: &Self::Input) -> Self::Output {
        let assessed = contradictions::execute(&contradictions::ContradictionGateInput {
            run_id: input.run_id.clone(),
            facts: input
                .facts
                .iter()
                .map(|fact| contradictions::FactAssertion {
                    subject_key: fact.subject_key.clone(),
                    predicate_key: fact.predicate_key.clone(),
                    value_normalized: fact.value_normalized.clone(),
                    source_key: fact.source_key.clone(),
                    confidence: fact.confidence,
                })
                .collect(),
        });
        ContradictionOutputV1 {
            is_blocked: assessed.is_blocked,
            conflict_count: assessed.conflict_count,
            conflicts: assessed
                .conflicts
                .into_iter()
                .map(|case| ConflictCaseV1 {
                    subject_key: case.subject_key,
                    predicate_key: case.predicate_key,
                    value_left: case.value_left,
                    value_right: case.value_right,
                    source_left: case.source_left,
                    source_right: case.source_right,
                    confidence_left: case.confidence_left,
                    confidence_right: case.confidence_right,
                    severity: case.severity,
                    reason: case.reason,
                })
                .collect(),
            needs_hitl: assessed.needs_hitl,
            hitl_reason: assessed.hitl_reason,
            decision: assessed.decision,
        }
    }
}

#[cfg(test)]
#[path = "operations_json.rs"]
mod tests;
