//! Acquisition-owned JSON wire codec and structural DTO attestations.
//!
//! This is the only external JSON boundary for these two pure operations.
//! The acquisition domain remains independent of the generic DAG runner and
//! never opens a provider, SQL connection, or Temporal connection. A separate
//! Temporal worker must still establish scope, source admission and History.

use std::collections::BTreeMap;
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, Operation, OperationRef, OwnerDtoDescriptorFormatV3,
    OwnerDtoDescriptorV3, OwnerOperationSchemaV3, OwnerSchemaErrorV3, PinnedOwnerOperationSchemaV3,
    PlanError, PortId, DAG_MAX_INPUT_BYTES, DAG_MAX_RESULT_BYTES,
    OWNER_SCHEMA_ATTESTATION_REVISION_V3,
};
use schemars::{generate::SchemaSettings, JsonSchema};
use serde::{de::DeserializeOwned, Serialize};

use crate::operations::{ClassifySourcePage, EvaluateDomBlocks, PureAcquisitionOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionPureWireError {
    UnknownOperation,
    InputTooLarge,
    InvalidInput,
    InvalidOutput,
    OutputTooLarge,
}

/// ValidateOnly performs exact typed decode without evaluating the business
/// function or interpreting the result as a remote-source observation.
pub fn validate_acquisition_pure_input(
    operation: &OperationRef,
    payload: &[u8],
) -> Result<(), AcquisitionPureWireError> {
    if payload.len() > DAG_MAX_INPUT_BYTES {
        return Err(AcquisitionPureWireError::InputTooLarge);
    }
    if operation == &ClassifySourcePage::contract().reference {
        return decode_input::<ClassifySourcePage>(payload);
    }
    if operation == &EvaluateDomBlocks::contract().reference {
        return decode_input::<EvaluateDomBlocks>(payload);
    }
    Err(AcquisitionPureWireError::UnknownOperation)
}

fn decode_input<O>(payload: &[u8]) -> Result<(), AcquisitionPureWireError>
where
    O: PureAcquisitionOperation,
{
    serde_json::from_slice::<O::Input>(payload)
        .map(|_| ())
        .map_err(|_| AcquisitionPureWireError::InvalidInput)
}

/// Exact, versioned leaf-owner dispatch. No request-supplied Rust type names,
/// arbitrary URLs, route maps or effect authorization are accepted. The
/// outer, independently authorized Temporal executor must check RunScope.
pub fn execute_acquisition_pure(
    operation: &OperationRef,
    payload: &[u8],
) -> Result<Vec<u8>, AcquisitionPureWireError> {
    if payload.len() > DAG_MAX_INPUT_BYTES {
        return Err(AcquisitionPureWireError::InputTooLarge);
    }
    if operation == &ClassifySourcePage::contract().reference {
        return evaluate::<ClassifySourcePage>(payload);
    }
    if operation == &EvaluateDomBlocks::contract().reference {
        return evaluate::<EvaluateDomBlocks>(payload);
    }
    Err(AcquisitionPureWireError::UnknownOperation)
}

fn evaluate<O>(payload: &[u8]) -> Result<Vec<u8>, AcquisitionPureWireError>
where
    O: PureAcquisitionOperation,
    O::Input: DeserializeOwned,
    O::Output: Serialize,
{
    let input: O::Input =
        serde_json::from_slice(payload).map_err(|_| AcquisitionPureWireError::InvalidInput)?;
    let output = O::evaluate(&input);
    let bytes = serde_json::to_vec(&output).map_err(|_| AcquisitionPureWireError::InvalidOutput)?;
    if bytes.len() > DAG_MAX_RESULT_BYTES {
        return Err(AcquisitionPureWireError::OutputTooLarge);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy)]
enum WireRole {
    DeserializeInput,
    SerializeOutput,
}

fn dto_descriptor<T: JsonSchema>(
    artifact: &ArtifactSchema,
    role: WireRole,
) -> Result<OwnerDtoDescriptorV3, OwnerSchemaErrorV3> {
    let settings = SchemaSettings::draft2020_12();
    let generator = match role {
        WireRole::DeserializeInput => settings.for_deserialize(),
        WireRole::SerializeOutput => settings.for_serialize(),
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

fn generated_owner_schema<O>() -> Result<PinnedOwnerOperationSchemaV3, OwnerSchemaErrorV3>
where
    O: PureAcquisitionOperation<Failure = Infallible>,
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
            dto_descriptor::<O::Input>(input_schema, WireRole::DeserializeInput)?,
        )]),
        outputs: BTreeMap::<PortId, OwnerDtoDescriptorV3>::from([(
            output_port.clone(),
            dto_descriptor::<O::Output>(output_schema, WireRole::SerializeOutput)?,
        )]),
        failure: OwnerDtoDescriptorV3 {
            schema_id: ArtifactTypeId::from("acquisition.infallible_failure"),
            revision: 1,
            format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
            descriptor_bytes: b"false".to_vec(),
        },
    }
    .seal(&contract)
}

