pub fn editorial_allowed_support(
    brief: &contracts::generated::alegria::temporal::v1::EditorialBrief,
) -> BTreeMap<String, Vec<Value>> {
    planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    )
    .into_iter()
    .filter(|block| factual_role(&block.section_role))
    .map(|block| {
        let entries = supports_for_role(&brief.verified_support, &block.section_role)
            .into_iter()
            .map(|s| json!({"support_ref":s.support_ref,"fragment_text":s.fragment_text}))
            .collect::<Vec<_>>();
        (block.section_role, entries)
    })
    .collect()
}

fn prompt_from_input(input: &EditorialDraftGenerateInputPayload) -> Result<String, DomainError> {
    let Some(request) = input.request.as_ref() else {
        return Err(DomainError::ValidationFailure {
            message: "editorial draft generation requires LlmDraftRequest".to_string(),
        });
    };
    let Some(brief) = request.editorial_brief.as_ref() else {
        return Err(DomainError::ValidationFailure {
            message: "editorial draft generation requires EditorialBrief".to_string(),
        });
    };
    let typed = request.output_contract == "llm_draft_candidate@2";
    let roles = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    )
    .into_iter()
    .filter(|block| {
        if typed {
            factual_role(&block.section_role)
        } else {
            !block.deterministic_only || block.section_role == "cta_disclaimer"
        }
    })
    .map(|block| {
        let supports = supports_for_role(&brief.verified_support, &block.section_role);
        let support_lines = supports
            .iter()
            .take(if typed { usize::MAX } else { 8 })
            .map(|support| {
                format!(
                    "- [{}] {}",
                    support.support_ref,
                    support.fragment_text.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "SECTION_ROLE: {}\nHEADING: {}\nALLOWED_SUPPORT:\n{}\n",
            block.section_role,
            block.heading,
            if support_lines.is_empty() {
                "- no direct support lines available; keep this section procedural and cautious"
            } else {
                &support_lines
            }
        )
    })
    .collect::<Vec<_>>()
    .join("\n");
    let source_context =
        brief
            .source_context_chunks
            .iter()
            .take(12)
            .map(|chunk| {
                format!(
                "- SOURCE_CONTEXT [{} | {} | score={} | policy={}]\n  Heading: {}\n  Excerpt: {}",
                chunk.chunk_key,
                chunk.source_domain,
                chunk.retrieval_score,
                chunk.usage_policy,
                chunk.heading_path,
                chunk.content_md.trim().chars().take(900).collect::<String>()
            )
            })
            .collect::<Vec<_>>()
            .join("\n");

    let instructions = if typed {
        "Return JSON exactly {claims: {section_role: [{support_ref, text, question?}]}, metadata: {title, description}} with quoted JSON keys. Use the requested locale. Each entry paraphrases only its exact allowed support, preserves all scope, conditions, exceptions, and ASCII numeric values. One entry per support ref; cover every support ref in each direct factual section. Overview and FAQ may select a subset. FAQ requires one question ending in '?'; no question in other sections. Text and questions must be plain text, without Markdown, HTML or URLs. Source links are resolved by the application. No invented facts, no external context as proof. Title 15-110 and description 45-190 characters. No prose outside JSON."
    } else {
        "Return a JSON object with two top-level keys: sections (role -> markdown) and metadata (title 15-110 chars, description 45-190 chars).\nDo not include any prose outside JSON. Do not invent facts. Paraphrase only the supplied support lines.\nSupplemental source context is for topic coverage and structure only; do not treat it as factual proof.\nDo not mention support refs explicitly in the final text. Keep each section concise and expert."
    };
    // One machine-readable representation of the same allowed evidence.
    // Strings are JSON escaped, so multiline evidence cannot change its role.
    let allowed = if typed {
        let map = editorial_allowed_support(brief);
        format!(
            "ALLOWED_CLAIMS_JSON: {}\n",
            serde_json::to_string(&map).map_err(|err| DomainError::ValidationFailure {
                message: format!("serialize editorial support: {err}")
            })?
        )
    } else {
        String::new()
    };
    Ok(format!(
        "{}\nPage node: {}\nEditorial locale: {}\nCanonical URL path: {}\nTarget audience: {}\nExpertise goal: {}\nPrompt version: {}\n{}\n{}",
        instructions,
        brief.page_node_key,
        brief.locale_code,
        brief.canonical_url_path,
        brief.target_audience,
        brief.expertise_goal,
        request.prompt_version,
        allowed,
        format!(
            "{}\nSUPPLEMENTAL_SOURCE_CONTEXT:\n{}",
            if typed { "" } else { &roles },
            if source_context.is_empty() {
                "- none"
            } else {
                &source_context
            }
        )
    ))
}

fn basic_json_headers() -> Result<reqwest::header::HeaderMap, DomainError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|e| DomainError::InfraUnavailable {
                message: format!("invalid content-type header: {e}"),
            })?,
    );
    Ok(headers)
}

fn openai_headers(api_key: &str) -> Result<reqwest::header::HeaderMap, DomainError> {
    let mut headers = basic_json_headers()?;
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {api_key}")
            .parse()
            .map_err(|e| DomainError::InfraUnavailable {
                message: format!("invalid OpenAI auth header: {e}"),
            })?,
    );
    Ok(headers)
}

fn anthropic_headers(api_key: &str) -> Result<reqwest::header::HeaderMap, DomainError> {
    let mut headers = basic_json_headers()?;
    headers.insert(
        "x-api-key",
        api_key.parse().map_err(|e| DomainError::InfraUnavailable {
            message: format!("invalid Anthropic api key header: {e}"),
        })?,
    );
    headers.insert(
        "anthropic-version",
        "2023-06-01"
            .parse()
            .map_err(|e| DomainError::InfraUnavailable {
                message: format!("invalid Anthropic version header: {e}"),
            })?,
    );
    Ok(headers)
}

fn request_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("SEO_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS),
    )
}

fn allow_fallback_on_error() -> bool {
    std::env::var("SEO_LLM_ALLOW_DETERMINISTIC_FALLBACK_ON_ERROR")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

async fn post_json_with_retries(
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: &Value,
    timeout: Duration,
    max_retries: usize,
) -> Result<Value, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| DomainError::InfraUnavailable {
            message: format!("llm http client build failed: {e}"),
        })?;
    let mut last_error = String::new();
    for attempt in 0..=max_retries {
        let response = client
            .post(url)
            .headers(headers.clone())
            .json(body)
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status();
                let text = response
                    .text()
                    .await
                    .map_err(|e| DomainError::InfraUnavailable {
                        message: format!("llm http read failed: {e}"),
                    })?;
                if !status.is_success() {
                    last_error = format!("status={} body={}", status, text);
                } else {
                    return serde_json::from_str::<Value>(&text).map_err(|e| {
                        DomainError::InfraUnavailable {
                            message: format!("llm response JSON decode failed: {e}; body={text}"),
                        }
                    });
                }
            }
            Err(error) => {
                last_error = format!("request error from {url}: {error:?}");
            }
        }
        if attempt < max_retries {
            tokio::time::sleep(Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
    }
    Err(DomainError::InfraUnavailable {
        message: format!("llm request failed after retries: {last_error}"),
    })
}
