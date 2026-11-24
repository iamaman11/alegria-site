//! Editorial rules for a publishable page. This module accepts domain values,
//! never Temporal messages: transport conversion belongs to the caller.

use std::collections::BTreeSet;

pub fn heading_for_role(section_role: &str) -> &'static str {
    match section_role {
        "overview" => "Overview",
        "who_fits" => "Who this fits",
        "documents" => "Documents",
        "process" => "Process",
        "fees" => "Fees",
        "timing" => "Timing",
        "where_to_apply" => "Where to apply",
        "faq" => "FAQ",
        "related_pages" => "Related pages",
        "cta_disclaimer" => "Next step",
        _ => "Section",
    }
}

/// The canonical page URL determines the editorial locale, not its evidence.
pub fn locale_from_canonical_path(path: &str) -> &str {
    path.split('/')
        .find(|part| !part.is_empty())
        .unwrap_or("en")
}

pub fn heading_for_role_locale(role: &str, locale: &str) -> &'static str {
    if locale != "ru" {
        return heading_for_role(role);
    }
    match role {
        "overview" => "Обзор",
        "who_fits" => "Кому подходит",
        "documents" => "Документы",
        "process" => "Порядок оформления",
        "fees" => "Консульский сбор",
        "timing" => "Сроки",
        "where_to_apply" => "Куда подавать заявление",
        "faq" => "Вопросы и ответы",
        "related_pages" => "Источники и связанные страницы",
        "cta_disclaimer" => "Что делать дальше",
        _ => heading_for_role(role),
    }
}

pub fn block_type_for_role(section_role: &str) -> &'static str {
    match section_role {
        "documents" => "documents_checklist",
        "fees" => "fees_table",
        "timing" => "timeline",
        "faq" => "faq",
        "related_pages" => "related_links",
        "cta_disclaimer" => "cta_disclaimer",
        _ => "prose",
    }
}

pub fn role_matches_section(role_type: &str, section_role: &str) -> bool {
    match section_role {
        "documents" => matches!(
            role_type,
            "document_required" | "must_provide" | "form_required"
        ),
        "fees" => matches!(role_type, "fee_item" | "must_pay"),
        "timing" => matches!(role_type, "timeline_item" | "timeline"),
        "where_to_apply" => matches!(role_type, "where_to_apply" | "appointment_rule"),
        "process" => matches!(role_type, "step"),
        "who_fits" => matches!(
            role_type,
            "eligibility_rule" | "must_satisfy" | "allows" | "forbids"
        ),
        "overview" | "faq" => true,
        _ => false,
    }
}

pub fn factual_role(section_role: &str) -> bool {
    !matches!(section_role, "related_pages" | "cta_disclaimer")
}

pub fn support_traceability_label(section_role: &str) -> &'static str {
    match section_role {
        "overview" | "faq" => "verified_summary",
        _ => "verified_fact",
    }
}

/// A traceability entry is verified evidence only when its verdict and label
/// agree; both content-contract and final QA use this editorial definition.
pub fn verified_traceability_evidence(verdict: &str, label: &str) -> bool {
    verdict == "supported" && matches!(label, "verified_fact" | "verified_summary")
}

pub fn default_section_roles() -> &'static [&'static str] {
    &[
        "overview",
        "who_fits",
        "documents",
        "process",
        "fees",
        "timing",
        "where_to_apply",
        "faq",
        "related_pages",
        "cta_disclaimer",
    ]
}

pub fn mandatory_section_roles() -> &'static [&'static str] {
    &[
        "overview",
        "documents",
        "fees",
        "timing",
        "related_pages",
        "cta_disclaimer",
    ]
}

#[derive(Debug, Clone, Copy)]
pub struct SectionTemplate<'a> {
    pub section_role: &'a str,
    pub heading: &'a str,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedContentBlock {
    pub section_role: String,
    pub block_type: String,
    pub heading: String,
    pub required: bool,
    pub deterministic_only: bool,
}

/// The one domain shape of a rendered editorial content block. The content
/// contract checks its required structure; final QA checks the same block's
/// source-bound text against its assembled section and claim ledger.
#[derive(Clone, Copy)]
pub struct ContentBlock<'a> {
    pub role: &'a str,
    pub block_type: &'a str,
    pub heading: &'a str,
    pub markdown: &'a str,
    pub required: bool,
    pub support_refs: &'a [String],
}

