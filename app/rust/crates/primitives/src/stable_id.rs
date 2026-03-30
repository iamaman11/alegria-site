use crate::hash::blake3_hex;

pub fn stable_rule_instance_id(parts: &[&str]) -> String {
    let joined = parts.join("|");
    blake3_hex(joined.as_bytes())
}

pub fn stable_exception_id(parts: &[&str]) -> String {
    let joined = parts.join("|");
    blake3_hex(joined.as_bytes())
}
