//! Additive structural owner-schema pin, separate from V2 plan/History hashes.
//!
//! The provider-free DAG cannot introspect Rust DTO fields. A trusted owner
//! publishes complete, versioned descriptor bytes from its own DTO/IDL build;
//! this protocol pins those bytes beside all three Input/Output/Failure roles.
//! An untrusted caller may never mint its own descriptor and treat the hash as
//! owner admission. V2 `OperationContract::fingerprint` is intentionally intact.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    verify_contract, ArtifactSchema, ArtifactTypeId, ContractHash, OperationContract,
    OperationOwnerId, OperationRef, PlanError, PortId,
};

pub const OWNER_SCHEMA_ATTESTATION_REVISION_V3: u16 = 3;
pub const MAX_OWNER_DTO_DESCRIPTOR_BYTES_V3: usize = 64 * 1024;
pub const MAX_OWNER_OPERATION_DESCRIPTORS_BYTES_V3: usize = 256 * 1024;

/// This is the shape description format, not an instruction to switch the
/// operation's existing protobuf/JSON transport or Temporal Workflow Type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerDtoDescriptorFormatV3 {
    JsonSchema2020_12,
    ProtobufFileDescriptorSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerDtoDescriptorV3 {
    /// Owner-scoped logical ID. Input/output IDs must match their V2 port
    /// artifacts; the V3 failure ID is introduced separately and explicitly.
    pub schema_id: ArtifactTypeId,
    pub revision: u32,
    pub format: OwnerDtoDescriptorFormatV3,
    /// Exact descriptor bytes, generated and semantically validated by the
    /// owner codec. The provider-free kernel does not parse owner payloads.
    pub descriptor_bytes: Vec<u8>,
}

impl OwnerDtoDescriptorV3 {
    fn validate(&self) -> Result<usize, OwnerSchemaErrorV3> {
        if self.schema_id.as_str().trim().is_empty() || self.revision == 0 {
            return Err(OwnerSchemaErrorV3::InvalidDescriptorIdentity);
        }
        let len = self.descriptor_bytes.len();
        if len == 0 || len > MAX_OWNER_DTO_DESCRIPTOR_BYTES_V3 {
            return Err(OwnerSchemaErrorV3::DescriptorSize);
        }
        match self.format {
            OwnerDtoDescriptorFormatV3::JsonSchema2020_12 => {
                crate::owner_schema_json::validate_json_schema_2020_12(&self.descriptor_bytes)
                    .map_err(|_| OwnerSchemaErrorV3::InvalidJsonSchemaDescriptor)?;
            }
            // A provider-neutral contract cannot link the Protobuf SDK: this
            // is only a bounded opaque carrier. The independently trusted
            // owner installation MUST decode and validate FileDescriptorSet.
            OwnerDtoDescriptorFormatV3::ProtobufFileDescriptorSet => {}
        }
        Ok(len)
    }

    fn matches_port(&self, schema: &ArtifactSchema) -> bool {
        self.schema_id == schema.id && self.revision == schema.revision
    }
}

/// V3 schema attestation is additive and never serialized as a V2
/// `OperationContract` or injected into a historical `ExecutionPlan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerOperationSchemaV3 {
    pub schema_revision: u16,
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub v2_contract_fingerprint: ContractHash,
    pub inputs: BTreeMap<PortId, OwnerDtoDescriptorV3>,
    pub outputs: BTreeMap<PortId, OwnerDtoDescriptorV3>,
    /// Failure is required even for pure operations. A pure Infallible owner
    /// can attest the JSON Schema boolean `false`, with its own type identity.
    pub failure: OwnerDtoDescriptorV3,
}

/// The immutable V3 payload and its independent content hash. Loading a
/// persisted pin requires `validate` against the trusted installed contract;
/// `seal` is only for a freshly built, owner-authored descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedOwnerOperationSchemaV3 {
    pub schema: OwnerOperationSchemaV3,
    pub fingerprint: ContractHash,
}

