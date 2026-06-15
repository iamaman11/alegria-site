use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::vertex_gemini_runtime;
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
pub struct VertexGeminiEditorialClient;
pub struct GeminiEditorialClient;
pub struct LocalCompatibleEditorialClient;
pub struct DeterministicEditorialClient;

#[derive(Debug, Clone, Default)]
struct GeneratedSections {
    sections: BTreeMap<String, String>,
}
