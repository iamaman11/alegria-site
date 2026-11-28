//! Shared source-section artifact contract.
use operation_protocol::ArtifactSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EntitySpanInput {
    pub section_id: String,
    pub raw_text: String,
}

pub fn entity_span_input_schema() -> ArtifactSchema {
    ArtifactSchema {
        id: "semantic.entity_span_input".into(),
        revision: 1,
    }
}
