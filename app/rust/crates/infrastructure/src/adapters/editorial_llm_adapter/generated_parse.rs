fn parse_generated_sections(text: &str) -> Result<GeneratedSections, DomainError> {
    let value: Value = serde_json::from_str(text).map_err(|e| DomainError::InfraUnavailable {
        message: format!("llm output is not valid JSON: {e}"),
    })?;
    let sections_value = value
        .get("sections")
        .and_then(Value::as_object)
        .ok_or_else(|| DomainError::InfraUnavailable {
            message: "llm output missing `sections` object".to_string(),
        })?;
    let mut sections = BTreeMap::new();
    for (role, body) in sections_value {
        if let Some(markdown) = body.as_str() {
            sections.insert(role.clone(), markdown.trim().to_string());
        }
    }
    Ok(GeneratedSections { sections })
}
