//! Strict owner-local DTO codec and structural attestations for three pure SEO operations.
//!
//! The module does not authorize a provider effect, open a database, or install
//! a Temporal route. The separately deployed owner must validate scope and
//! provenance before it calls this deterministic dispatch.

use std::collections::BTreeMap;
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, Operation, OperationRef, OwnerDtoDescriptorFormatV3,
    OwnerDtoDescriptorV3, OwnerOperationSchemaV3, OwnerSchemaErrorV3,
    PinnedOwnerOperationSchemaV3, PlanError, PortId, DAG_MAX_INPUT_BYTES,
    DAG_MAX_RESULT_BYTES, OWNER_SCHEMA_ATTESTATION_REVISION_V3,
};
use schemars::{generate::SchemaSettings, JsonSchema};
use serde::{de::DeserializeOwned, Serialize};

use crate::owner_pure_operations::{
    BuildSeoOpportunities, ExtractProceduralRules, NormalizeSerpQueries, OwnerPureOperation,
    OpportunityBuildFailureV1, SerpNormalizeFailureV1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeoStepsPureWireError {
    UnknownOperation,
    InputTooLarge,
    InvalidInput,
    InvalidOutput,
    OutputTooLarge,
    /// A real owner-defined, versioned Failure DTO; never a successful
    /// placeholder or a transient provider retry.
    DomainFailure { operation: OperationRef, failure_json: Vec<u8> },
}

pub fn validate_seo_steps_pure_input(
    operation: &OperationRef,
    payload: &[u8],
) -> Result<(), SeoStepsPureWireError> {
    // Evaluating the deterministic pure business rule here additionally
    // rejects semantic domain failures and oversized outputs *before* the
    // parent starts a Child. It never reads provider, SQL, or Temporal state.
    execute_seo_steps_pure(operation, payload).map(|_| ())
}

pub fn execute_seo_steps_pure(
    operation: &OperationRef,
    payload: &[u8],
) -> Result<Vec<u8>, SeoStepsPureWireError> {
    if payload.len() > DAG_MAX_INPUT_BYTES {
        return Err(SeoStepsPureWireError::InputTooLarge);
    }
    if operation == &ExtractProceduralRules::contract().reference {
        return evaluate_infallible::<ExtractProceduralRules>(payload);
    }
    if operation == &NormalizeSerpQueries::contract().reference {
        return evaluate::<NormalizeSerpQueries>(payload);
    }
    if operation == &BuildSeoOpportunities::contract().reference {
        return evaluate::<BuildSeoOpportunities>(payload);
    }
    Err(SeoStepsPureWireError::UnknownOperation)
}

fn evaluate_infallible<O>(payload: &[u8]) -> Result<Vec<u8>, SeoStepsPureWireError>
where
    O: OwnerPureOperation<Failure = Infallible>,
    O::Input: DeserializeOwned,
    O::Output: Serialize,
{
    let input: O::Input =
        serde_json::from_slice(payload).map_err(|_| SeoStepsPureWireError::InvalidInput)?;
    let output = match O::evaluate(&input) {
        Ok(output) => output,
        Err(never) => match never {},
    };
    serialize_output(&output)
}

fn evaluate<O: OwnerPureOperation>(payload: &[u8]) -> Result<Vec<u8>, SeoStepsPureWireError>
where
    O::Input: DeserializeOwned,
    O::Output: Serialize,
    O::Failure: Serialize,
{
    let input: O::Input =
        serde_json::from_slice(payload).map_err(|_| SeoStepsPureWireError::InvalidInput)?;
    let output = O::evaluate(&input).map_err(|failure| {
        serde_json::to_vec(&failure).map_or(SeoStepsPureWireError::InvalidOutput, |failure_json| {
            SeoStepsPureWireError::DomainFailure {
                operation: O::contract().reference,
                failure_json,
            }
        })
    })?;
    serialize_output(&output)
}

fn serialize_output<T: Serialize>(output: &T) -> Result<Vec<u8>, SeoStepsPureWireError> {
    let bytes = serde_json::to_vec(output).map_err(|_| SeoStepsPureWireError::InvalidOutput)?;
    if bytes.len() > DAG_MAX_RESULT_BYTES {
        return Err(SeoStepsPureWireError::OutputTooLarge);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy)]
enum WireRole {
    DeserializeInput,
    SerializeOutput,
    SerializeFailure,
}

fn descriptor<T: JsonSchema>(
    artifact: &ArtifactSchema,
    role: WireRole,
) -> Result<OwnerDtoDescriptorV3, OwnerSchemaErrorV3> {
    let settings = SchemaSettings::draft2020_12();
    let generator = match role {
        WireRole::DeserializeInput => settings.for_deserialize(),
        WireRole::SerializeOutput | WireRole::SerializeFailure => settings.for_serialize(),
    };
    let schema = generator.into_generator().into_root_schema_for::<T>();
    let descriptor_bytes = serde_json::to_vec(&schema).map_err(|error| {
        OwnerSchemaErrorV3::InvalidV2Contract(PlanError::Serialization(error.to_string()))
    })?;
    Ok(OwnerDtoDescriptorV3 {
        schema_id: artifact.id.clone(),
        revision: artifact.revision,
        format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
        descriptor_bytes,
    })
}

fn owner_schema<O>(
    failure: OwnerDtoDescriptorV3,
) -> Result<PinnedOwnerOperationSchemaV3, OwnerSchemaErrorV3>
where
    O: OwnerPureOperation,
    O::Input: JsonSchema,
    O::Output: JsonSchema,
{
    let contract = O::contract();
    if contract.inputs.len() != 1 || contract.outputs.len() != 1 {
        return Err(OwnerSchemaErrorV3::PortDescriptorMismatch);
    }
    let (input_port, input_schema) = contract
        .inputs
        .iter()
        .next()
        .ok_or(OwnerSchemaErrorV3::PortDescriptorMismatch)?;
    let (output_port, output_schema) = contract
        .outputs
        .iter()
        .next()
        .ok_or(OwnerSchemaErrorV3::PortDescriptorMismatch)?;
    OwnerOperationSchemaV3 {
        schema_revision: OWNER_SCHEMA_ATTESTATION_REVISION_V3,
        operation: contract.reference.clone(),
        owner: contract.owner.clone(),
        v2_contract_fingerprint: contract.fingerprint()?,
        inputs: BTreeMap::<PortId, OwnerDtoDescriptorV3>::from([(
            input_port.clone(),
            descriptor::<O::Input>(input_schema, WireRole::DeserializeInput)?,
        )]),
        outputs: BTreeMap::<PortId, OwnerDtoDescriptorV3>::from([(
            output_port.clone(),
            descriptor::<O::Output>(output_schema, WireRole::SerializeOutput)?,
        )]),
        failure,
    }
    .seal(&contract)
}

pub fn seo_steps_owner_schemas(
) -> Result<BTreeMap<OperationRef, PinnedOwnerOperationSchemaV3>, OwnerSchemaErrorV3> {
    let entries = [
        owner_schema::<ExtractProceduralRules>(OwnerDtoDescriptorV3 {
            schema_id: ArtifactTypeId::from("seo_steps.infallible_failure"),
            revision: 1,
            format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
            descriptor_bytes: b"false".to_vec(),
        })?,
        owner_schema::<NormalizeSerpQueries>(descriptor::<SerpNormalizeFailureV1>(
            &ArtifactSchema {
                id: ArtifactTypeId::from("seo_steps.serp_normalize_failure"),
                revision: 1,
            },
            WireRole::SerializeFailure,
        )?)?,
        owner_schema::<BuildSeoOpportunities>(descriptor::<OpportunityBuildFailureV1>(
            &ArtifactSchema {
                id: ArtifactTypeId::from("seo_steps.opportunity_build_failure"),
                revision: 1,
            },
            WireRole::SerializeFailure,
        )?)?,
    ];
    Ok(entries
        .into_iter()
        .map(|entry| (entry.schema.operation.clone(), entry))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn all_three_schemas_are_actual_owner_dto_structures_and_failure_is_precise() {
        let schemas = seo_steps_owner_schemas().unwrap();
        assert_eq!(schemas.len(), 3);
        for (operation, entry) in &schemas {
            let contract = [
                ExtractProceduralRules::contract(),
                NormalizeSerpQueries::contract(),
                BuildSeoOpportunities::contract(),
            ]
            .into_iter()
            .find(|contract| &contract.reference == operation)
            .unwrap();
            entry.validate(&contract).unwrap();
            for dto in entry.schema.inputs.values().chain(entry.schema.outputs.values()) {
                let json: Value = serde_json::from_slice(&dto.descriptor_bytes).unwrap();
                assert_eq!(json["$schema"], "https://json-schema.org/draft/2020-12/schema");
                assert_eq!(json["type"], "object");
            }
        }
        assert_eq!(
            schemas[&ExtractProceduralRules::contract().reference]
                .schema.failure.descriptor_bytes,
            b"false"
        );
        for operation in [
            NormalizeSerpQueries::contract().reference,
            BuildSeoOpportunities::contract().reference,
        ] {
            let schema: Value =
                serde_json::from_slice(&schemas[&operation].schema.failure.descriptor_bytes)
                    .unwrap();
            assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
            assert_ne!(schema, Value::Bool(false));
        }
    }

    #[test]
    fn exact_input_rejects_unknown_and_domain_failure_keeps_typed_payload() {
        let op = NormalizeSerpQueries::contract().reference;
        let good = br#"{"run_id":"run-a","queries":["travel visa"]}"#;
        validate_seo_steps_pure_input(&op, good).unwrap();
        assert!(execute_seo_steps_pure(&op, good).is_ok());
        assert_eq!(
            validate_seo_steps_pure_input(
                &op,
                br#"{"run_id":"run-a","queries":[],"unexpected":true}"#,
            ),
            Err(SeoStepsPureWireError::InvalidInput)
        );
        assert_eq!(
            validate_seo_steps_pure_input(&op, br#"{"run_id":"r","run_id":"s","queries":[]}"#),
            Err(SeoStepsPureWireError::InvalidInput)
        );
        let bad = br#"{"run_id":" ","queries":[]}"#;
        match execute_seo_steps_pure(&op, bad).unwrap_err() {
            SeoStepsPureWireError::DomainFailure { operation, failure_json } => {
                assert_eq!(operation, op);
                let failure: crate::owner_pure_operations::SerpNormalizeFailureV1 =
                    serde_json::from_slice(&failure_json).unwrap();
                assert_eq!(
                    failure,
                    crate::owner_pure_operations::SerpNormalizeFailureV1::MissingRunId
                );
            }
            other => panic!("expected typed domain failure, got {other:?}"),
        }
    }
}
