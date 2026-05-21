use uuid::Uuid;

pub fn qdrant_point_id_v1(collection_name: &str, entity_type: &str, entity_key: &str) -> String {
    let joined = format!("{collection_name}|{entity_type}|{entity_key}");
    let digest = blake3::hash(joined.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

pub fn is_valid_qdrant_point_id(point_id: &str) -> bool {
    point_id.parse::<u64>().is_ok() || Uuid::parse_str(point_id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_qdrant_point_id, qdrant_point_id_v1};

    #[test]
    fn qdrant_point_id_is_deterministic_and_valid() {
        let a = qdrant_point_id_v1("content_chunks", "raw_section", "raw_section:42");
        let b = qdrant_point_id_v1("content_chunks", "raw_section", "raw_section:42");
        let c = qdrant_point_id_v1("content_chunks", "raw_section", "raw_section:43");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(is_valid_qdrant_point_id(&a));
    }
}
