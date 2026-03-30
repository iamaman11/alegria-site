use crate::hash::blake3_hex;

pub fn qdrant_point_id_v1(collection_name: &str, entity_type: &str, entity_key: &str) -> String {
    let joined = format!("{collection_name}|{entity_type}|{entity_key}");
    blake3_hex(joined.as_bytes())
}
