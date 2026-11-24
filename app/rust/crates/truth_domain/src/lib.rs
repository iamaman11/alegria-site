//! Typed, deterministic Truth candidate and verification decisions.
//! External effects and durable workflow orchestration belong to adapters/runtime.

pub mod candidate_intake;
pub mod completeness;
pub mod contradictions;
pub mod evidence;
pub mod fact_verifier;
pub mod hitl_decision;
pub mod persisted_candidates;
pub mod procedural_candidates;
pub mod review_decisions;
pub mod semantic_truth_write;
pub mod truth_candidates;
pub mod truth_governance;
