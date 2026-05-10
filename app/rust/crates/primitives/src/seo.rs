use crate::hash::blake3_hex;

fn normalize_component(input: &str) -> String {
    input
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn canonical_scope_tuple(parts: &[(&str, &str)]) -> String {
    let mut normalized = parts
        .iter()
        .map(|(key, value)| (normalize_component(key), normalize_component(value)))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort_by(|a, b| a.0.cmp(&b.0));
    normalized
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn scope_signature(parts: &[(&str, &str)]) -> String {
    blake3_hex(canonical_scope_tuple(parts).as_bytes())
}

pub fn seo_artifact_key(prefix: &str, parts: &[&str]) -> String {
    let mut material = Vec::with_capacity(parts.len() + 1);
    material.push(prefix);
    material.extend_from_slice(parts);
    blake3_hex(material.join("|").as_bytes())
}

pub fn canonical_slug(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn clamp_score(score: f64) -> f64 {
    if score.is_nan() {
        0.0
    } else {
        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_scope_tuple, canonical_slug, scope_signature};

    #[test]
    fn scope_tuple_is_order_and_case_stable() {
        let a = canonical_scope_tuple(&[("Market", "BY"), ("Locale", "ru-RU")]);
        let b = canonical_scope_tuple(&[("locale", " RU-RU "), ("market", "by")]);
        assert_eq!(a, b);
        assert_eq!(
            scope_signature(&[("Market", "BY")]),
            scope_signature(&[("market", " by ")])
        );
    }

    #[test]
    fn slug_is_ascii_and_stable() {
        assert_eq!(
            canonical_slug(" Tourist Visa / Belarus "),
            "tourist-visa-belarus"
        );
    }
}
