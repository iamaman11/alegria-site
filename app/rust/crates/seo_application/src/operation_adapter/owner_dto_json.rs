//! Test-only JSON inspection of generated owner schema descriptors.
//! No dynamic JSON is admitted to the production owner-schema factory.

use super::*;
use serde_json::Value;

fn descriptor_json(descriptor: &OwnerDtoDescriptorV3) -> Value {
    serde_json::from_slice(&descriptor.descriptor_bytes)
        .expect("derived JSON Schema is valid JSON")
}

#[test]
fn schema_is_derived_for_each_real_installed_semantic_contract() {
    let schemas = semantic_extraction_owner_schemas().unwrap();
    assert_eq!(schemas.len(), 5);
    for schema in schemas.values() {
        let installed = super::super::semantic_extraction_registry()
            .unwrap()
            .catalog()
            .contract(&schema.schema.operation)
            .cloned()
            .expect("every derived schema belongs to installed operation");
        schema.validate(&installed).unwrap();
        for desc in schema
            .schema
            .inputs
            .values()
            .chain(schema.schema.outputs.values())
        {
            let json = descriptor_json(desc);
            assert_eq!(json["$schema"], "https://json-schema.org/draft/2020-12/schema");
            assert_eq!(json["type"], "object");
            assert!(json["properties"].is_object());
        }
        assert_eq!(descriptor_json(&schema.schema.failure), Value::Bool(false));
    }
}

#[test]
fn generated_schema_includes_optional_fields_nested_structs_and_serde_enum_variants() {
    let schemas = semantic_extraction_owner_schemas().unwrap();
    let layer = &schemas[&LayerRoute::contract().reference];
    let output = descriptor_json(&layer.schema.outputs[&PortId::from("output")]);
    assert!(output["properties"]["hitl_reason"].is_object());
    assert!(output["properties"]["layer_scores"].is_object());
    assert!(output["$defs"]["LayerScores"]["properties"]["procedural"].is_object());
    assert!(!output["required"]
        .as_array()
        .unwrap()
        .contains(&Value::String("hitl_reason".into())));

    let mapping = &schemas[&CanonicalizeMentions::contract().reference];
    let mapping_output = descriptor_json(&mapping.schema.outputs[&PortId::from("output")]);
    let enum_values = &mapping_output["$defs"]["MatchingStage"]["enum"];
    assert!(enum_values
        .as_array()
        .unwrap()
        .contains(&Value::String("vector_qdrant".into())));
}

#[test]
fn dto_field_type_and_variant_changes_produce_different_structural_descriptors() {
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct OldDto {
        item: String,
        optional: Option<u32>,
    }
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct NewDto {
        item: u32,
        optional: Option<u32>,
    }
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    enum OldEnum {
        First,
        Second,
    }
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    enum NewEnum {
        First,
        Third,
    }
    let artifact = ArtifactSchema {
        id: ArtifactTypeId::from("test.field"),
        revision: 1,
    };
    assert_ne!(
        descriptor_from_dto::<OldDto>(&artifact, DtoWireRole::InputDeserialize)
            .unwrap()
            .descriptor_bytes,
        descriptor_from_dto::<NewDto>(&artifact, DtoWireRole::InputDeserialize)
            .unwrap()
            .descriptor_bytes
    );
    assert_ne!(
        descriptor_from_dto::<OldEnum>(&artifact, DtoWireRole::InputDeserialize)
            .unwrap()
            .descriptor_bytes,
        descriptor_from_dto::<NewEnum>(&artifact, DtoWireRole::InputDeserialize)
            .unwrap()
            .descriptor_bytes
    );
    let descriptor = descriptor_json(
        &descriptor_from_dto::<OldDto>(&artifact, DtoWireRole::InputDeserialize).unwrap(),
    );
    assert!(descriptor["required"]
        .as_array()
        .unwrap()
        .contains(&Value::String("item".into())));
    assert!(!descriptor["required"]
        .as_array()
        .unwrap()
        .contains(&Value::String("optional".into())));
}

#[test]
fn direction_sensitive_serde_wire_is_reflected_in_generated_input_and_output() {
    #[derive(JsonSchema, serde::Serialize, serde::Deserialize)]
    #[allow(dead_code)]
    struct DirectionalDto {
        #[serde(rename(serialize = "sent", deserialize = "received"))]
        name: String,
        #[serde(skip_serializing)]
        private_input: String,
    }
    let artifact = ArtifactSchema {
        id: ArtifactTypeId::from("test.directional"),
        revision: 1,
    };
    let decoded = descriptor_from_dto::<DirectionalDto>(&artifact, DtoWireRole::InputDeserialize)
        .unwrap();
    let encoded = descriptor_from_dto::<DirectionalDto>(&artifact, DtoWireRole::OutputSerialize)
        .unwrap();
    assert_ne!(decoded.descriptor_bytes, encoded.descriptor_bytes);
    let decode_json = descriptor_json(&decoded);
    let encode_json = descriptor_json(&encoded);
    assert!(decode_json["properties"]["received"].is_object());
    assert!(decode_json["properties"]["private_input"].is_object());
    assert!(encode_json["properties"]["sent"].is_object());
    assert!(encode_json["properties"].get("private_input").is_none());
}
