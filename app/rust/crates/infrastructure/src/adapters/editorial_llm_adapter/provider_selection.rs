fn selected_provider_policy(requested_policy: &str) -> String {
    let requested = requested_policy.trim();
    let env_provider = std::env::var("SEO_LLM_PROVIDER").unwrap_or_default();
    if requested.is_empty() || requested == "multi_provider:first_available" {
        if env_provider.is_empty() {
            "multi_provider:first_available".to_string()
        } else {
            env_provider
        }
    } else {
        requested.to_string()
    }
}

pub async fn generate_editorial_draft(
    input: &EditorialDraftGenerateInputPayload,
) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
    let policy = if input.provider_policy.trim().is_empty() {
        input
            .request
            .as_ref()
            .map(|request| request.provider_policy.as_str())
            .unwrap_or("multi_provider:first_available")
    } else {
        input.provider_policy.as_str()
    };
    let selected_policy = selected_provider_policy(policy);
    let fallback_allowed = allow_fallback_on_error();

    if selected_policy != "multi_provider:first_available" {
        for client in ordered_clients() {
            if client.provider_key() != selected_policy {
                continue;
            }
            if !client.is_configured() {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "editorial LLM provider `{selected_policy}` is not configured"
                    ),
                });
            }
            let model_key = client.model_key();
            return client.generate(input).await.map(|candidate| {
                EditorialDraftGenerateOutputPayload {
                    candidate: Some(candidate),
                    provider_key: selected_policy.clone(),
                    model_key,
                    status: "done".to_string(),
                }
            });
        }
        return Err(DomainError::ValidationFailure {
            message: format!("unknown editorial LLM provider policy `{selected_policy}`"),
        });
    }

    let clients = ordered_clients();
    let mut last_error: Option<DomainError> = None;
    let mut saw_real_provider = false;
    for client in clients {
        if client.provider_key() != "deterministic_fixture" {
            saw_real_provider = true;
        }
        if !client.is_configured() {
            continue;
        }
        let provider_key = client.provider_key().to_string();
        let model_key = client.model_key();
        match client.generate(input).await {
            Ok(candidate) => {
                return Ok(EditorialDraftGenerateOutputPayload {
                    candidate: Some(candidate),
                    provider_key,
                    model_key,
                    status: "done".to_string(),
                });
            }
            Err(error) => {
                if provider_key == "deterministic_fixture" || !fallback_allowed {
                    last_error = Some(error);
                } else {
                    last_error = Some(error);
                    continue;
                }
            }
        }
    }

    if !saw_real_provider {
        let deterministic = DeterministicEditorialClient;
        let model_key = deterministic.model_key();
        let candidate = deterministic.generate(input).await?;
        return Ok(EditorialDraftGenerateOutputPayload {
            candidate: Some(candidate),
            provider_key: deterministic.provider_key().to_string(),
            model_key,
            status: "done".to_string(),
        });
    }

    if fallback_allowed {
        let deterministic = DeterministicEditorialClient;
        let model_key = deterministic.model_key();
        let candidate = deterministic.generate(input).await?;
        return Ok(EditorialDraftGenerateOutputPayload {
            candidate: Some(candidate),
            provider_key: deterministic.provider_key().to_string(),
            model_key,
            status: "done:fallback".to_string(),
        });
    }

    Err(last_error.unwrap_or_else(|| DomainError::InfraUnavailable {
        message: "no editorial LLM provider is configured".to_string(),
    }))
}
