//! Narrow JSON descriptor wire boundary, not an owner DTO/payload decoder.
//!
//! JSON Schema dialect and root-shape checks are a defense-in-depth ingress
//! guard. Only the owner-local generated schema + typed codec can attest that
//! every declared schema field actually describes the compiled Rust DTO.

use std::fmt;

use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;

const DRAFT_2020_12_URI: &str = "https://json-schema.org/draft/2020-12/schema";

/// `serde_json::Value` alone has last-wins map semantics on duplicate keys.
/// Reject repeated keys recursively before judging any descriptor syntax.
struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Bool(value)))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(
                    serde_json::Number::from_f64(value)
                        .ok_or_else(|| E::custom("invalid JSON number"))?,
                )))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value.to_owned())))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value)))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }

            fn visit_some<D: serde::Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                UniqueJson::deserialize(deserializer)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(value)) = sequence.next_element::<UniqueJson>()? {
                    values.push(value);
                }
                Ok(UniqueJson(Value::Array(values)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueJson(value))) =
                    map.next_entry::<String, UniqueJson>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(A::Error::custom("duplicate owner descriptor JSON key"));
                    }
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(crate) fn validate_json_schema_2020_12(bytes: &[u8]) -> Result<(), &'static str> {
    let UniqueJson(schema) =
        serde_json::from_slice(bytes).map_err(|_| "invalid/ambiguous descriptor JSON")?;
    match schema {
        // The impossible payload schema is the wire shape of Infallible.
        Value::Bool(false) => Ok(()),
        Value::Object(object) => {
            if object.get("$schema").and_then(Value::as_str) != Some(DRAFT_2020_12_URI) {
                return Err("owner schema dialect must be JSON Schema 2020-12");
            }
            if ![
                "type", "properties", "items", "prefixItems", "enum", "const", "$ref",
                "oneOf", "anyOf", "allOf", "not", "if", "patternProperties",
            ]
            .iter()
            .any(|key| object.contains_key(*key))
            {
                return Err("schema does not constrain the root payload");
            }
            if let Some(kind) = object.get("type") {
                let valid_type = |name: &str| {
                    matches!(
                        name,
                        "null" | "boolean" | "object" | "array" | "number" | "integer" | "string"
                    )
                };
                match kind {
                    Value::String(name) if valid_type(name) => {}
                    Value::Array(names)
                        if !names.is_empty()
                            && names
                                .iter()
                                .all(|name| name.as_str().is_some_and(valid_type)) => {}
                    _ => return Err("invalid JSON Schema root type"),
                }
            }
            Ok(())
        }
        _ => Err("owner descriptor is not a structural JSON Schema"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ingress_rejects_unknown_dialect_and_nested_duplicates() {
        let valid = br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"a":{"type":"string"}}}"#;
        assert!(validate_json_schema_2020_12(valid).is_ok());
        assert!(validate_json_schema_2020_12(b"false").is_ok());
        assert!(validate_json_schema_2020_12(b"true").is_err());
        assert!(validate_json_schema_2020_12(br#"{"type":"object"}"#).is_err());
        let nested_duplicate = br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"a":{"type":"string","type":"integer"}}}"#;
        assert!(validate_json_schema_2020_12(nested_duplicate).is_err());
    }
}
