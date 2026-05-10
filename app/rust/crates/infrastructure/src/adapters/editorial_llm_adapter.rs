use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use contracts::generated::alegria::temporal::v1::{
    ClaimLedgerEntry, EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    LlmDraftCandidate, LlmDraftRequest, RenderedContentBlock, SeoDraftSectionState,
};
use primitives::errors::DomainError;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use runtime_models::seo_blocks::{
    factual_role, planned_content_blocks, support_traceability_label, supports_for_role,
};
use serde_json::{json, Value};

type ClientFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LlmDraftCandidate, DomainError>> + Send + 'a>>;

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const GEMINI_GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models/";
const DEFAULT_TIMEOUT_SECS: u64 = 45;
const DEFAULT_MAX_RETRIES: usize = 2;
const DETERMINISTIC_PROMPT_VERSION: &str = "editorial_prompt@1";

pub trait EditorialLlmClient {
    fn provider_key(&self) -> &'static str;
    fn model_key(&self) -> String;
    fn is_configured(&self) -> bool;
    fn generate<'a>(&'a self, input: &'a EditorialDraftGenerateInputPayload) -> ClientFuture<'a>;
}

pub struct OpenAiEditorialClient;
pub struct AnthropicEditorialClient;
pub struct GeminiEditorialClient;
pub struct LocalCompatibleEditorialClient;
pub struct DeterministicEditorialClient;

#[derive(Debug, Clone, Default)]
struct GeneratedSections {
    sections: BTreeMap<String, String>,
}

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

