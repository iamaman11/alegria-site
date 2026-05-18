use contracts::generated::alegria::temporal::v1::{
    ClaimLedgerEntry, LlmDraftCandidate, RenderedContentBlock, SeoDraftSectionState,
    SeoVerifiedFactSupportState, SourceContextChunkState,
};
use regex::Regex;
use std::sync::OnceLock;

fn email_re() -> &'static Regex {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    EMAIL_RE.get_or_init(|| {
        Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").expect("email regex")
    })
}

fn phone_re() -> &'static Regex {
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();
    PHONE_RE
        .get_or_init(|| Regex::new(r"(?x)(?i)\b(?:\+?\d[\d\s().-]{6,}\d)\b").expect("phone regex"))
}

fn passport_re() -> &'static Regex {
    static PASSPORT_RE: OnceLock<Regex> = OnceLock::new();
    PASSPORT_RE.get_or_init(|| {
        Regex::new(r"(?i)\bpassport(?:\s*(?:number|no\.?|#))?[:\s-]*[a-z0-9]{6,12}\b")
            .expect("passport regex")
    })
}

pub fn sanitize_text(text: &str) -> String {
    let mut output = text.to_string();
    for (regex, replacement) in [
        (passport_re(), "[redacted:passport]"),
        (email_re(), "[redacted:email]"),
        (phone_re(), "[redacted:phone]"),
    ] {
        output = regex.replace_all(&output, replacement).to_string();
    }
    output
}

pub fn sanitize_source_context_chunks(
    chunks: &[SourceContextChunkState],
) -> Vec<SourceContextChunkState> {
    chunks
        .iter()
        .map(|chunk| SourceContextChunkState {
            chunk_key: chunk.chunk_key.clone(),
            source_url: sanitize_text(&chunk.source_url),
            source_domain: sanitize_text(&chunk.source_domain),
            heading_path: sanitize_text(&chunk.heading_path),
            section_type: sanitize_text(&chunk.section_type),
            content_md: sanitize_text(&chunk.content_md),
            retrieval_score: chunk.retrieval_score.clone(),
            usage_policy: sanitize_text(&chunk.usage_policy),
        })
        .collect()
}

pub fn sanitize_support_bundle(
    support: &[SeoVerifiedFactSupportState],
) -> Vec<SeoVerifiedFactSupportState> {
    support
        .iter()
        .map(|entry| SeoVerifiedFactSupportState {
            fragment_text: sanitize_text(&entry.fragment_text),
            support_ref: entry.support_ref.clone(),
            role_type: entry.role_type.clone(),
            source_label: sanitize_text(&entry.source_label),
            source_tier: entry.source_tier.clone(),
            freshness_class: entry.freshness_class.clone(),
            observed_at: entry.observed_at.clone(),
            valid_until: entry.valid_until.clone(),
        })
        .collect()
}

pub fn sanitize_sections(sections: &[SeoDraftSectionState]) -> Vec<SeoDraftSectionState> {
    sections
        .iter()
        .map(|section| SeoDraftSectionState {
            section_key: section.section_key.clone(),
            section_role: section.section_role.clone(),
            heading: sanitize_text(&section.heading),
            body_markdown: sanitize_text(&section.body_markdown),
            required: section.required,
            traceability_label: section.traceability_label.clone(),
            support_refs: section.support_refs.clone(),
            template_key: section.template_key.clone(),
        })
        .collect()
}

pub fn sanitize_claim_ledger(entries: &[ClaimLedgerEntry]) -> Vec<ClaimLedgerEntry> {
    entries
        .iter()
        .map(|entry| ClaimLedgerEntry {
            claim_key: entry.claim_key.clone(),
            fragment_text: sanitize_text(&entry.fragment_text),
            claim_kind: entry.claim_kind.clone(),
            traceability_label: entry.traceability_label.clone(),
            support_refs: entry.support_refs.clone(),
            validation_verdict: entry.validation_verdict.clone(),
            source_section_key: entry.source_section_key.clone(),
        })
        .collect()
}

pub fn sanitize_content_blocks(blocks: &[RenderedContentBlock]) -> Vec<RenderedContentBlock> {
    blocks
        .iter()
        .map(|block| RenderedContentBlock {
            block_key: block.block_key.clone(),
            block_type: block.block_type.clone(),
            section_role: block.section_role.clone(),
            heading: sanitize_text(&block.heading),
            markdown: sanitize_text(&block.markdown),
            support_refs: block.support_refs.clone(),
            traceability_label: block.traceability_label.clone(),
            required: block.required,
        })
        .collect()
}

pub fn sanitize_candidate(candidate: &LlmDraftCandidate) -> LlmDraftCandidate {
    LlmDraftCandidate {
        candidate_key: candidate.candidate_key.clone(),
        request_key: candidate.request_key.clone(),
        provider_key: candidate.provider_key.clone(),
        model_key: candidate.model_key.clone(),
        prompt_version: candidate.prompt_version.clone(),
        body_markdown: sanitize_text(&candidate.body_markdown),
        sections: sanitize_sections(&candidate.sections),
        claim_ledger: sanitize_claim_ledger(&candidate.claim_ledger),
        content_blocks: sanitize_content_blocks(&candidate.content_blocks),
        faq_json: sanitize_text(&candidate.faq_json),
        schema_markup_json: sanitize_text(&candidate.schema_markup_json),
        status: candidate.status.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_email_phone_and_passport() {
        let text = "Contact info@appai.pl or +48 123 456 789, passport number AB123456.";
        let redacted = sanitize_text(text);
        assert!(!redacted.contains("info@appai.pl"));
        assert!(!redacted.contains("123 456 789"));
        assert!(!redacted.contains("AB123456"));
        assert!(redacted.contains("[redacted:email]"));
        assert!(redacted.contains("[redacted:phone]"));
        assert!(redacted.contains("[redacted:passport]"));
    }
}
