use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::vertex_gemini_runtime;
use primitives::errors::DomainError;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

type ClientFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TruthExtractionResponse, DomainError>> + Send + 'a>>;

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const GEMINI_GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models/";
const DEFAULT_TIMEOUT_SECS: u64 = 45;
const DEFAULT_MAX_RETRIES: usize = 2;
pub const EXTRACTION_PROMPT_VERSION: &str = "truth_extraction_prompt@1";

#[derive(Debug, Clone)]
pub struct TruthExtractionInput {
    pub context_key: String,
    pub raw_section_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub raw_text: String,
    pub source_snapshot_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthRuleCandidate {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub concept_canonical_key: String,
    #[serde(default)]
    pub raw_mention: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub scope: Value,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub applies_to_profiles: Vec<String>,
    #[serde(default)]
    pub exceptions_raw: Option<String>,
    #[serde(default)]
    pub conditions_raw: Option<String>,
    #[serde(default)]
    pub alternatives: Value,
    #[serde(default)]
    pub modality_raw: Option<String>,
    #[serde(default)]
    pub derivation_type: String,
    #[serde(default)]
    pub is_numeric: bool,
    #[serde(default)]
    pub is_range: bool,
    #[serde(default)]
    pub is_incomplete: bool,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence_section_id: Option<i64>,
    #[serde(default)]
    pub evidence_quote: String,
    #[serde(default)]
    pub span_start: usize,
    #[serde(default)]
    pub span_end: usize,
    #[serde(default)]
    pub uncertainty_flags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TruthExtractionResponse {
    pub provider_key: String,
    pub model_key: String,
    pub prompt_version: String,
    pub candidates: Vec<TruthRuleCandidate>,
}

trait TruthExtractionClient {
    fn provider_key(&self) -> &'static str;
    fn model_key(&self) -> String;
    fn is_configured(&self) -> bool;
    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a>;
}

struct OpenAiTruthExtractionClient;
struct AnthropicTruthExtractionClient;
struct VertexGeminiTruthExtractionClient;
struct GeminiTruthExtractionClient;
struct LocalCompatibleTruthExtractionClient;

impl TruthExtractionClient for OpenAiTruthExtractionClient {
    fn provider_key(&self) -> &'static str {
        "openai"
    }

    fn model_key(&self) -> String {
        std::env::var("OPENAI_TRUTH_MODEL")
            .or_else(|_| std::env::var("OPENAI_SEO_MODEL"))
            .unwrap_or_else(|_| "gpt-5.2".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("OPENAI_API_KEY").is_ok()
    }

    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key =
                std::env::var("OPENAI_API_KEY").map_err(|_| DomainError::InfraUnavailable {
                    message: "OPENAI_API_KEY is not set".to_string(),
                })?;
            let prompt = extraction_prompt(input);
            let body = json!({
                "model": self.model_key(),
                "temperature": 0.0,
                "response_format": { "type": "json_object" },
                "messages": [
                    {
                        "role": "system",
                        "content": extraction_system_prompt()
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
                .unwrap_or("{}");
            response_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl TruthExtractionClient for AnthropicTruthExtractionClient {
    fn provider_key(&self) -> &'static str {
        "anthropic"
    }

    fn model_key(&self) -> String {
        std::env::var("ANTHROPIC_TRUTH_MODEL")
            .or_else(|_| std::env::var("ANTHROPIC_SEO_MODEL"))
            .unwrap_or_else(|_| "claude-4.5-sonnet".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").map_err(|_| DomainError::InfraUnavailable {
                    message: "ANTHROPIC_API_KEY is not set".to_string(),
                })?;
            let body = json!({
                "model": self.model_key(),
                "max_tokens": 2000,
                "temperature": 0.0,
                "system": extraction_system_prompt(),
                "messages": [
                    {
                        "role": "user",
                        "content": extraction_prompt(input)
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
                .unwrap_or("{}");
            response_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl TruthExtractionClient for VertexGeminiTruthExtractionClient {
    fn provider_key(&self) -> &'static str {
        "vertex_gemini"
    }

    fn model_key(&self) -> String {
        vertex_gemini_runtime::vertex_model("VERTEX_GEMINI_TRUTH_MODEL", "gemini-2.5-pro")
    }

    fn is_configured(&self) -> bool {
        vertex_gemini_runtime::vertex_provider_configured()
    }

    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a> {
        Box::pin(async move {
            let mut body_map = serde_json::Map::new();
            body_map.insert(
                "generationConfig".to_string(),
                json!({
                    "temperature": 0.0,
                    "responseMimeType": "application/json"
                }),
            );
            body_map.insert(
                "contents".to_string(),
                json!([
                    {
                        "role": "user",
                        "parts": [
                            {
                                "text": format!("{}\n\n{}", extraction_system_prompt(), extraction_prompt(input))
                            }
                        ]
                    }
                ]),
            );
            if let Some(datastore_path) = vertex_gemini_runtime::vertex_datastore_resource() {
                body_map.insert(
                    "tools".to_string(),
                    json!([
                        {
                            "retrieval": {
                                "vertexAiSearch": {
                                    "datastore": datastore_path
                                }
                            }
                        }
                    ]),
                );
            }
            let body = Value::Object(body_map);
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
                .unwrap_or("{}");
            response_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl TruthExtractionClient for GeminiTruthExtractionClient {
    fn provider_key(&self) -> &'static str {
        "gemini"
    }

    fn model_key(&self) -> String {
        std::env::var("GEMINI_TRUTH_MODEL")
            .or_else(|_| std::env::var("GEMINI_SEO_MODEL"))
            .unwrap_or_else(|_| "gemini-2.5-pro".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("GEMINI_API_KEY").is_ok() || std::env::var("GOOGLE_API_KEY").is_ok()
    }

    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a> {
        Box::pin(async move {
            let api_key = std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .map_err(|_| DomainError::InfraUnavailable {
                    message: "GEMINI_API_KEY or GOOGLE_API_KEY is not set".to_string(),
                })?;
            let body = json!({
                "generationConfig": {
                    "temperature": 0.0,
                    "responseMimeType": "application/json"
                },
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "text": format!("{}\n\n{}", extraction_system_prompt(), extraction_prompt(input))
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
                .unwrap_or("{}");
            response_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

impl TruthExtractionClient for LocalCompatibleTruthExtractionClient {
    fn provider_key(&self) -> &'static str {
        "local_compatible"
    }

    fn model_key(&self) -> String {
        std::env::var("SEO_TRUTH_LLM_LOCAL_MODEL")
            .or_else(|_| std::env::var("SEO_LLM_LOCAL_MODEL"))
            .unwrap_or_else(|_| "local-truth-extraction-model".to_string())
    }

    fn is_configured(&self) -> bool {
        std::env::var("SEO_TRUTH_LLM_LOCAL_ENDPOINT").is_ok()
            || std::env::var("SEO_LLM_LOCAL_ENDPOINT").is_ok()
    }

    fn extract<'a>(&'a self, input: &'a TruthExtractionInput) -> ClientFuture<'a> {
        Box::pin(async move {
            let endpoint = std::env::var("SEO_TRUTH_LLM_LOCAL_ENDPOINT")
                .or_else(|_| std::env::var("SEO_LLM_LOCAL_ENDPOINT"))
                .map_err(|_| DomainError::InfraUnavailable {
                    message: "SEO_TRUTH_LLM_LOCAL_ENDPOINT or SEO_LLM_LOCAL_ENDPOINT is not set"
                        .to_string(),
                })?;
            let body = json!({
                "model": self.model_key(),
                "temperature": 0.0,
                "response_format": { "type": "json_object" },
                "messages": [
                    {
                        "role": "system",
                        "content": extraction_system_prompt()
                    },
                    {
                        "role": "user",
                        "content": extraction_prompt(input)
                    }
                ]
            });
            let mut headers = basic_json_headers()?;
            if let Ok(api_key) = std::env::var("SEO_TRUTH_LLM_LOCAL_API_KEY")
                .or_else(|_| std::env::var("SEO_LLM_LOCAL_API_KEY"))
            {
                headers.insert(
                    AUTHORIZATION,
                    format!("Bearer {api_key}").parse().map_err(|e| {
                        DomainError::InfraUnavailable {
                            message: format!("invalid truth local auth header: {e}"),
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
                .unwrap_or("{}");
            response_from_generated_text(input, self.provider_key(), &self.model_key(), text)
        })
    }
}

pub fn extraction_provider_available() -> bool {
    provider_clients()
        .iter()
        .any(|client| client.is_configured())
}

pub fn configured_truth_provider_summary() -> Vec<(String, String)> {
    provider_clients()
        .into_iter()
        .filter(|client| client.is_configured())
        .map(|client| (client.provider_key().to_string(), client.model_key()))
        .collect()
}

pub async fn extract_rule_candidates(
    input: &TruthExtractionInput,
) -> Result<Option<TruthExtractionResponse>, DomainError> {
    for client in provider_clients() {
        if !client.is_configured() {
            continue;
        }
        let response = client.extract(input).await?;
        return Ok(Some(response));
    }
    Ok(None)
}

fn provider_clients() -> Vec<Box<dyn TruthExtractionClient + Send + Sync>> {
    vec![
        Box::new(LocalCompatibleTruthExtractionClient),
        Box::new(VertexGeminiTruthExtractionClient),
        Box::new(OpenAiTruthExtractionClient),
        Box::new(AnthropicTruthExtractionClient),
        Box::new(GeminiTruthExtractionClient),
    ]
}

fn extraction_system_prompt() -> &'static str {
    "You are an evidence-grade visa procedural extractor. Return JSON only. Do not invent facts. Extract only explicit procedural claims from the provided source section. Every claim must include exact evidence_quote plus span_start/span_end in the provided raw_text. If unsure, omit the claim."
}

fn extraction_prompt(input: &TruthExtractionInput) -> String {
    format!(
        "Return a JSON object with top-level key `candidates`.\nEach candidate must include: role, concept_canonical_key, raw_mention, params, scope, severity, applies_to_profiles, exceptions_raw, conditions_raw, alternatives, modality_raw, derivation_type, is_numeric, is_range, is_incomplete, confidence, evidence_section_id, evidence_quote, span_start, span_end, uncertainty_flags.\nUse only these roles: DOCUMENT_REQUIRED, ELIGIBILITY_RULE, FEE_ITEM, TIMELINE_ITEM, WHERE_TO_APPLY, APPOINTMENT_RULE, FORM_REQUIRED, STEP.\nUse exact evidence spans from raw_text. If a field is unknown, use null or empty array/object as appropriate. Do not output prose.\n\ncontext_key: {}\nraw_section_id: {}\nsource_url: {}\nsource_domain: {}\nheading_path: {}\nraw_text:\n{}",
        input.context_key,
        input.raw_section_id,
        input.source_url,
        input.source_domain,
        input.heading_path,
        input.raw_text
    )
}

fn response_from_generated_text(
    input: &TruthExtractionInput,
    provider_key: &str,
    model_key: &str,
    text: &str,
) -> Result<TruthExtractionResponse, DomainError> {
    let value = serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({}));
    let raw_candidates = value
        .get("candidates")
        .or_else(|| value.get("claims"))
        .or_else(|| value.get("extracted_rules"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut candidates = Vec::new();
    for raw_candidate in raw_candidates {
        if let Ok(candidate) = serde_json::from_value::<TruthRuleCandidate>(raw_candidate) {
            if let Some(validated) = validate_candidate(input, candidate) {
                candidates.push(validated);
            }
        }
    }
    Ok(TruthExtractionResponse {
        provider_key: provider_key.to_string(),
        model_key: model_key.to_string(),
        prompt_version: EXTRACTION_PROMPT_VERSION.to_string(),
        candidates,
    })
}

fn validate_candidate(
    input: &TruthExtractionInput,
    mut candidate: TruthRuleCandidate,
) -> Option<TruthRuleCandidate> {
    candidate.role = normalize_role(&candidate.role)?;
    if candidate.concept_canonical_key.trim().is_empty() {
        return None;
    }
    if candidate.evidence_quote.trim().is_empty() {
        return None;
    }
    if candidate.evidence_section_id.unwrap_or_default() != input.raw_section_id {
        candidate.evidence_section_id = Some(input.raw_section_id);
    }
    if candidate.span_start >= candidate.span_end {
        return None;
    }
    if candidate.span_end > input.raw_text.len() {
        return None;
    }
    if !input.raw_text.is_char_boundary(candidate.span_start)
        || !input.raw_text.is_char_boundary(candidate.span_end)
    {
        return None;
    }
    let snippet = input
        .raw_text
        .get(candidate.span_start..candidate.span_end)
        .unwrap_or_default()
        .trim()
        .to_string();
    if snippet.is_empty() {
        return None;
    }
    if candidate.raw_mention.trim().is_empty() {
        candidate.raw_mention = snippet.clone();
    }
    if candidate.evidence_quote.trim() != snippet {
        candidate.evidence_quote = snippet;
    }
    if !candidate.confidence.is_finite()
        || candidate.confidence <= 0.0
        || candidate.confidence > 1.0
    {
        return None;
    }
    if !matches!(candidate.params, Value::Object(_)) {
        candidate.params = json!({});
    }
    if !matches!(candidate.scope, Value::Object(_)) {
        candidate.scope = json!({});
    }
    if !matches!(candidate.alternatives, Value::Array(_)) {
        candidate.alternatives = json!([]);
    }
    if candidate.severity.trim().is_empty() {
        candidate.severity = "unknown".to_string();
    }
    if candidate.derivation_type.trim().is_empty() {
        candidate.derivation_type = "direct".to_string();
    }
    Some(candidate)
}

fn normalize_role(role: &str) -> Option<String> {
    match role.trim().to_ascii_uppercase().as_str() {
        "DOCUMENT_REQUIRED" => Some("DOCUMENT_REQUIRED".to_string()),
        "ELIGIBILITY_RULE" => Some("ELIGIBILITY_RULE".to_string()),
        "FEE_ITEM" => Some("FEE_ITEM".to_string()),
        "TIMELINE_ITEM" => Some("TIMELINE_ITEM".to_string()),
        "WHERE_TO_APPLY" => Some("WHERE_TO_APPLY".to_string()),
        "APPOINTMENT_RULE" => Some("APPOINTMENT_RULE".to_string()),
        "FORM_REQUIRED" => Some("FORM_REQUIRED".to_string()),
        "STEP" => Some("STEP".to_string()),
        _ => None,
    }
}

fn request_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("SEO_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS),
    )
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
            message: format!("invalid Anthropic auth header: {e}"),
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
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("build truth extraction HTTP client: {err}"),
        })?;
    let mut last_error = None;
    for _ in 0..=max_retries {
        let response = client
            .post(url)
            .headers(headers.clone())
            .json(body)
            .send()
            .await;
        match response {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    let value = resp.json::<Value>().await.map_err(|err| {
                        DomainError::InfraUnavailable {
                            message: format!("decode truth extraction response JSON: {err:?}"),
                        }
                    })?;
                    return Ok(value);
                }
                let body = resp.text().await.unwrap_or_else(|err| {
                    format!("<failed to read truth extraction error body: {err:?}>")
                });
                last_error = Some(format!(
                    "truth extraction HTTP {} from {} body={}",
                    status, url, body
                ));
            }
            Err(err) => {
                last_error = Some(format!(
                    "truth extraction request error from {url}: {err:?}"
                ));
            }
        }
    }
    Err(DomainError::InfraUnavailable {
        message: last_error.unwrap_or_else(|| "truth extraction HTTP request failed".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> TruthExtractionInput {
        TruthExtractionInput {
            context_key: "es:tourist:by".to_string(),
            raw_section_id: 42,
            source_url: "https://example.gov/visa/fees".to_string(),
            source_domain: "example.gov".to_string(),
            heading_path: "Fees".to_string(),
            raw_text: "Consular fee is 35 EUR".to_string(),
            source_snapshot_hash: "hash-1".to_string(),
        }
    }

    #[test]
    fn malformed_candidate_is_dropped_before_persistence() {
        let input = sample_input();
        let text = serde_json::json!({
            "candidates": [{
                "role": "FEE_ITEM",
                "concept_canonical_key": "consular_fee",
                "raw_mention": "Consular fee is 35 EUR",
                "params": { "amount": 35, "currency": "EUR" },
                "scope": {},
                "severity": "mandatory",
                "applies_to_profiles": [],
                "alternatives": [],
                "derivation_type": "direct",
                "is_numeric": true,
                "is_range": false,
                "is_incomplete": false,
                "confidence": 0.93,
                "evidence_section_id": 42,
                "evidence_quote": "Consular fee is 35 EUR",
                "span_start": 0,
                "span_end": 999,
                "uncertainty_flags": []
            }]
        })
        .to_string();

        let response =
            response_from_generated_text(&input, "local_compatible", "stub-truth-model", &text)
                .expect("response");
        assert!(response.candidates.is_empty());
    }
}
