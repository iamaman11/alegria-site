//! Owner-issued schemas of the *actual* semantic operation Rust DTOs.
//!
//! Descriptors are generated from `JsonSchema` derives on domain Input and
//! Output types. A compiler error, rather than a silently stale hand-written
//! shape, results if a new operation DTO lacks a generated descriptor.
//! This owner-local function is for trusted installation at deployment; a
//! caller-supplied copy must never become the authoritative schema catalog.
//! Existing V1 Protobuf, V2 plan fingerprints and V3 History stay unchanged.

use std::collections::BTreeMap;
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, Operation, OperationRef, OwnerDtoDescriptorFormatV3,
    OwnerDtoDescriptorV3, OwnerOperationSchemaV3, OwnerSchemaErrorV3,
    PinnedOwnerOperationSchemaV3, PlanError, PortId, OWNER_SCHEMA_ATTESTATION_REVISION_V3,
};
use schemars::{generate::SchemaSettings, JsonSchema};
use seo_domain::extraction::operations::{
    CanonicalizeMentions, EditorialTopics, EntitySpans, LayerRoute, OperationalEntities,
    PureExtractionOperation,
};

/// Generate the complete JSON Schema 2020-12 descriptor, including nested
/// structs, enum variants and optional/required fields, from the actual
/// domain DTO definition. The serialization is deterministic for identical
/// source and generator versions (the version is pinned in Cargo.lock).
#[derive(Clone, Copy)]
enum DtoWireRole {
    InputDeserialize,
    OutputSerialize,
}

fn descriptor_from_dto<T: JsonSchema>(
    artifact: &ArtifactSchema,
    role: DtoWireRole,
) -> Result<OwnerDtoDescriptorV3, OwnerSchemaErrorV3> {
    let settings = SchemaSettings::draft2020_12();
    let settings = match role {
        DtoWireRole::InputDeserialize => settings.for_deserialize(),
        DtoWireRole::OutputSerialize => settings.for_serialize(),
    };
    let generated = settings.into_generator().into_root_schema_for::<T>();
    let descriptor_bytes = serde_json::to_vec(&generated).map_err(|error| {
        OwnerSchemaErrorV3::InvalidV2Contract(PlanError::Serialization(error.to_string()))
    })?;
    Ok(OwnerDtoDescriptorV3 {
        schema_id: artifact.id.clone(),
        revision: artifact.revision,
        format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
        descriptor_bytes,
    })
}

/// There is no serializable Failure value for these operations: their actual
/// associated type is `Infallible`. The JSON Schema of the empty set of
/// possible values is the boolean schema `false`, not an arbitrary error DTO.
/// The trait bound prevents reusing this declaration for a fallible owner.
fn infallible_failure_descriptor<O>() -> OwnerDtoDescriptorV3
where
    O: PureExtractionOperation<Failure = Infallible>,
{
    OwnerDtoDescriptorV3 {
        schema_id: ArtifactTypeId::from("semantic.infallible_failure"),
        revision: 1,
        format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
        descriptor_bytes: b"false".to_vec(),
    }
}

fn owner_schema<O>() -> Result<PinnedOwnerOperationSchemaV3, OwnerSchemaErrorV3>
where
    O: PureExtractionOperation<Failure = Infallible>,
    O::Input: JsonSchema,
    O::Output: JsonSchema,
{
    let installed = O::contract();
    if installed.inputs.len() != 1 || installed.outputs.len() != 1 {
        return Err(OwnerSchemaErrorV3::PortDescriptorMismatch);
    }
    let (input_port, input_artifact) = installed
        .inputs
        .iter()
        .next()
        .ok_or(OwnerSchemaErrorV3::PortDescriptorMismatch)?;
    let (output_port, output_artifact) = installed
        .outputs
        .iter()
        .next()
        .ok_or(OwnerSchemaErrorV3::PortDescriptorMismatch)?;
    OwnerOperationSchemaV3 {
        schema_revision: OWNER_SCHEMA_ATTESTATION_REVISION_V3,
        operation: installed.reference.clone(),
        owner: installed.owner.clone(),
        v2_contract_fingerprint: installed.fingerprint()?,
        inputs: BTreeMap::<PortId, OwnerDtoDescriptorV3>::from([(
            input_port.clone(),
            descriptor_from_dto::<O::Input>(input_artifact, DtoWireRole::InputDeserialize)?,
        )]),
        outputs: BTreeMap::<PortId, OwnerDtoDescriptorV3>::from([(
            output_port.clone(),
            descriptor_from_dto::<O::Output>(output_artifact, DtoWireRole::OutputSerialize)?,
        )]),
        failure: infallible_failure_descriptor::<O>(),
    }
    .seal(&installed)
}

/// Complete owner-originated structural attestations for all five installed
/// `semantic.*` operations. A trusted deployment can register these beside
/// owner routes, without permitting authoring clients to mint descriptors.
pub fn semantic_extraction_owner_schemas(
) -> Result<BTreeMap<OperationRef, PinnedOwnerOperationSchemaV3>, OwnerSchemaErrorV3> {
    let schemas = [
        owner_schema::<LayerRoute>()?,
        owner_schema::<EntitySpans>()?,
        owner_schema::<CanonicalizeMentions>()?,
        owner_schema::<EditorialTopics>()?,
        owner_schema::<OperationalEntities>()?,
    ];
    Ok(schemas
        .into_iter()
        .map(|schema| (schema.schema.operation.clone(), schema))
        .collect())
}

#[cfg(test)]
#[path = "owner_dto_json.rs"]
mod tests;
