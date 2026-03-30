use anyhow::{bail, Result};

pub fn normalize_context_key(
    country_code: &str,
    visa_family: &str,
    visa_subtype: Option<&str>,
    citizenship_code: &str,
) -> Result<String> {
    let country = country_code.trim().to_uppercase();
    let family = visa_family.trim().to_lowercase();
    let subtype = visa_subtype.unwrap_or("").trim().to_lowercase();
    let citizenship = citizenship_code.trim().to_uppercase();

    if country.is_empty() || family.is_empty() || citizenship.is_empty() {
        bail!("context key parts must be non-empty");
    }

    Ok(format!("{country}|{family}|{subtype}|{citizenship}"))
}
