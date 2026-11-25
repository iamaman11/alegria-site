//! Provider-free, frozen V7 native Temporal Qdrant child transport.
//! The opt-in DAG is initially a *serial one-owner* operation route; no PG
//! fanout or generic effect registry is implied. Every command is preflighted
//! before any child can be started, and independently admitted by an operator.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::TemporalExecutionIdentityV4;

pub const QDRANT_KEYWORD_PROJECTION_COMMAND_REVISION: u16 = 1;
pub const QDRANT_OWNER_CHILD_TYPE_V1: &str = "QdrantKeywordProjectionV1Workflow";
pub const QDRANT_OWNER_COMMAND_QUEUE_V1: &str = "alegria-qdrant-command-v1";
pub const QDRANT_OWNER_ACTIVITY_QUEUE_V1: &str = "alegria-qdrant-effects-v1";
pub const QDRANT_DAG_PARENT_TYPE_V1: &str = "AlegriaQdrantProjectionDagV1Workflow";
pub const QDRANT_DAG_PARENT_QUEUE_V1: &str = "alegria-qdrant-dag-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantKeywordProjectionCommandV1 {
    pub schema_revision: u16,
    pub admission_id: Uuid,
    pub tenant_id: String,
    pub context_key: String,
    pub keyword_key: String,
    pub vector: Vec<f32>,
}

impl QdrantKeywordProjectionCommandV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_revision != QDRANT_KEYWORD_PROJECTION_COMMAND_REVISION {
            return Err("unsupported Qdrant owner command revision");
        }
        if self.admission_id.is_nil() {
            return Err("Qdrant admission ID cannot be nil");
        }
        for value in [&self.tenant_id, &self.context_key, &self.keyword_key] {
            if value.is_empty() || value.trim() != value {
                return Err("noncanonical Qdrant owner identity");
            }
        }
        if self.vector.len() != 1024 {
            return Err("Qdrant keyword embedding dimension mismatch");
        }
        if !self.vector.iter().all(|x| x.is_finite()) {
            return Err("Qdrant embedding contains NaN/Infinity");
        }
        if !self.vector.iter().any(|x| *x != 0.0) {
            return Err("Qdrant embedding cannot be the zero vector");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantProjectionRunV1 {
    pub schema_revision: u16,
    pub canonical_run_id: Uuid,
    pub tenant_id: String,
    pub context_key: String,
    /// Explicit, deterministic operation order. Future typed PG/other-owner
    /// DAG nodes require a distinct versioned route, never V1 reinterpretation.
    pub commands: Vec<QdrantKeywordProjectionCommandV1>,
}

impl QdrantProjectionRunV1 {
    pub fn preflight(&self) -> Result<(), &'static str> {
        if self.schema_revision != 1
            || self.canonical_run_id.is_nil()
            || self.tenant_id.is_empty()
            || self.context_key.is_empty()
            || self.tenant_id.trim() != self.tenant_id
            || self.context_key.trim() != self.context_key
            || self.commands.is_empty()
            || self.commands.len() > 32
        {
            return Err("invalid Qdrant parent run envelope or child count");
        }
        let mut admissions = BTreeSet::new();
        let mut keywords = BTreeSet::new();
        for command in &self.commands {
            command.validate()?;
            if command.tenant_id != self.tenant_id || command.context_key != self.context_key {
                return Err("mixed tenant/context Qdrant child in parent run");
            }
            if !admissions.insert(command.admission_id)
                || !keywords.insert(command.keyword_key.as_str())
            {
                return Err("duplicate Qdrant admission or conflicting business keyword");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantKeywordChildInvocationV1 {
    pub schema_revision: u16,
    pub parent: TemporalExecutionIdentityV4,
    pub canonical_run_id: Uuid,
    pub command: QdrantKeywordProjectionCommandV1,
}

impl QdrantKeywordChildInvocationV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_revision != 1
            || self.canonical_run_id.is_nil()
            || self.parent.workflow_id.is_empty()
            || self.parent.run_id.is_empty()
            || self.parent.workflow_id.trim() != self.parent.workflow_id
            || self.parent.run_id.trim() != self.parent.run_id
        {
            return Err("invalid canonical Qdrant child execution identity");
        }
        self.command.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QdrantKeywordChildResultV1 {
    pub schema_revision: u16,
    pub admission_id: Uuid,
    pub outcome: QdrantChildOutcomeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QdrantChildOutcomeV1 {
    Applied,
    Reconciled,
    AlreadyConfirmed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_qdrant_parent_preflights_complete_command_set() {
        let command = QdrantKeywordProjectionCommandV1 {
            schema_revision: 1,
            admission_id: Uuid::new_v4(),
            tenant_id: "tenant-a".into(),
            context_key: "context-a".into(),
            keyword_key: "keyword-a".into(),
            vector: vec![0.1; 1024],
        };
        let mut run = QdrantProjectionRunV1 {
            schema_revision: 1,
            canonical_run_id: Uuid::new_v4(),
            tenant_id: command.tenant_id.clone(),
            context_key: command.context_key.clone(),
            commands: vec![command.clone()],
        };
        run.preflight().unwrap();
        run.commands.push(command.clone());
        assert!(run.preflight().is_err());
        run.commands[1].admission_id = Uuid::new_v4();
        assert!(run.preflight().is_err());
        run.commands[1].keyword_key = "keyword-b".into();
        run.preflight().unwrap();
        run.commands[1].tenant_id = "tenant-b".into();
        assert!(run.preflight().is_err());
    }

    #[test]
    fn child_identity_and_full_parent_are_checked_before_external_effect() {
        let mut child = QdrantKeywordChildInvocationV1 {
            schema_revision: 1,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: "canonical-parent".into(),
                run_id: Uuid::new_v4().to_string(),
            },
            canonical_run_id: Uuid::new_v4(),
            command: QdrantKeywordProjectionCommandV1 {
                schema_revision: 1,
                admission_id: Uuid::new_v4(),
                tenant_id: "tenant-a".into(),
                context_key: "context-a".into(),
                keyword_key: "keyword-a".into(),
                vector: vec![0.1; 1024],
            },
        };
        child.validate().unwrap();
        child.schema_revision = 2;
        assert!(child.validate().is_err());
        child.schema_revision = 1;
        child.parent.run_id.push(' ');
        assert!(child.validate().is_err());
        child.parent.run_id.pop();
        child.canonical_run_id = Uuid::nil();
        assert!(child.validate().is_err());
    }
}
