impl EditorialLlmClient for OpenAiEditorialClient {
    fn provider_key(&self) -> &'static str {
        "openai"
    }

    fn model_key(&self) -> String {
        std::env::var("OPENAI_SEO_MODEL").unwrap_or_else(|_| "gpt-5.2".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("OPENAI_API_KEY").is_ok()
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key =
                std::env::var("OPENAI_API_KEY").map_err(|_| DomainError::InfraUnavailable {
                    message: "OPENAI_API_KEY is not set".to_string(),
                })?;
            let prompt = prompt_from_input(input)?;
            let body = json!({
                "model": self.model_key(),
                "temperature": 0.2,
                "response_format": { "type": "json_object" },
                "messages": [
                    {
                        "role": "system",
                        "content": "You write concise expert travel-visa editorial prose. Never invent facts. Use only provided support snippets. Return JSON only."
                    },
                    {
                        "role": "user",
                        "content": prompt
                    }
                ]
            });
            let value = post_json_with_retries(
                OPENAI_CHAT_COMPLETIONS_URL,
                openai_headers(&api_key)?,
                &body,
                request_timeout(),
                DEFAULT_MAX_RETRIES,
            )
            .await?;
            let text = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::InfraUnavailable {
                    message: "OpenAI response missing choices[0].message.content".to_string(),
                })?;
            candidate_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl EditorialLlmClient for AnthropicEditorialClient {
    fn provider_key(&self) -> &'static str {
        "anthropic"
    }

    fn model_key(&self) -> String {
        std::env::var("ANTHROPIC_SEO_MODEL").unwrap_or_else(|_| "claude-4.5-sonnet".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| DomainError::InfraUnavailable {
                    message: "ANTHROPIC_API_KEY is not set".to_string(),
                })?;
            let prompt = prompt_from_input(input)?;
            let body = json!({
                "model": self.model_key(),
                "max_tokens": 1800,
                "temperature": 0.2,
                "system": "You write concise expert travel-visa editorial prose. Never invent facts. Use only provided support snippets. Return JSON only.",
                "messages": [
                    {
                        "role": "user",
                        "content": prompt
                    }
                ]
            });
            let value = post_json_with_retries(
                ANTHROPIC_MESSAGES_URL,
                anthropic_headers(&api_key)?,
                &body,
                request_timeout(),
                DEFAULT_MAX_RETRIES,
            )
            .await?;
            let text = value
                .get("content")
                .and_then(Value::as_array)
                .and_then(|content| content.first())
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::InfraUnavailable {
                    message: "Anthropic response missing content[0].text".to_string(),
                })?;
            candidate_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl EditorialLlmClient for VertexGeminiEditorialClient {
    fn provider_key(&self) -> &'static str {
        "vertex_gemini"
    }

    fn model_key(&self) -> String {
        vertex_gemini_runtime::vertex_model("VERTEX_GEMINI_EDITORIAL_MODEL", "gemini-2.5-pro")
    }

    fn is_configured(&self) -> bool {
        vertex_gemini_runtime::vertex_provider_configured()
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            let prompt = prompt_from_input(input)?;
            let body = json!({
                "generationConfig": {
                    "temperature": 0.2,
                    "responseMimeType": "application/json"
                },
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "text": format!(
                                    "You write concise expert travel-visa editorial prose. Never invent facts. Use only provided support snippets. Return JSON only.\n\n{}",
                                    prompt
                                )
                            }
                        ]
                    }
                ]
            });
            let value = post_json_with_retries(
                &vertex_gemini_runtime::vertex_generate_url(&self.model_key()),
                vertex_gemini_runtime::vertex_bearer_headers(request_timeout()).await?,
                &body,
                request_timeout(),
                DEFAULT_MAX_RETRIES,
            )
            .await?;
            let text = value
                .get("candidates")
                .and_then(Value::as_array)
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::InfraUnavailable {
                    message: "Vertex Gemini response missing candidates[0].content.parts[0].text"
                        .to_string(),
                })?;
            candidate_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl EditorialLlmClient for GeminiEditorialClient {
    fn provider_key(&self) -> &'static str {
        "gemini"
    }

    fn model_key(&self) -> String {
        std::env::var("GEMINI_SEO_MODEL").unwrap_or_else(|_| "gemini-2.5-pro".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("GEMINI_API_KEY").is_ok() || std::env::var("GOOGLE_API_KEY").is_ok()
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key = std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .map_err(|_| DomainError::InfraUnavailable {
                    message: "GEMINI_API_KEY or GOOGLE_API_KEY is not set".to_string(),
                })?;
            let prompt = prompt_from_input(input)?;
            let body = json!({
                "generationConfig": {
                    "temperature": 0.2,
                    "responseMimeType": "application/json"
                },
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "text": format!(
                                    "You write concise expert travel-visa editorial prose. Never invent facts. Use only provided support snippets. Return JSON only.\n\n{}",
                                    prompt
                                )
                            }
                        ]
                    }
                ]
            });
            let url = format!(
                "{}{}:generateContent?key={}",
                GEMINI_GENERATE_URL_PREFIX,
                self.model_key(),
                api_key
            );
            let value = post_json_with_retries(
                &url,
                basic_json_headers()?,
                &body,
                request_timeout(),
                DEFAULT_MAX_RETRIES,
            )
            .await?;
            let text = value
                .get("candidates")
                .and_then(Value::as_array)
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::InfraUnavailable {
                    message: "Gemini response missing candidates[0].content.parts[0].text"
                        .to_string(),
                })?;
            candidate_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl EditorialLlmClient for LocalCompatibleEditorialClient {
    fn provider_key(&self) -> &'static str {
        "local_compatible"
    }

    fn model_key(&self) -> String {
        std::env::var("SEO_LLM_LOCAL_MODEL").unwrap_or_else(|_| "local-editorial-model".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("SEO_LLM_LOCAL_ENDPOINT").is_ok()
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            let endpoint = std::env::var("SEO_LLM_LOCAL_ENDPOINT").map_err(|_| {
                DomainError::InfraUnavailable {
                    message: "SEO_LLM_LOCAL_ENDPOINT is not set".to_string(),
                }
            })?;
            let prompt = prompt_from_input(input)?;
            let body = json!({
                "model": self.model_key(),
                "temperature": 0.2,
                "response_format": { "type": "json_object" },
                "messages": [
                    {
                        "role": "system",
                        "content": "You write concise expert travel-visa editorial prose. Never invent facts. Use only provided support snippets. Return JSON only."
                    },
                    {
                        "role": "user",
                        "content": prompt
                    }
                ]
            });
            let mut headers = basic_json_headers()?;
            if let Ok(api_key) = std::env::var("SEO_LLM_LOCAL_API_KEY") {
                headers.insert(
                    AUTHORIZATION,
                    format!("Bearer {api_key}").parse().map_err(|e| {
                        DomainError::InfraUnavailable {
                            message: format!("invalid local LLM auth header: {e}"),
                        }
                    })?,
                );
            }
            let value = post_json_with_retries(
                &endpoint,
                headers,
                &body,
                request_timeout(),
                DEFAULT_MAX_RETRIES,
            )
            .await?;
            let text = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::InfraUnavailable {
                    message: "local-compatible response missing choices[0].message.content"
                        .to_string(),
                })?;
            candidate_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl EditorialLlmClient for DeterministicEditorialClient {
    fn provider_key(&self) -> &'static str {
        "deterministic_fixture"
    }

    fn model_key(&self) -> String {
        "source_backed_template@1".to_string()
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a> {
        Box::pin(async move {
            build_deterministic_candidate(input, self.provider_key(), &self.model_key())
        })
    }
}

fn ordered_clients() -> Vec<Box<dyn EditorialLlmClient + Send + Sync>> {
    vec![
        Box::new(VertexGeminiEditorialClient),
        Box::new(OpenAiEditorialClient),
        Box::new(AnthropicEditorialClient),
        Box::new(GeminiEditorialClient),
        Box::new(LocalCompatibleEditorialClient),
        Box::new(DeterministicEditorialClient),
    ]
}
