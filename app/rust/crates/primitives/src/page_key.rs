use anyhow::{bail, Result};

pub fn normalize_page_key(url_path: &str) -> Result<String> {
    let path = url_path.trim();
    if !path.starts_with('/') {
        bail!("url_path must start with /");
    }
    Ok(path.to_string())
}