/// Structural schemas are generated only from this owner's compiled DTOs.
/// They may be installed in a root-owned trusted catalog but never self-issued
/// as an admission credential by an untrusted caller.
pub fn acquisition_owner_schemas(
) -> Result<BTreeMap<OperationRef, PinnedOwnerOperationSchemaV3>, OwnerSchemaErrorV3> {
    let entries = [
        generated_owner_schema::<ClassifySourcePage>()?,
        generated_owner_schema::<EvaluateDomBlocks>()?,
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
    fn both_owner_schemas_are_generated_from_real_input_and_output_dtos() {
        let schemas = acquisition_owner_schemas().unwrap();
        assert_eq!(schemas.len(), 2);
        for (operation, pinned) in schemas {
            let contract = if operation == ClassifySourcePage::contract().reference {
                ClassifySourcePage::contract()
            } else {
                EvaluateDomBlocks::contract()
            };
            pinned.validate(&contract).unwrap();
            for descriptor in pinned
                .schema
                .inputs
                .values()
                .chain(pinned.schema.outputs.values())
            {
                let schema: Value = serde_json::from_slice(&descriptor.descriptor_bytes).unwrap();
                assert_eq!(
                    schema["$schema"],
                    "https://json-schema.org/draft/2020-12/schema"
                );
                assert_eq!(schema["type"], "object");
                assert!(schema["properties"].is_object());
            }
            assert_eq!(pinned.schema.failure.descriptor_bytes, b"false");
        }
    }

    #[test]
    fn source_dto_schema_reflects_nested_blocks_and_serde_enum_variants() {
        let schemas = acquisition_owner_schemas().unwrap();
        let blocks = &schemas[&EvaluateDomBlocks::contract().reference];
        let schema: Value =
            serde_json::from_slice(&blocks.schema.inputs[&PortId::from("input")].descriptor_bytes)
                .unwrap();
        assert_eq!(schema["properties"]["blocks"]["type"], "array");
        assert_eq!(schema["$defs"]["StrictDomBlockInput"]["type"], "object");
        let enum_variants = &schema["$defs"]["BlockRole"]["enum"];
        assert!(enum_variants
            .as_array()
            .unwrap()
            .contains(&Value::String("content_main".to_owned())));
        assert!(enum_variants
            .as_array()
            .unwrap()
            .contains(&Value::String("navigation".to_owned())));
    }

    #[test]
    fn leaf_owner_executes_exactly_two_operations_and_fails_closed() {
        let page = execute_acquisition_pure(
            &ClassifySourcePage::contract().reference,
            br#"{"url":"https://example.org/menu","title":"Menu","raw_text":"Visa fee EUR 25"}"#,
        )
        .unwrap();
        let page: Value = serde_json::from_slice(&page).unwrap();
        assert_eq!(page["page_mode"], "menu_page");
        let blocks = execute_acquisition_pure(
            &EvaluateDomBlocks::contract().reference,
            br#"{"blocks":[{"dom_block_id":"body","block_role":"content_main","text":"Visa fee EUR 25"}]}"#,
        )
        .unwrap();
        let blocks: Value = serde_json::from_slice(&blocks).unwrap();
        assert_eq!(blocks["blocks"][0]["allow_extraction"], true);

        let bad = br#"{"blocks":[{"dom_block_id":"body","block_role":"content_main","text":"x","extra":true}]}"#;
        assert_eq!(
            execute_acquisition_pure(&EvaluateDomBlocks::contract().reference, bad),
            Err(AcquisitionPureWireError::InvalidInput)
        );
        assert_eq!(
            validate_acquisition_pure_input(&EvaluateDomBlocks::contract().reference, bad),
            Err(AcquisitionPureWireError::InvalidInput)
        );
        assert_eq!(
            execute_acquisition_pure(
                &ClassifySourcePage::contract().reference,
                br#"{"url":"a","url":"b","title":"x","raw_text":"x"}"#
            ),
            Err(AcquisitionPureWireError::InvalidInput)
        );
        assert_eq!(
            execute_acquisition_pure(
                &OperationRef {
                    id: "acquisition.unknown".into(),
                    revision: 1,
                },
                b"{}"
            ),
            Err(AcquisitionPureWireError::UnknownOperation)
        );
        assert_eq!(
            execute_acquisition_pure(
                &ClassifySourcePage::contract().reference,
                &vec![b'x'; DAG_MAX_INPUT_BYTES + 1]
            ),
            Err(AcquisitionPureWireError::InputTooLarge)
        );
    }

    #[test]
    fn changed_owner_dto_changes_structural_descriptor() {
        #[derive(JsonSchema)]
        struct Original {
            keyword: String,
        }
        #[derive(JsonSchema)]
        struct Changed {
            keyword: Vec<String>,
        }
        let schema = ArtifactSchema {
            id: ArtifactTypeId::from("test.acquisition"),
            revision: 1,
        };
        assert_ne!(
            dto_descriptor::<Original>(&schema, WireRole::DeserializeInput)
                .unwrap()
                .descriptor_bytes,
            dto_descriptor::<Changed>(&schema, WireRole::DeserializeInput)
                .unwrap()
                .descriptor_bytes
        );
    }
}
