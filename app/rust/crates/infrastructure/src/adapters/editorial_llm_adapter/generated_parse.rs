fn parse_generated_sections(text: &str) -> Result<GeneratedSections, DomainError> {
    let value: Value = serde_json::from_str(text).map_err(|e| DomainError::InfraUnavailable {
        message: format!("llm output is not valid JSON: {e}"),
    })?;
    let object = value.as_object().ok_or_else(|| DomainError::InfraUnavailable {
        message: "llm output must be a JSON object".to_string(),
    })?;
    if object.len() != 1 || !object.contains_key("sections") {
        return Err(DomainError::InfraUnavailable {
            message: "llm output must contain only the sections key".to_string(),
        });
    }
    let sections_value = object
        .get("sections")
        .and_then(Value::as_object)
        .filter(|sections| !sections.is_empty())
        .ok_or_else(|| DomainError::InfraUnavailable {
            message: "llm output requires a non-empty sections object".to_string(),
        })?;
    let mut sections = BTreeMap::new();
    for (role, body) in sections_value {
        let markdown = body.as_str().filter(|body| !body.trim().is_empty()).ok_or_else(|| {
            DomainError::InfraUnavailable {
                message: format!("llm section {role} must be a non-empty string"),
            }
        })?;
        sections.insert(role.clone(), markdown.trim().to_string());
    }
    Ok(GeneratedSections { sections })
}

#[cfg(test)]
mod strict_json_shape_tests {
    use super::*;

    #[test]
    fn section_contract_rejects_incomplete_or_invalid_payloads() {
        assert!(parse_generated_sections(r#"{"sections":{"intro":"Evidence-based introduction"}}"#).is_ok());
        for payload in [
            "invalid",
            "{}",
            r#"{"sections":{}}"#,
            r#"{"sections":{"intro":null}}"#,
            r#"{"sections":{"intro":"  "}}"#,
            r#"{"sections":{"intro":"ok"},"approved":true}"#,
        ] {
            assert!(parse_generated_sections(payload).is_err(), "{payload}");
        }
    }
}
