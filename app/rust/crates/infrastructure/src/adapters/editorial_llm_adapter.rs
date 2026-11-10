include!("editorial_llm_adapter/preamble.rs");
include!("editorial_llm_adapter/provider_clients.rs");
include!("editorial_llm_adapter/prompt_http.rs");
include!("editorial_llm_adapter/generated_parse.rs");
include!("editorial_llm_adapter/deterministic_candidate.rs");
include!("editorial_llm_adapter/generated_overlay.rs");
include!("editorial_llm_adapter/provider_selection.rs");
include!("editorial_llm_adapter/tests.rs");

pub fn manual_candidate_from_generated_text(
    input: &contracts::generated::alegria::temporal::v1::EditorialDraftGenerateInputPayload,
    model_key: &str,
    response_json: &str,
) -> Result<
    contracts::generated::alegria::temporal::v1::LlmDraftCandidate,
    primitives::errors::DomainError,
> {
    candidate_from_generated_text(input, "chat_manual", model_key, response_json)
}