#[derive(Debug, Error)]
pub enum OwnerSchemaErrorV3 {
    #[error("unsupported owner schema revision")]
    UnsupportedRevision,
    #[error("owner schema does not match the installed operation owner/reference")]
    WrongOperation,
    #[error("owner schema differs from its installed V2 operation contract")]
    ContractFingerprintMismatch,
    #[error("owner schema input/output ports or artifact identities do not match V2")]
    PortDescriptorMismatch,
    #[error("owner DTO descriptor identity is empty or not versioned")]
    InvalidDescriptorIdentity,
    #[error("owner DTO descriptor bytes are empty or over the strict budget")]
    DescriptorSize,
    #[error("owner JSON Schema descriptor must be unambiguous draft 2020-12 syntax with a declared root shape")]
    InvalidJsonSchemaDescriptor,
    #[error("owner DTO descriptors exceed total operation budget")]
    TotalDescriptorSize,
    #[error("pinned owner V3 structural schema fingerprint changed")]
    FingerprintMismatch,
    #[error("caller-provided schema does not match trusted owner-installed attestation")]
    UntrustedOwnerDescriptor,
    #[error(transparent)]
    InvalidV2Contract(#[from] PlanError),
}

impl OwnerOperationSchemaV3 {
    fn validate_against(&self, installed: &OperationContract) -> Result<(), OwnerSchemaErrorV3> {
        verify_contract(installed)?;
        if self.schema_revision != OWNER_SCHEMA_ATTESTATION_REVISION_V3 {
            return Err(OwnerSchemaErrorV3::UnsupportedRevision);
        }
        if self.operation != installed.reference || self.owner != installed.owner {
            return Err(OwnerSchemaErrorV3::WrongOperation);
        }
        if self.v2_contract_fingerprint != installed.fingerprint()? {
            return Err(OwnerSchemaErrorV3::ContractFingerprintMismatch);
        }
        if self.inputs.len() != installed.inputs.len()
            || self.outputs.len() != installed.outputs.len()
            || !installed.inputs.iter().all(|(port, schema)| {
                self.inputs
                    .get(port)
                    .is_some_and(|dto| dto.matches_port(schema))
            })
            || !installed.outputs.iter().all(|(port, schema)| {
                self.outputs
                    .get(port)
                    .is_some_and(|dto| dto.matches_port(schema))
            })
        {
            return Err(OwnerSchemaErrorV3::PortDescriptorMismatch);
        }
        let total = self
            .inputs
            .values()
            .chain(self.outputs.values())
            .chain(std::iter::once(&self.failure))
            .try_fold(0usize, |total, descriptor| {
                let size = descriptor.validate()?;
                total
                    .checked_add(size)
                    .ok_or(OwnerSchemaErrorV3::TotalDescriptorSize)
            })?;
        if total > MAX_OWNER_OPERATION_DESCRIPTORS_BYTES_V3 {
            return Err(OwnerSchemaErrorV3::TotalDescriptorSize);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<ContractHash, OwnerSchemaErrorV3> {
        let bytes = serde_json::to_vec(self).map_err(|error| {
            OwnerSchemaErrorV3::InvalidV2Contract(PlanError::Serialization(error.to_string()))
        })?;
        Ok(ContractHash(blake3::hash(&bytes).to_hex().to_string()))
    }

    pub fn seal(
        self,
        installed: &OperationContract,
    ) -> Result<PinnedOwnerOperationSchemaV3, OwnerSchemaErrorV3> {
        self.validate_against(installed)?;
        Ok(PinnedOwnerOperationSchemaV3 {
            fingerprint: self.fingerprint()?,
            schema: self,
        })
    }
}

impl PinnedOwnerOperationSchemaV3 {
    pub fn validate(&self, installed: &OperationContract) -> Result<(), OwnerSchemaErrorV3> {
        self.schema.validate_against(installed)?;
        if self.fingerprint != self.schema.fingerprint()? {
            return Err(OwnerSchemaErrorV3::FingerprintMismatch);
        }
        Ok(())
    }

    /// Comparing with only a V2 `OperationContract` is insufficient: any
    /// caller can write new bytes and call `seal`. The `trusted` pin MUST come
    /// from an independently installed, owner-issued catalog/attestation, not
    /// from a plan, request, or other caller-controlled payload. This method
    /// checks structural identity; it does not grant runtime effect admission.
    pub fn verify_against_trusted_owner(
        &self,
        trusted: &Self,
        installed: &OperationContract,
    ) -> Result<(), OwnerSchemaErrorV3> {
        self.validate(installed)?;
        trusted.validate(installed)?;
        if self.fingerprint != trusted.fingerprint {
            return Err(OwnerSchemaErrorV3::UntrustedOwnerDescriptor);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::{
        ArtifactSchema, ArtifactTypeId, IdempotencyPolicy, OperationContract, OperationId,
        OperationOwnerId, OperationRef, PortId,
    };

    use super::*;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId::from("semantic.owner_v3_test"),
                revision: 1,
            },
            owner: OperationOwnerId::from("semantic_extraction"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                ArtifactSchema {
                    id: ArtifactTypeId::from("semantic.input"),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                ArtifactSchema {
                    id: ArtifactTypeId::from("semantic.output"),
                    revision: 1,
                },
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }

    fn dto(id: &str, shape: &str) -> OwnerDtoDescriptorV3 {
        OwnerDtoDescriptorV3 {
            schema_id: ArtifactTypeId::from(id),
            revision: 1,
            format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
            descriptor_bytes: shape.as_bytes().to_vec(),
        }
    }

    fn spec(contract: &OperationContract) -> OwnerOperationSchemaV3 {
        OwnerOperationSchemaV3 {
            schema_revision: OWNER_SCHEMA_ATTESTATION_REVISION_V3,
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            v2_contract_fingerprint: contract.fingerprint().unwrap(),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                dto(
                    "semantic.input",
                    r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"source":{"type":"string"}}}"#,
                ),
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                dto(
                    "semantic.output",
                    r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"array","items":{"type":"integer"}}"#,
                ),
            )]),
            failure: dto("semantic.infallible", "false"),
        }
    }

    #[test]
    fn v3_pins_full_nested_descriptor_bytes_without_changing_v2_fingerprint() {
        let installed = contract();
        let v2_before = installed.fingerprint().unwrap();
        let pinned = spec(&installed).seal(&installed).unwrap();
        pinned.validate(&installed).unwrap();
        assert_eq!(installed.fingerprint().unwrap(), v2_before);

        let mut tampered = pinned.clone();
        tampered
            .schema
            .inputs
            .get_mut(&PortId::from("input"))
            .unwrap()
            .descriptor_bytes =
            br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"source":{"type":"integer"}}}"#.to_vec();
        assert!(matches!(
            tampered.validate(&installed),
            Err(OwnerSchemaErrorV3::FingerprintMismatch)
        ));

        let re_sealed = tampered.schema.seal(&installed).unwrap();
        assert_ne!(re_sealed.fingerprint, pinned.fingerprint);
        // An attacker may self-seal a different structure, but that cannot
        // impersonate a separately installed owner attestation.
        re_sealed.validate(&installed).unwrap();
        assert!(matches!(
            re_sealed.verify_against_trusted_owner(&pinned, &installed),
            Err(OwnerSchemaErrorV3::UntrustedOwnerDescriptor)
        ));
        pinned
            .verify_against_trusted_owner(&pinned, &installed)
            .unwrap();
        assert_eq!(installed.fingerprint().unwrap(), v2_before);
    }

    #[test]
    fn v3_rejects_drifted_owner_ports_and_v2_contract_without_resealing() {
        let installed = contract();
        let pinned = spec(&installed).seal(&installed).unwrap();
        let mut wrong_port = pinned.clone();
        wrong_port.schema.inputs.remove(&PortId::from("input"));
        assert!(matches!(
            wrong_port.validate(&installed),
            Err(OwnerSchemaErrorV3::PortDescriptorMismatch)
        ));

        let mut wrong_contract = installed.clone();
        wrong_contract.idempotency = IdempotencyPolicy::Idempotent;
        assert!(matches!(
            pinned.validate(&wrong_contract),
            Err(OwnerSchemaErrorV3::ContractFingerprintMismatch)
        ));
    }

    #[test]
    fn v3_rejects_missing_failure_invalid_identity_and_budgets() {
        let installed = contract();
        let mut missing_failure = spec(&installed);
        missing_failure.failure.descriptor_bytes.clear();
        assert!(matches!(
            missing_failure.seal(&installed),
            Err(OwnerSchemaErrorV3::DescriptorSize)
        ));

        let mut invalid_identity = spec(&installed);
        invalid_identity
            .inputs
            .get_mut(&PortId::from("input"))
            .unwrap()
            .revision = 0;
        assert!(matches!(
            invalid_identity.seal(&installed),
            Err(OwnerSchemaErrorV3::PortDescriptorMismatch)
        ));

        let mut unnamed_failure = spec(&installed);
        unnamed_failure.failure.schema_id = ArtifactTypeId::from("");
        assert!(matches!(
            unnamed_failure.seal(&installed),
            Err(OwnerSchemaErrorV3::InvalidDescriptorIdentity)
        ));

        let mut oversized = spec(&installed);
        oversized.failure.descriptor_bytes = vec![b'1'; MAX_OWNER_DTO_DESCRIPTOR_BYTES_V3 + 1];
        assert!(matches!(
            oversized.seal(&installed),
            Err(OwnerSchemaErrorV3::DescriptorSize)
        ));
    }

    #[test]
    fn v3_refuses_vacuous_or_malformed_descriptors_even_if_self_sealed() {
        let installed = contract();
        for bad in ["true", "{}", "[]", "{", r#"{"type":"object"}"#] {
            let mut candidate = spec(&installed);
            candidate.failure.descriptor_bytes = bad.as_bytes().to_vec();
            assert!(matches!(
                candidate.seal(&installed),
                Err(OwnerSchemaErrorV3::InvalidJsonSchemaDescriptor)
            ));
        }
        // Opaque protobuf bytes cannot be validated in this dependency-free
        // protocol. An independently trusted installer must reject them.
        let mut opaque = spec(&installed);
        opaque.failure.format = OwnerDtoDescriptorFormatV3::ProtobufFileDescriptorSet;
        opaque.failure.descriptor_bytes = b"not a descriptor set".to_vec();
        opaque.seal(&installed).unwrap();
        let mut duplicate = spec(&installed);
        duplicate.failure.descriptor_bytes = br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"person":{"type":"object","properties":{"x":{"type":"string"},"x":{"type":"number"}}}}}"#.to_vec();
        assert!(matches!(
            duplicate.seal(&installed),
            Err(OwnerSchemaErrorV3::InvalidJsonSchemaDescriptor)
        ));
    }
}
