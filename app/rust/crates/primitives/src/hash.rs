use blake3::Hasher;

pub fn blake3_hex(input: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(input);
    hasher.finalize().to_hex().to_string()
}

pub fn content_hash_v1(content: &str) -> String {
    blake3_hex(content.as_bytes())
}

pub fn stable_u64_from_parts(parts: &[&str]) -> u64 {
    let joined = parts.join("|");
    let hex = blake3_hex(joined.as_bytes());
    u64::from_str_radix(&hex[..16], 16).unwrap_or(0)
}

pub fn compute_triple_id(
    subject_key: &str,
    relation_type: &str,
    object_key: &str,
    params_bytes: &[u8],
    conditions_key: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(subject_key.as_bytes());
    hasher.update(relation_type.as_bytes());
    hasher.update(object_key.as_bytes());
    hasher.update(params_bytes);
    hasher.update(conditions_key.as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub fn compute_conditions_key(conditions_raw: &str) -> String {
    if conditions_raw.trim().is_empty() {
        return String::new();
    }
    blake3_hex(conditions_raw.trim().as_bytes())
}
