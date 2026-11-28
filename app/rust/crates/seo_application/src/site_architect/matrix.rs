//! Read-only product hierarchy joined to the catalog by exact operation revision.
//! Planning rows never enter ContractCatalog or affect execution admission.
use super::{OperationRef, ToolDescription};
use operation_protocol::{ArtifactSchema, PortId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Decision {
    Keep,
    Split,
    Internal,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Acquisition,
    Extraction,
    Truth,
    Knowledge,
    Planning,
    Editorial,
    CmsReview,
    Publishing,
}

impl Area {
    fn from_candidate(id: &str) -> Option<Self> {
        match id.split_once('-')?.0 {
            "ACQ" => Some(Self::Acquisition),
            "EX" => Some(Self::Extraction),
            "TR" => Some(Self::Truth),
            "KN" => Some(Self::Knowledge),
            "PL" => Some(Self::Planning),
            "ED" => Some(Self::Editorial),
            "CMS" => Some(Self::CmsReview),
            "PUB" => Some(Self::Publishing),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Decisions {
    candidates: Vec<SourceRow>,
    declared_rust_contracts: BTreeMap<String, String>,
    proposed_ports: BTreeMap<String, ProposedPorts>,
}

#[derive(Deserialize)]
struct SourceRow {
    id: String,
    decision: Decision,
    operations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: String,
    pub decision: Decision,
    pub operations: Vec<OperationRef>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Planned,
    SourceDeclared,
    InstalledContract,
}

#[derive(Debug, Serialize)]
pub struct MatrixOperation {
    pub operation: OperationRef,
    pub status: ContractStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_ports: Option<ProposedPorts>,
}

/// Design vocabulary for an action awaiting an owner's concrete DTOs.
/// Only an installed OperationContract can participate in execution.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedPorts {
    pub inputs: BTreeMap<PortId, ArtifactSchema>,
    pub outputs: BTreeMap<PortId, ArtifactSchema>,
}

#[derive(Debug, Serialize)]
pub struct OperationMatrix {
    pub schema_revision: u16,
    pub areas: BTreeMap<Area, Vec<Candidate>>,
    pub actions: Vec<MatrixOperation>,
    /// Installed extensions stay discoverable before undergoing product classification.
    pub installed_outside_matrix: Vec<OperationRef>,
}

#[derive(Debug, thiserror::Error)]
pub enum MatrixError {
    #[error("invalid bundled action matrix: {0}")]
    InvalidManifest(#[from] serde_json::Error),
    #[error("unknown product area for candidate {0}")]
    UnknownArea(String),
    #[error("invalid versioned action in the product matrix: {0}")]
    InvalidReference(String),
}

fn operation_ref(key: &str) -> Result<OperationRef, MatrixError> {
    let (id, revision) = key
        .rsplit_once('@')
        .and_then(|(id, revision)| Some((id, revision.parse::<u32>().ok()?)))
        .filter(|(id, revision)| !id.is_empty() && *revision > 0)
        .ok_or_else(|| MatrixError::InvalidReference(key.to_owned()))?;
    Ok(OperationRef {
        id: id.into(),
        revision,
    })
}

/// Join two observations: the accepted product plan and installed owner descriptions.
/// Port schemas, parameters, effects and readiness live once in ToolDescription.
pub fn operation_matrix(tools: &[ToolDescription]) -> Result<OperationMatrix, MatrixError> {
    let manifest = include_bytes!("site_architect_action_matrix_v1.json");
    let plan = Decisions::deserialize(crate::typed_json::parse_unique_json_bytes(manifest)?)?;
    let installed: BTreeSet<_> = tools.iter().map(|tool| tool.operation.clone()).collect();
    let mut areas = BTreeMap::<_, Vec<_>>::new();
    let mut actions = BTreeMap::new();
    for row in plan.candidates {
        let area = Area::from_candidate(&row.id)
            .ok_or_else(|| MatrixError::UnknownArea(row.id.clone()))?;
        let mut operations = Vec::with_capacity(row.operations.len());
        for key in &row.operations {
            let operation = operation_ref(key)?;
            let declaration = plan.declared_rust_contracts.get(key);
            actions.entry(operation.clone()).or_insert_with(|| {
                let (status, proposed_ports) = if installed.contains(&operation) {
                    (ContractStatus::InstalledContract, None)
                } else if declaration.is_some() {
                    (ContractStatus::SourceDeclared, None)
                } else {
                    (
                        ContractStatus::Planned,
                        plan.proposed_ports.get(key).cloned(),
                    )
                };
                MatrixOperation {
                    operation: operation.clone(),
                    status,
                    declaration: declaration.cloned(),
                    proposed_ports,
                }
            });
            operations.push(operation);
        }
        areas.entry(area).or_default().push(Candidate {
            id: row.id,
            decision: row.decision,
            operations,
        });
    }
    Ok(OperationMatrix {
        schema_revision: 1,
        areas,
        installed_outside_matrix: installed
            .into_iter()
            .filter(|operation| !actions.contains_key(operation))
            .collect(),
        actions: actions.into_values().collect(),
    })
}
