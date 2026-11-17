fn parse_generated_sections(text: &str) -> Result<GeneratedSections, DomainError> {
    let value: Value = serde_json::from_str(text).map_err(|e| DomainError::InfraUnavailable {
        message: format!("llm output is not valid JSON: {e}"),
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| DomainError::InfraUnavailable {
            message: "llm output must be a JSON object".to_string(),
        })?;
    if !object.contains_key("sections")
        || object.len() > 2
        || object.keys().any(|k| k != "sections" && k != "metadata")
    {
        return Err(DomainError::InfraUnavailable {
            message: "llm output allows only sections and optional metadata".to_string(),
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
        let markdown = body
            .as_str()
            .filter(|body| !body.trim().is_empty())
            .ok_or_else(|| DomainError::InfraUnavailable {
                message: format!("llm section {role} must be a non-empty string"),
            })?;
        sections.insert(role.clone(), markdown.trim().to_string());
    }

    let (seo_title, seo_meta_description) = if let Some(metadata) = object.get("metadata") {
        let metadata = metadata
            .as_object()
            .filter(|m| m.len() == 2 && m.contains_key("title") && m.contains_key("description"))
            .ok_or_else(|| DomainError::ValidationFailure {
                message: "editorial metadata requires only title and description".to_string(),
            })?;
        let get = |key| -> Result<String, DomainError> {
            let value = metadata
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let chars = value.chars().count();
            let (min, max) = if key == "title" { (15, 110) } else { (45, 190) };
            if chars < min
                || chars > max
                || value
                    .chars()
                    .any(|ch| matches!(ch, '<' | '>' | '\n' | '\r' | '\0'))
            {
                return Err(DomainError::ValidationFailure {
                    message: format!("editorial metadata {key} failed length/plain-text checks"),
                });
            }
            Ok(value.to_string())
        };
        (get("title")?, get("description")?)
    } else {
        (String::new(), String::new())
    };
    Ok(GeneratedSections {
        sections,
        seo_title,
        seo_meta_description,
    })
}

#[cfg(test)]
mod strict_json_shape_tests {
    use super::*;

    #[test]
    fn legacy_sections_and_typed_localized_metadata_are_accepted() {
        assert!(parse_generated_sections(
            r#"{"sections":{"intro":"Evidence-based introduction"}}"#
        )
        .is_ok());
        let ru = parse_generated_sections(r#"{
          "sections":{"overview":"Визовые требования из официальных источников"},
          "metadata":{
            "title":"Шенгенская виза: документы и правила подачи",
            "description":"Проверенные документы, сборы и сроки оформления на основе действующих официальных источников без неподтверждённых обещаний."
          }
        }"#).unwrap();
        assert!(ru.seo_title.contains("Шенгенская виза"));
        assert!(ru.seo_meta_description.contains("Проверенные"));
    }

    #[test]
    fn section_contract_rejects_incomplete_or_invalid_payloads() {
        for payload in [
            "invalid",
            "{}",
            r#"{"sections":{}}"#,
            r#"{"sections":{"intro":null}}"#,
            r#"{"sections":{"intro":"  "}}"#,
            r#"{"sections":{"intro":"ok"},"approved":true}"#,
            r#"{"sections":{"overview":"ok"},"metadata":{"title":"Short","description":"Short"}}"#,
            r#"{"sections":{"overview":"ok"},"metadata":{"title":"Valid explanatory headline","description":"A valid, sufficiently detailed description for the title and content","approval":"yes"}}"#,
            r#"{"sections":{"overview":"ok"},"metadata":null}"#,
        ] {
            assert!(parse_generated_sections(payload).is_err(), "{payload}");
        }
    }
}
