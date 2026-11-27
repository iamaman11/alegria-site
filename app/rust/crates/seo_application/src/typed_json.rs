//! Shared checked Serde/JSON representation at operation boundaries.
//!
//! The Site Lab authoring adapter and the pure DAG artifact adapter both use
//! this policy. It does not replace owner DTOs or historical Protobuf messages.

use serde::{
    de::{Error as _, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::Value;

/// Check that every value supplied by a caller survives decoding into its
/// owner's DTO. DTO defaults may add fields, but unknown keys, skipped fields,
/// and changed values may never be silently ignored at admission.
pub(crate) fn preserves_authored_fields(authored: &Value, normalized: &Value) -> bool {
    match (authored, normalized) {
        (Value::Object(given), Value::Object(typed)) => given.iter().all(|(key, value)| {
            typed
                .get(key)
                .is_some_and(|typed_value| preserves_authored_fields(value, typed_value))
        }),
        (Value::Array(given), Value::Array(typed)) => {
            given.len() == typed.len()
                && given
                    .iter()
                    .zip(typed)
                    .all(|(given, typed)| preserves_authored_fields(given, typed))
        }
        _ => authored == normalized,
    }
}

/// Compare with the JSON the owner would actually put on the wire.
/// `serde_json::to_value` can widen an f32 to its precise binary f64 value,
/// whereas `to_vec` emits the shortest round-trippable decimal. Reject
/// duplicates even if an owner uses `#[serde(flatten)]` or a custom Serializer.
pub(crate) fn normalized_wire_json_value<T: Serialize>(
    value: &T,
) -> Result<Value, serde_json::Error> {
    let wire = serde_json::to_vec(value)?;
    parse_unique_json_bytes(&wire)
}

/// `serde_json::Value` silently keeps the last repeated object key. Inspect
/// the complete raw wire before that lossy map conversion, at every depth.
pub(crate) struct UniqueJsonValue(pub(crate) Value);

pub(crate) fn parse_unique_json_bytes(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UniqueJsonValue>(bytes).map(|parsed| parsed.0)
}

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueJsonVisitor;

        impl<'de> Visitor<'de> for UniqueJsonVisitor {
            type Value = Value;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object fields")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
                Ok(Value::from(value))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
                Ok(Value::from(value))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
                Ok(Value::String(value.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
                Ok(Value::String(value))
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
                UniqueJsonValue::deserialize(deserializer).map(|value| value.0)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
                    values.push(value.0);
                }
                Ok(Value::Array(values))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom("duplicate operation JSON field"));
                    }
                    let value = object.next_value::<UniqueJsonValue>()?;
                    values.insert(key, value.0);
                }
                Ok(Value::Object(values))
            }
        }

        deserializer.deserialize_any(UniqueJsonVisitor).map(Self)
    }
}