impl EditorialLlmClient for GeminiEditorialClient {
    fn provider_key(&self) -> &'static str {
        "gemini"
    }

    fn model_key(&self) -> String {
        std::env::var("GEMINI_SEO_MODEL").unwrap_or_else(|_| "gemini-3-pro".to_string())
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
        Box::new(OpenAiEditorialClient),
        Box::new(AnthropicEditorialClient),
        Box::new(GeminiEditorialClient),
        Box::new(LocalCompatibleEditorialClient),
        Box::new(DeterministicEditorialClient),
    ]
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
    let roles = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    )
    .into_iter()
    .filter(|block| !block.deterministic_only || block.section_role == "cta_disclaimer")
    .map(|block| {
        let supports = supports_for_role(&brief.verified_support, &block.section_role);
        let support_lines = supports
            .iter()
            .take(8)
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

    Ok(format!(
        "Return a JSON object with one top-level key `sections` mapping section_role -> markdown body.\n\
Do not include any prose outside JSON. Do not invent facts. Paraphrase only the supplied support lines.\n\
Supplemental source context is for topic coverage and structure only; do not treat it as factual proof.\n\
Do not mention support refs explicitly in the final text. Keep each section concise and expert.\n\
Page node: {}\nTarget audience: {}\nExpertise goal: {}\nPrompt version: {}\n\n{}",
        brief.page_node_key,
        brief.target_audience,
        brief.expertise_goal,
        request.prompt_version,
        format!(
            "{}\nSUPPLEMENTAL_SOURCE_CONTEXT:\n{}",
            roles,
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
                last_error = error.to_string();
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

fn build_deterministic_candidate(
    input: &EditorialDraftGenerateInputPayload,
    provider_key: &str,
    model_key: &str,
) -> Result<LlmDraftCandidate, DomainError> {
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
    let mut markdown = String::new();
    let mut sections = Vec::new();
    let mut claim_ledger = Vec::new();
    let mut content_blocks = Vec::new();
    let page_node_key = &brief.page_node_key;
    markdown.push_str("# Expert travel visa guide\n\n");
    let block_plan = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    );
    for block in &block_plan {
        let supports = supports_for_role(&brief.verified_support, &block.section_role);
        let heading = block.heading.clone();
        let mut body = String::new();
        if block.section_role == "related_pages" {
            for link in &brief.required_links {
                body.push_str("- ");
                body.push_str(&link.link_role);
                body.push_str(" -> ");
                body.push_str(&link.target_page_key);
                body.push('\n');
            }
        } else if block.section_role == "cta_disclaimer" {
            body.push_str("This draft is ready for human editorial review. Confirm current official requirements before publication.\n");
        } else {
            body.push_str(if block.deterministic_only {
                "Verified source-backed structured content:\n"
            } else {
                "Based on verified source-backed evidence:\n"
            });
            for support in supports {
                body.push_str("- ");
                body.push_str(support.fragment_text.trim());
                body.push('\n');
                let claim_key = primitives::seo::seo_artifact_key(
                    "claim",
                    &[page_node_key, &block.section_role, &support.support_ref],
                );
                claim_ledger.push(ClaimLedgerEntry {
                    claim_key,
                    fragment_text: support.fragment_text.clone(),
                    claim_kind: if factual_role(&block.section_role) {
                        "factual".to_string()
                    } else {
                        "editorial".to_string()
                    },
                    traceability_label: support_traceability_label(&block.section_role).to_string(),
                    support_refs: vec![support.support_ref.clone()],
                    validation_verdict: "supported".to_string(),
                    source_section_key: block.section_role.clone(),
                });
            }
        }
        let section_key = primitives::seo::seo_artifact_key(
            "draft_section",
            &[page_node_key, &block.section_role],
        );
        markdown.push_str("## ");
        markdown.push_str(&heading);
        markdown.push_str("\n\n");
        markdown.push_str(&body);
        markdown.push('\n');
        sections.push(SeoDraftSectionState {
            section_key: section_key.clone(),
            section_role: block.section_role.clone(),
            heading: heading.clone(),
            body_markdown: body.clone(),
            required: block.required,
            traceability_label: support_traceability_label(&block.section_role).to_string(),
            support_refs: claim_ledger
                .iter()
                .filter(|claim| claim.source_section_key == block.section_role)
                .flat_map(|claim| claim.support_refs.clone())
                .collect(),
            template_key: brief
                .section_templates
                .iter()
                .find(|template| template.section_role == block.section_role)
                .map(|template| template.template_key.clone())
                .unwrap_or_default(),
        });
        content_blocks.push(RenderedContentBlock {
            block_key: primitives::seo::seo_artifact_key(
                "content_block",
                &[page_node_key, &block.section_role, "llm"],
            ),
            block_type: block.block_type.clone(),
            section_role: block.section_role.clone(),
            heading,
            markdown: body,
            support_refs: claim_ledger
                .iter()
                .filter(|claim| claim.source_section_key == block.section_role)
                .flat_map(|claim| claim.support_refs.clone())
                .collect(),
            traceability_label: support_traceability_label(&block.section_role).to_string(),
            required: block.required,
        });
    }
    let faq_items = brief
        .verified_support
        .iter()
        .take(6)
        .map(|support| {
            serde_json::json!({
                "@type": "Question",
                "name": "What does the verified source say?",
                "acceptedAnswer": {
                    "@type": "Answer",
                    "text": support.fragment_text,
                }
            })
        })
        .collect::<Vec<_>>();
    let faq_json = serde_json::to_string(&faq_items).unwrap_or_else(|_| "[]".to_string());
    let schema_markup_json = serde_json::to_string(&serde_json::json!({
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "Article",
                "headline": "Expert travel visa guide",
            },
            {
                "@type": "FAQPage",
                "mainEntity": faq_items,
            }
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string());
    Ok(LlmDraftCandidate {
        candidate_key: primitives::seo::seo_artifact_key(
            "llm_draft_candidate",
            &[&request.request_key, provider_key, model_key],
        ),
        request_key: request.request_key.clone(),
        provider_key: provider_key.to_string(),
        model_key: model_key.to_string(),
        prompt_version: if request.prompt_version.trim().is_empty() {
            DETERMINISTIC_PROMPT_VERSION.to_string()
        } else {
            request.prompt_version.clone()
        },
        body_markdown: markdown,
        sections,
        claim_ledger,
        content_blocks,
        faq_json,
        schema_markup_json,
        status: "ready".to_string(),
    })
}

fn overlay_generated_sections(
    mut candidate: LlmDraftCandidate,
    request: &LlmDraftRequest,
    generated: GeneratedSections,
) -> LlmDraftCandidate {
    let Some(brief) = request.editorial_brief.as_ref() else {
        return candidate;
    };
    let block_plan = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    );
    let mut body_parts = vec!["# Expert travel visa guide".to_string(), String::new()];
    for section in &mut candidate.sections {
        let Some(block) = block_plan
            .iter()
            .find(|block| block.section_role == section.section_role)
        else {
            continue;
        };
        if block.deterministic_only && section.section_role != "cta_disclaimer" {
            body_parts.push(format!(
                "## {}\n\n{}\n",
                section.heading, section.body_markdown
            ));
            continue;
        }
        if let Some(generated_markdown) = generated.sections.get(&section.section_role) {
            if !generated_markdown.trim().is_empty() {
                section.body_markdown = generated_markdown.trim().to_string();
            }
        }
        body_parts.push(format!(
            "## {}\n\n{}\n",
            section.heading, section.body_markdown
        ));
    }
    for block in &mut candidate.content_blocks {
        if let Some(generated_markdown) = generated.sections.get(&block.section_role) {
            let deterministic_only = block_plan
                .iter()
                .find(|plan| plan.section_role == block.section_role)
                .map(|plan| plan.deterministic_only)
                .unwrap_or(false);
            if (!deterministic_only || block.section_role == "cta_disclaimer")
                && !generated_markdown.trim().is_empty()
            {
                block.markdown = generated_markdown.trim().to_string();
            }
        }
    }
    candidate.body_markdown = body_parts.join("\n");
    candidate
}

fn candidate_from_generated_text(
    input: &EditorialDraftGenerateInputPayload,
    provider_key: &str,
    model_key: &str,
    text: &str,
) -> Result<LlmDraftCandidate, DomainError> {
    let request = input
        .request
        .as_ref()
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "editorial draft generation requires LlmDraftRequest".to_string(),
        })?;
    let deterministic = build_deterministic_candidate(input, provider_key, model_key)?;
    let generated = parse_generated_sections(text)?;
    Ok(overlay_generated_sections(
        deterministic,
        request,
        generated,
    ))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        EditorialBrief, LinkRecommendationState, LlmDraftRequest, SectionTemplateBinding,
        SeoVerifiedFactSupportState,
    };

    fn sample_input() -> EditorialDraftGenerateInputPayload {
        EditorialDraftGenerateInputPayload {
            run_id: "run-1".to_string(),
            request: Some(LlmDraftRequest {
                request_key: "req-1".to_string(),
                run_id: "run-1".to_string(),
                editorial_brief: Some(EditorialBrief {
                    editorial_brief_key: "brief-1".to_string(),
                    page_node_key: "page-1".to_string(),
                    page_brief_key: "page-brief-1".to_string(),
                    target_audience: "tourist".to_string(),
                    expertise_goal: "help applicant understand the process".to_string(),
                    section_templates: vec![SectionTemplateBinding {
                        template_key: "overview".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "informational".to_string(),
                        section_role: "overview".to_string(),
                        heading: "Overview".to_string(),
                        template_body: "overview".to_string(),
                        template_version: 1,
                        required: true,
                    }],
                    verified_support: vec![SeoVerifiedFactSupportState {
                        support_ref: "rule-1".to_string(),
                        role_type: "document_required".to_string(),
                        fragment_text: "Passport is required.".to_string(),
                        source_label: "Official source".to_string(),
                        source_tier: "official".to_string(),
                        freshness_class: "fresh".to_string(),
                        observed_at: "2026-05-08T00:00:00Z".to_string(),
                        valid_until: "2027-05-08T00:00:00Z".to_string(),
                    }],
                    required_links: vec![LinkRecommendationState::default()],
                    truth_snapshot_ref: "truth-1".to_string(),
                    source_context_chunks: Vec::new(),
                }),
                provider_policy: "multi_provider:first_available".to_string(),
                prompt_version: "editorial_prompt@1".to_string(),
                output_contract: "headless".to_string(),
            }),
            provider_policy: String::new(),
        }
    }

    #[test]
    fn parses_generated_sections_json() {
        let sections = parse_generated_sections(
            r#"{"sections":{"overview":"Expert overview.","cta_disclaimer":"Confirm official requirements."}}"#,
        )
        .expect("parse sections");
        assert_eq!(
            sections.sections.get("overview").map(String::as_str),
            Some("Expert overview.")
        );
    }

    #[tokio::test]
    async fn deterministic_fallback_is_used_when_no_provider_is_configured() {
        let input = sample_input();
        let output = generate_editorial_draft(&input)
            .await
            .expect("fallback output");
        assert_eq!(output.provider_key, "deterministic_fixture");
        assert!(output.candidate.is_some());
    }
}
