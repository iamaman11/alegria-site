//! Owner-neutral, versioned semantic projection data exchanged through Temporal.
//!
//! PostgreSQL may produce this business artifact from a trusted durable source;
//! Qdrant may consume it through its own independently admitted operation.
//! Neither owner imports the other's SDK, command, or execution implementation.

use operation_protocol::{ArtifactSchema, ArtifactTypeId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const VECTOR_PROJECTION_SCHEMA_ID: &str = "projection.vector_projection";
pub const VECTOR_PROJECTION_SCHEMA_REVISION: u32 = 1;
pub const VECTOR_PROJECTION_SCHEMA_ID_V1: &str = VECTOR_PROJECTION_SCHEMA_ID;
pub const VECTOR_PROJECTION_SCHEMA_REVISION_V1: u32 = VECTOR_PROJECTION_SCHEMA_REVISION;
pub const VECTOR_PROJECTION_PORT_V1: &str = "vector_projection";
pub const VECTOR_LOOKUP_SCHEMA_ID: &str = "projection.vector_lookup";
pub const VECTOR_LOOKUP_SCHEMA_REVISION: u32 = 1;
pub const VECTOR_DIMENSIONS_V1: usize = 1024;

pub fn vector_projection_schema() -> ArtifactSchema {
    ArtifactSchema {
        id: ArtifactTypeId(VECTOR_PROJECTION_SCHEMA_ID.to_string()),
        revision: VECTOR_PROJECTION_SCHEMA_REVISION,
    }
}

pub fn vector_lookup_schema() -> ArtifactSchema {
    ArtifactSchema {
        id: ArtifactTypeId(VECTOR_LOOKUP_SCHEMA_ID.to_string()),
        revision: VECTOR_LOOKUP_SCHEMA_REVISION,
    }
}

/// The canonical, *precomputed* semantic vector stored by the source owner.
/// A receipt, object identifier or arbitrary text is never a legal vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorProjectionV1 {
    pub tenant_id: String,
    pub context_key: String,
    pub keyword_key: String,
    pub vector: Vec<f32>,
}

/// Exact identity of a PG-owned projection row, independent of the provider
/// that may later consume a compatible VectorProjectionV1 artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorProjectionLookupV1 {
    pub context_key: String,
    pub keyword_key: String,
}

/// Verifiable data-source receipt, shared as data rather than as PG owner code.
/// Operator-only neutral SQL attestation corroborates it independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorProjectionSourceReceiptV1 {
    pub receipt_id: Uuid,
    pub admission_id: Uuid,
    pub execution_key: String,
    pub source_hash: String,
    pub child_workflow_run_id: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VectorProjectionValidationError {
    #[error("projection tenant/context/keyword must be nonempty canonical values")]
    MissingIdentity,
    #[error("projection vector must have exactly 1024 finite values")]
    InvalidVector,
}

impl VectorProjectionV1 {
    pub fn validate(&self) -> Result<(), VectorProjectionValidationError> {
        if [self.tenant_id.as_str(), self.context_key.as_str(), self.keyword_key.as_str()]
            .iter()
            .any(|s| s.is_empty() || s.trim() != *s || s.contains('\0'))
        {
            return Err(VectorProjectionValidationError::MissingIdentity);
        }
        if self.vector.len() != VECTOR_DIMENSIONS_V1 || self.vector.iter().any(|n| !n.is_finite()) {
            return Err(VectorProjectionValidationError::InvalidVector);
        }
        Ok(())
    }
}

impl VectorProjectionLookupV1 {
    pub fn validate(&self) -> Result<(), VectorProjectionValidationError> {
        if [self.context_key.as_str(), self.keyword_key.as_str()]
            .iter()
            .any(|s| s.is_empty() || s.trim() != *s || s.contains('\0'))
        {
            return Err(VectorProjectionValidationError::MissingIdentity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_rejects_nonfinite_or_wrong_dimension() {
        let mut projection = VectorProjectionV1 {
            tenant_id: "tenant-a".into(),
            context_key: "ES:tourist:BY".into(),
            keyword_key: "visa-fee".into(),
            vector: vec![0.5; VECTOR_DIMENSIONS_V1],
        };
        assert!(projection.validate().is_ok());
        projection.vector[0] = f32::NAN;
        assert_eq!(projection.validate(), Err(VectorProjectionValidationError::InvalidVector));
        projection.vector[0] = 0.5;
        projection.tenant_id = " ".to_owned();
        assert_eq!(projection.validate(), Err(VectorProjectionValidationError::MissingIdentity));
    }
}
