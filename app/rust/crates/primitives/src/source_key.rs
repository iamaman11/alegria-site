use anyhow::{bail, Result};

pub fn normalize_source_key(raw: &str) -> Result<String> {
    let key = raw.trim().to_lowercase().replace(' ', "_");
    if key.is_empty() {
        bail!("empty source_key");
    }
    Ok(key)
}