/// Select and describe the blocks to generate, consistently for both the
/// deterministic draft and LLM prompt. No transport fields enter this rule.
pub fn planned_content_blocks(
    templates: &[SectionTemplate<'_>],
    verified_support: &[VerifiedSupport<'_>],
    has_required_links: bool,
) -> Vec<PlannedContentBlock> {
    templates
        .iter()
        .filter_map(|template| {
            let role = template.section_role;
            let has_support = verified_support
                .iter()
                .any(|entry| role_matches_section(entry.role_type, role));
            if !template.required
                && !has_support
                && role != "cta_disclaimer"
                && !(role == "related_pages" && has_required_links)
            {
                return None;
            }
            Some(PlannedContentBlock {
                section_role: role.to_string(),
                block_type: block_type_for_role(role).to_string(),
                heading: if template.heading.trim().is_empty() {
                    heading_for_role(role).to_string()
                } else {
                    template.heading.to_string()
                },
                required: template.required,
                deterministic_only: matches!(
                    role,
                    "documents" | "fees" | "timing" | "related_pages"
                ),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct VerifiedSupport<'a> {
    pub support_ref: &'a str,
    pub fragment_text: &'a str,
    pub role_type: &'a str,
    pub freshness_class: &'a str,
    pub source_url: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct EditorialProposal<'a> {
    pub text: &'a str,
    pub question: &'a str,
    pub source_url: &'a str,
    pub evidence_spans: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub struct EditorialClaim<'a> {
    pub source_section_key: &'a str,
    pub claim_kind: &'a str,
    pub validation_verdict: &'a str,
    pub traceability_label: &'a str,
    pub fragment_text: &'a str,
    pub support_refs: &'a [String],
    pub proposal: Option<EditorialProposal<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorialClaimError {
    MissingProposal,
    InvalidText,
    InvalidQuestion,
    MissingOrAmbiguousSupport,
    EvidenceMismatch,
    InvalidSourceUrl,
    NumericEvidenceMismatch,
    InvalidEvidenceSpan,
}

fn plain_editorial_text(text: &str) -> bool {
    let text = text.trim();
    let size = text.chars().count();
    (4..=2000).contains(&size)
        && !text.contains("://")
        && !text.to_ascii_lowercase().contains("www.")
        && !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '[' | ']' | '*' | '`'))
}

fn numeric_tokens(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn safe_verified_source_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
        && !url.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '\\')
        })
}

/// The claim must bind to an independent, current, unique verified quotation.
/// This protects provenance; semantic correctness remains subject to CMS review.
pub fn validate_editorial_claim(
    claim: EditorialClaim<'_>,
    support: &[VerifiedSupport<'_>],
) -> Result<(), EditorialClaimError> {
    let proposal = claim.proposal.ok_or(EditorialClaimError::MissingProposal)?;
    if !plain_editorial_text(proposal.text) {
        return Err(EditorialClaimError::InvalidText);
    }
    if (claim.source_section_key == "faq"
        && (!plain_editorial_text(proposal.question)
            || !proposal.question.trim_end().ends_with('?')))
        || (claim.source_section_key != "faq" && !proposal.question.is_empty())
    {
        return Err(EditorialClaimError::InvalidQuestion);
    }
    if claim.support_refs.len() != 1 || claim.support_refs[0].trim().is_empty() {
        return Err(EditorialClaimError::MissingOrAmbiguousSupport);
    }
    let mut matches = support
        .iter()
        .filter(|entry| entry.support_ref == claim.support_refs[0]);
    let source = matches
        .next()
        .ok_or(EditorialClaimError::MissingOrAmbiguousSupport)?;
    if matches.next().is_some() {
        return Err(EditorialClaimError::MissingOrAmbiguousSupport);
    }
    if claim.claim_kind != "factual"
        || claim.validation_verdict != "supported"
        || claim.traceability_label != support_traceability_label(claim.source_section_key)
        || source.freshness_class != "fresh"
        || !factual_role(claim.source_section_key)
        || !role_matches_section(source.role_type, claim.source_section_key)
        || claim.fragment_text.trim().is_empty()
        || claim.fragment_text != source.fragment_text
        || proposal.source_url != source.source_url
    {
        return Err(EditorialClaimError::EvidenceMismatch);
    }
    if !safe_verified_source_url(proposal.source_url) {
        return Err(EditorialClaimError::InvalidSourceUrl);
    }
    // An explicit evidence span must be a complete, disjoint source line or
    // paragraph, not a cherry-picked fragment or model-invented quotation.
    let source_numbers = if proposal.evidence_spans.is_empty() {
        numeric_tokens(source.fragment_text)
    } else {
        if proposal.evidence_spans.len() > 8 {
            return Err(EditorialClaimError::InvalidEvidenceSpan);
        }
        let quote = source.fragment_text;
        let mut ranges = Vec::new();
        let mut selected = BTreeSet::new();
        for span in proposal.evidence_spans {
            if span.trim() != span || span.chars().count() < 12 || span.contains('\r') {
                return Err(EditorialClaimError::InvalidEvidenceSpan);
            }
            let start = quote
                .find(span)
                .ok_or(EditorialClaimError::InvalidEvidenceSpan)?;
            let end = start + span.len();
            if (start != 0 && quote.as_bytes()[start - 1] != b'\n')
                || (end != quote.len() && quote.as_bytes()[end] != b'\n')
                || ranges.iter().any(|(lo, hi)| start < *hi && end > *lo)
            {
                return Err(EditorialClaimError::InvalidEvidenceSpan);
            }
            ranges.push((start, end));
            selected.extend(numeric_tokens(span));
        }
        selected
    };
    if numeric_tokens(proposal.text) != source_numbers
        || !numeric_tokens(proposal.question).is_subset(&source_numbers)
    {
        return Err(EditorialClaimError::NumericEvidenceMismatch);
    }
    Ok(())
}

/// Only verified ledger proposals may become public Markdown text.
pub fn render_editorial_claim(
    claim: EditorialClaim<'_>,
    locale: &str,
) -> Result<String, EditorialClaimError> {
    let proposal = claim.proposal.ok_or(EditorialClaimError::MissingProposal)?;
    if !plain_editorial_text(proposal.text) {
        return Err(EditorialClaimError::InvalidText);
    }
    if !safe_verified_source_url(proposal.source_url) {
        return Err(EditorialClaimError::InvalidSourceUrl);
    }
    let label = match locale {
        "ru" => "Источник",
        "es" => "Fuente",
        "pl" => "Źródło",
        _ => "Source",
    };
    let text = format!(
        "{} [{}]({})",
        proposal.text.trim(),
        label,
        proposal.source_url
    );
    if claim.source_section_key == "faq" {
        if !plain_editorial_text(proposal.question) || !proposal.question.trim_end().ends_with('?')
        {
            return Err(EditorialClaimError::InvalidQuestion);
        }
        Ok(format!("### {}\n\n{}", proposal.question.trim(), text))
    } else if !proposal.question.is_empty() {
        Err(EditorialClaimError::InvalidQuestion)
    } else {
        Ok(format!("- {text}"))
    }
}

pub fn editorial_cta(locale: &str) -> &'static str {
    match locale {
        "ru" => "Перед принятием решения ознакомьтесь с действующими официальными источниками.",
        "es" => "Consulte las fuentes oficiales vigentes antes de tomar una decisión.",
        "pl" => "Przed podjęciem decyzji sprawdź aktualne źródła oficjalne.",
        _ => "Consult current official sources before making a decision.",
    }
}

/// Graph-owned navigation and externally verified evidence are distinct links.
pub fn render_verified_source_list(support: &[VerifiedSupport<'_>], locale: &str) -> String {
    let label = match locale {
        "ru" => "Источник",
        "es" => "Fuente",
        "pl" => "Źródło",
        _ => "Source",
    };
    let urls = support
        .iter()
        .filter(|entry| {
            entry.freshness_class == "fresh" && safe_verified_source_url(entry.source_url)
        })
        .map(|entry| entry.source_url)
        .collect::<BTreeSet<_>>();
    urls.into_iter()
        .map(|url| format!("- [{label}: {url}]({url})"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(
        support_ref: &'a [String],
        spans: &'a [String],
    ) -> (EditorialClaim<'a>, VerifiedSupport<'static>) {
        let claim = EditorialClaim {
            claim_kind: "factual",
            validation_verdict: "supported",
            traceability_label: "verified_fact",
            source_section_key: "timing",
            fragment_text: "The decision normally takes 15 days.",
            support_refs: support_ref,
            proposal: Some(EditorialProposal {
                text: "Обычно решение принимается за 15 дней.",
                question: "",
                source_url: "https://example.org/official",
                evidence_spans: spans,
            }),
        };
        let source = VerifiedSupport {
            support_ref: "rule-one",
            role_type: "timeline_item",
            freshness_class: "fresh",
            fragment_text: "The decision normally takes 15 days.",
            source_url: "https://example.org/official",
        };
        (claim, source)
    }

    #[test]
    fn locale_roles_and_opaque_values_are_not_transport_dependent() {
        assert_eq!(
            locale_from_canonical_path("/ru/gardening/plant-care/"),
            "ru"
        );
        assert_eq!(heading_for_role_locale("fees", "ru"), "Консульский сбор");
        assert_eq!(heading_for_role_locale("unknown", "ru"), "Section");
        assert_eq!(block_type_for_role("documents"), "documents_checklist");
    }

    #[test]
    fn only_supported_facts_and_summaries_are_verified_traceability() {
        assert!(verified_traceability_evidence("supported", "verified_fact"));
        assert!(verified_traceability_evidence("supported", "verified_summary"));
        for (verdict, label) in [
            ("blocked", "verified_fact"),
            ("supported", "unsupported_factual_fragment"),
            ("", "verified_summary"),
        ] {
            assert!(!verified_traceability_evidence(verdict, label));
        }
    }

    #[test]
    fn block_selection_retains_mandatory_and_verified_sections_only() {
        let sections = [
            SectionTemplate {
                section_role: "fees",
                heading: "",
                required: false,
            },
            SectionTemplate {
                section_role: "documents",
                heading: "Custom title",
                required: true,
            },
            SectionTemplate {
                section_role: "related_pages",
                heading: "",
                required: false,
            },
        ];
        let source = VerifiedSupport {
            support_ref: "fee-one",
            fragment_text: "Fee",
            role_type: "fee_item",
            freshness_class: "fresh",
            source_url: "https://example.org/fees",
        };
        let blocks = planned_content_blocks(&sections, &[source], false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].heading, "Fees");
        assert!(blocks[0].deterministic_only);
        assert_eq!(blocks[1].heading, "Custom title");
        assert!(blocks[1].required);
        assert_eq!(planned_content_blocks(&sections, &[], true).len(), 2);
    }

    #[test]
    fn verified_claim_renders_localized_text_without_rewriting_evidence() {
        let refs = vec!["rule-one".into()];
        let (claim, source) = sample(&refs, &[]);
        assert_eq!(validate_editorial_claim(claim, &[source]), Ok(()));
        assert_eq!(
            render_editorial_claim(claim, "ru").unwrap(),
            "- Обычно решение принимается за 15 дней. [Источник](https://example.org/official)"
        );
    }

    #[test]
    fn mismatched_numbers_or_sources_fail_closed() {
        let refs = vec!["rule-one".into()];
        let (mut claim, source) = sample(&refs, &[]);
        claim.proposal.as_mut().unwrap().text = "Срок 3 дня.";
        assert_eq!(
            validate_editorial_claim(claim, &[source]),
            Err(EditorialClaimError::NumericEvidenceMismatch)
        );
        claim.proposal.as_mut().unwrap().text = "Обычно решение принимается за 15 дней.";
        assert_eq!(
            validate_editorial_claim(claim, &[source, source]),
            Err(EditorialClaimError::MissingOrAmbiguousSupport)
        );
    }

    #[test]
    fn composite_quote_accepts_only_exact_lines() {
        let refs = vec!["rule-one".into()];
        let spans = vec!["The decision normally takes 15 days.".into()];
        let (claim, source) = sample(&refs, &spans);
        assert_eq!(validate_editorial_claim(claim, &[source]), Ok(()));
        let bad = vec!["15 days".into()];
        let (claim, source) = sample(&refs, &bad);
        assert_eq!(
            validate_editorial_claim(claim, &[source]),
            Err(EditorialClaimError::InvalidEvidenceSpan)
        );
    }
}
