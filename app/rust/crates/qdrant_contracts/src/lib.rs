//! Owner-specific, provider-free contracts for Qdrant operations.
//!
//! The generic operation protocol, PostgreSQL owner and Temporal dispatcher
//! do not import this crate. Qdrant owner implementations consume the
//! versioned wire types without leaking qdrant-client or sqlx downstream.

mod qdrant_effect_child_v1;
mod qdrant_effect_child_v2;
mod qdrant_owner_generic_v1;
mod qdrant_produced_vector_v1;

pub use qdrant_effect_child_v1::{
    QdrantChildOutcomeV1, QdrantKeywordChildInvocationV1, QdrantKeywordChildResultV1,
    QdrantKeywordProjectionCommandV1, QdrantProjectionRunV1, QDRANT_DAG_PARENT_QUEUE_V1,
    QDRANT_DAG_PARENT_TYPE_V1, QDRANT_OWNER_ACTIVITY_QUEUE_V1, QDRANT_OWNER_CHILD_TYPE_V1,
    QDRANT_OWNER_COMMAND_QUEUE_V1,
};
pub use qdrant_effect_child_v2::{
    QdrantChildProtocolErrorV2, QdrantKeywordChildInvocationV2, QdrantKeywordChildResultV2,
    QdrantKeywordProjectionContractV2, QDRANT_CHILD_SCHEMA_REVISION_V2,
    QDRANT_EFFECT_INPUT_PORT_V2, QDRANT_EFFECT_OPERATION_ID_V2, QDRANT_EFFECT_OUTPUT_PORT_V2,
    QDRANT_EFFECT_OWNER_ID_V2, QDRANT_OWNER_ACTIVITY_QUEUE_V2, QDRANT_OWNER_CHILD_TYPE_V2,
    QDRANT_OWNER_COMMAND_QUEUE_V2,
};
pub use qdrant_owner_generic_v1::{
    confirmed_qdrant_owner_result, verify_external_qdrant_owner_invocation,
    verify_qdrant_owner_invocation, GenericQdrantContractError, QDRANT_GENERIC_ACTIVITY_QUEUE_V1,
    QDRANT_GENERIC_ADMISSION_SIGNAL_V1, QDRANT_GENERIC_OWNER_QUEUE_V1,
};
pub use qdrant_produced_vector_v1::{
    verify_produced_qdrant_owner_invocation, QdrantProducedVectorCommandV1,
    QdrantProducedVectorProjectionContractV1, QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1,
    QDRANT_PRODUCED_VECTOR_OPERATION_ID_V1,
};
