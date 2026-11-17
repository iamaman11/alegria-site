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

// A URL authority is structured data, not a telephone number. Protect only
// a validated IPv4 host and its optional port, never URL credentials,
// paths, query parameters or fragments. The same rule applies to a source
// field and links embedded in Markdown/JSON, preserving canonical rendering.
fn ipv4_url_authority_re() -> &'static Regex {
    static URL_AUTHORITY: OnceLock<Regex> = OnceLock::new();
    URL_AUTHORITY.get_or_init(|| {
        Regex::new(
            r"(?i)\bhttps?://(?P<host>(?:[0-9]{1,3}\.){3}[0-9]{1,3})(?::(?P<port>[0-9]{1,5}))?",
        )
        .expect("IPv4 URL authority regex")
    })
}

fn redact_prose(text: &str) -> String {
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

pub fn sanitize_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for captures in ipv4_url_authority_re().captures_iter(text) {
        let authority = captures.get(0).expect("full authority match");
        // Reject partial hosts, userinfo and truncated/invalid ports.
        let boundary = text[authority.end()..].chars().next();
        if !boundary.is_none_or(|ch| {
            ch.is_whitespace() || matches!(ch, '/' | '?' | '#' | ')' | ']' | '>' | '"' | '\'')
        }) {
            continue;
        }
        if captures["host"].parse::<std::net::Ipv4Addr>().is_err()
            || captures
                .name("port")
                .is_some_and(|port| port.as_str().parse::<u16>().is_err())
        {
            continue;
        }
        output.push_str(&redact_prose(&text[cursor..authority.start()]));
        output.push_str(authority.as_str());
        cursor = authority.end();
    }
    output.push_str(&redact_prose(&text[cursor..]));
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
            source_url: sanitize_text(&entry.source_url),
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
            editorial_proposal: entry.editorial_proposal.as_ref().map(|proposal| {
                contracts::generated::alegria::temporal::v1::EditorialClaimProposal {
                    text: sanitize_text(&proposal.text),
                    question: sanitize_text(&proposal.question),
                    source_url: sanitize_text(&proposal.source_url),
                }
            }),
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
        seo_title: sanitize_text(&candidate.seo_title),
        seo_meta_description: sanitize_text(&candidate.seo_meta_description),
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

    #[test]
    fn source_authority_preserved_in_fields_and_rendered_links() {
        let url = "http://127.0.0.1:41671/official/visa";
        let support = SeoVerifiedFactSupportState {
            source_url: url.into(),
            source_label: "Official source".into(),
            fragment_text: "Decision normally takes 15 days.".into(),
            support_ref: "rule-1".into(),
            role_type: "timeline_item".into(),
            freshness_class: "fresh".into(),
            ..Default::default()
        };
        assert_eq!(sanitize_support_bundle(&[support])[0].source_url, url);
        let markdown = format!("- Decision normally takes 15 days. [Source]({url})");
        assert_eq!(sanitize_text(&markdown), markdown);
        let json = format!(r#"{{"url":"{url}"}}"#);
        assert_eq!(sanitize_text(&json), json);
        assert_eq!(sanitize_text(url), url);
        assert_eq!(sanitize_text(&sanitize_text(url)), url);
    }

    #[test]
    fn keeps_redacting_sensitive_url_paths_queries_and_surrounding_prose() {
        let url = "https://127.0.0.1:443/path/AB?email=person@example.org&phone=123456789";
        let text = format!("Call +48 123 456 789 before opening {url}");
        let clean = sanitize_text(&text);
        assert!(clean.contains("https://127.0.0.1:443/path/AB"));
        assert!(!clean.contains("person@example.org"));
        assert!(!clean.contains("123456789"));
        assert!(!clean.contains("123 456 789"));
        let invalid = "https://127.0.0.1:99999@other.example/";
        assert!(sanitize_text(invalid).contains("[redacted:phone]"));
        let standalone = "Call 127.0.0.1 for help";
        assert!(sanitize_text(standalone).contains("[redacted:phone]"));
    }
}
