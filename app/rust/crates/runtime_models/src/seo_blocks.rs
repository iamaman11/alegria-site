use contracts::generated::alegria::temporal::v1::{
    LinkRecommendationState, PageBlueprintState, SectionTemplateBinding,
    SeoVerifiedFactSupportState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedContentBlock {
    pub section_role: String,
    pub block_type: String,
    pub heading: String,
    pub required: bool,
    pub deterministic_only: bool,
}

/// Refuse technical identifiers as public slugs, including SHA-256 and UUID
/// shapes; keep this rule in one owner shared by IA and publication.
pub fn is_opaque_topic_identifier(value: &str) -> bool {
    let trimmed = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    let hex = trimmed.replace('-', "");
    hex.len() >= 32 && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

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

/// A public page's editorial locale comes from its canonical URL, not from
/// the language of a cited source. Custom non-default headings are preserved.
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

pub fn supports_for_role<'a>(
    support: &'a [SeoVerifiedFactSupportState],
    section_role: &str,
) -> Vec<&'a SeoVerifiedFactSupportState> {
    support
        .iter()
        .filter(|entry| role_matches_section(&entry.role_type, section_role))
        .collect()
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

/// Required factual sections must have *applicable, current* VERIFIED support
/// before scheduling expensive editorial generation. This is a completeness
/// check on a support bundle already resolved by the database; it does not
/// create verified knowledge or act as a replacement for publish-time QA.
pub fn missing_required_factual_roles(
    blueprint: &PageBlueprintState,
    support: &[SeoVerifiedFactSupportState],
) -> Vec<String> {
    let required_roles = if blueprint.required_sections.is_empty() {
        mandatory_section_roles()
            .iter()
            .map(|role| (*role).to_string())
            .collect::<Vec<_>>()
    } else {
        blueprint.required_sections.clone()
    };
    let mut missing = required_roles
        .into_iter()
        .filter(|role| factual_role(role))
        .filter(|role| {
            !supports_for_role(support, role).iter().any(|entry| {
                !entry.support_ref.trim().is_empty()
                    && !entry.fragment_text.trim().is_empty()
                    && entry.freshness_class == "fresh"
            })
        })
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    missing
}

pub fn planned_content_blocks(
    templates: &[SectionTemplateBinding],
    verified_support: &[SeoVerifiedFactSupportState],
    required_links: &[LinkRecommendationState],
) -> Vec<PlannedContentBlock> {
    templates
        .iter()
        .filter_map(|template| {
            let section_role = template.section_role.as_str();
            let has_support = !supports_for_role(verified_support, section_role).is_empty();
            let has_required_links = !required_links.is_empty();
            let keep = template.required
                || has_support
                || section_role == "cta_disclaimer"
                || (section_role == "related_pages" && has_required_links);
            if !keep {
                return None;
            }
            Some(PlannedContentBlock {
                section_role: template.section_role.clone(),
                block_type: block_type_for_role(section_role).to_string(),
                heading: if template.heading.trim().is_empty() {
                    heading_for_role(section_role).to_string()
                } else {
                    template.heading.clone()
                },
                required: template.required,
                deterministic_only: matches!(
                    section_role,
                    "documents" | "fees" | "timing" | "related_pages"
                ),
            })
        })
        .collect()
}

#[cfg(test)]
mod verified_page_readiness_tests {
    use super::*;

    fn support(role_type: &str) -> SeoVerifiedFactSupportState {
        SeoVerifiedFactSupportState {
            role_type: role_type.to_string(),
            support_ref: "verified-rule-1".to_string(),
            fragment_text: "Exact original source quotation".to_string(),
            freshness_class: "fresh".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn russian_locale_headings_are_reusable_across_unrelated_topics() {
        assert_eq!(locale_from_canonical_path("/ru/visa/spain/"), "ru");
        assert_eq!(
            locale_from_canonical_path("/ru/gardening/plant-care/"),
            "ru"
        );
        assert_eq!(heading_for_role_locale("fees", "ru"), "Консульский сбор");
        assert_eq!(heading_for_role_locale("documents", "ru"), "Документы");
        assert_eq!(heading_for_role_locale("fees", "en"), "Fees");
        assert_eq!(heading_for_role_locale("unknown", "ru"), "Section");
    }
    #[test]
    fn opaque_sha_and_uuid_are_never_public_slugs() {
        assert!(is_opaque_topic_identifier(&"a".repeat(64)));
        assert!(is_opaque_topic_identifier(&format!(
            "sha256:{}",
            "9".repeat(64)
        )));
        assert!(is_opaque_topic_identifier(
            "123e4567-e89b-12d3-a456-426614174000"
        ));
        assert!(!is_opaque_topic_identifier("plant-leaf-care"));
        assert!(!is_opaque_topic_identifier("spain-tourist-visa"));
    }

    #[test]
    fn page_specific_readiness_does_not_demand_unrelated_visa_blocks() {
        let blueprint = PageBlueprintState {
            required_sections: vec![
                "overview".to_string(),
                "faq".to_string(),
                "related_pages".to_string(),
            ],
            ..Default::default()
        };
        assert!(
            missing_required_factual_roles(&blueprint, &[support("eligibility_rule")]).is_empty()
        );
    }

    #[test]
    fn missing_fee_support_blocks_only_page_requiring_fee() {
        let blueprint = PageBlueprintState {
            required_sections: vec!["overview".to_string(), "fees".to_string()],
            ..Default::default()
        };
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[support("eligibility_rule")]),
            vec!["fees".to_string()]
        );
    }

    #[test]
    fn empty_or_stale_support_cannot_satisfy_required_role() {
        let blueprint = PageBlueprintState {
            required_sections: vec!["fees".to_string()],
            ..Default::default()
        };
        let mut stale = support("fee_item");
        stale.freshness_class = "stale".to_string();
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[stale]),
            vec!["fees"]
        );
        let mut empty = support("fee_item");
        empty.fragment_text.clear();
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[empty]),
            vec!["fees"]
        );
    }

    #[test]
    fn default_blueprint_requires_real_factual_section_support() {
        let missing = missing_required_factual_roles(
            &PageBlueprintState::default(),
            &[support("eligibility_rule")],
        );
        assert!(missing.contains(&"documents".to_string()));
        assert!(missing.contains(&"fees".to_string()));
        assert!(missing.contains(&"timing".to_string()));
    }
}

/// The original quotation remains the verified evidence. A proposed editorial
/// rendering is not itself a VERIFIED fact; semantic equivalence is reviewed
/// by the existing human CMS approval of the exact revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorialClaimError {
    MissingProposal,
    InvalidText,
    InvalidQuestion,
    MissingOrAmbiguousSupport,
    EvidenceMismatch,
    InvalidSourceUrl,
    NumericEvidenceMismatch,
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

fn numeric_tokens(text: &str) -> std::collections::BTreeSet<String> {
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

/// Resolve the exact ref, quotation, source URL, freshness and role independently
/// against the admissible bundle. Never accept a model's self-declared evidence.
pub fn validate_editorial_claim(
    claim: &contracts::generated::alegria::temporal::v1::ClaimLedgerEntry,
    support: &[SeoVerifiedFactSupportState],
) -> Result<(), EditorialClaimError> {
    let proposal = claim
        .editorial_proposal
        .as_ref()
        .ok_or(EditorialClaimError::MissingProposal)?;
    if !plain_editorial_text(&proposal.text) {
        return Err(EditorialClaimError::InvalidText);
    }
    if (claim.source_section_key == "faq"
        && (!plain_editorial_text(&proposal.question)
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
        || claim.traceability_label != support_traceability_label(&claim.source_section_key)
        || source.freshness_class != "fresh"
        || !factual_role(&claim.source_section_key)
        || !role_matches_section(&source.role_type, &claim.source_section_key)
        || claim.fragment_text.trim().is_empty()
        || claim.fragment_text != source.fragment_text
        || proposal.source_url != source.source_url
    {
        return Err(EditorialClaimError::EvidenceMismatch);
    }
    if !safe_verified_source_url(&proposal.source_url) {
        return Err(EditorialClaimError::InvalidSourceUrl);
    }
    let source_numbers = numeric_tokens(&source.fragment_text);
    if numeric_tokens(&proposal.text) != source_numbers
        || !numeric_tokens(&proposal.question).is_subset(&source_numbers)
    {
        return Err(EditorialClaimError::NumericEvidenceMismatch);
    }
    Ok(())
}

/// Canonical display text is generated from ledger proposals, never from an
/// independent free-form LLM Markdown block.
pub fn render_editorial_claim(
    claim: &contracts::generated::alegria::temporal::v1::ClaimLedgerEntry,
    locale: &str,
) -> Result<String, EditorialClaimError> {
    let proposal = claim
        .editorial_proposal
        .as_ref()
        .ok_or(EditorialClaimError::MissingProposal)?;
    if !plain_editorial_text(&proposal.text) {
        return Err(EditorialClaimError::InvalidText);
    }
    if !safe_verified_source_url(&proposal.source_url) {
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
        if !plain_editorial_text(&proposal.question) || !proposal.question.trim_end().ends_with('?')
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

#[cfg(test)]
mod localized_claim_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{ClaimLedgerEntry, EditorialClaimProposal};

    fn sample() -> (ClaimLedgerEntry, SeoVerifiedFactSupportState) {
        let source = SeoVerifiedFactSupportState {
            support_ref: "rule-one".into(),
            role_type: "timeline_item".into(),
            freshness_class: "fresh".into(),
            fragment_text: "The decision normally takes 15 days.".into(),
            source_url: "https://example.org/official".into(),
            ..Default::default()
        };
        let claim = ClaimLedgerEntry {
            claim_kind: "factual".into(),
            traceability_label: "verified_fact".into(),
            validation_verdict: "supported".into(),
            source_section_key: "timing".into(),
            support_refs: vec![source.support_ref.clone()],
            fragment_text: source.fragment_text.clone(),
            editorial_proposal: Some(EditorialClaimProposal {
                text: "Обычно решение принимается за 15 дней.".into(),
                source_url: source.source_url.clone(),
                question: String::new(),
            }),
            ..Default::default()
        };
        (claim, source)
    }

    #[test]
    fn verified_original_and_proposed_localized_text_are_distinct() {
        let (claim, source) = sample();
        assert_eq!(validate_editorial_claim(&claim, &[source.clone()]), Ok(()));
        let rendered = render_editorial_claim(&claim, "ru").unwrap();
        assert!(rendered.contains("Обычно решение принимается за 15 дней."));
        assert!(rendered.contains("[Источник](https://example.org/official)"));
        assert!(!rendered.contains(&source.fragment_text));
        assert_eq!(claim.fragment_text, source.fragment_text);
    }

    #[test]
    fn reject_wrong_ref_quote_role_or_untrusted_link() {
        let (original, source) = sample();
        for claim in [
            ClaimLedgerEntry {
                support_refs: vec!["wrong".into()],
                ..original.clone()
            },
            ClaimLedgerEntry {
                fragment_text: "The decision takes 3 days.".into(),
                ..original.clone()
            },
            ClaimLedgerEntry {
                source_section_key: "fees".into(),
                ..original.clone()
            },
            ClaimLedgerEntry {
                editorial_proposal: Some(EditorialClaimProposal {
                    source_url: "https://unverified.test".into(),
                    ..original.editorial_proposal.clone().unwrap()
                }),
                ..original.clone()
            },
        ] {
            assert!(validate_editorial_claim(&claim, &[source.clone()]).is_err());
        }
    }

    #[test]
    fn reject_stale_ambiguous_evidence_and_markup_injection() {
        let (mut claim, mut source) = sample();
        assert!(validate_editorial_claim(&claim, &[source.clone(), source.clone()]).is_err());
        source.freshness_class = "stale".into();
        assert!(validate_editorial_claim(&claim, &[source.clone()]).is_err());
        claim.editorial_proposal.as_mut().unwrap().text =
            "[injected](https://elsewhere.test)".into();
        assert_eq!(
            render_editorial_claim(&claim, "ru"),
            Err(EditorialClaimError::InvalidText)
        );
    }

    #[test]
    fn faq_produces_reviewable_question_and_answer() {
        let (mut claim, source) = sample();
        claim.source_section_key = "faq".into();
        claim.traceability_label = "verified_summary".into();
        claim.editorial_proposal.as_mut().unwrap().question = "Сколько ждать решения?".into();
        assert!(validate_editorial_claim(&claim, &[source]).is_ok());
        assert!(render_editorial_claim(&claim, "ru")
            .unwrap()
            .starts_with("### Сколько ждать решения?"));
    }

    #[test]
    fn reject_new_or_dropped_numeric_values() {
        let (mut claim, source) = sample();
        for text in [
            "Решение принимается за 3 дня.",
            "Решение принимается быстро.",
        ] {
            claim.editorial_proposal.as_mut().unwrap().text = text.into();
            assert_eq!(
                validate_editorial_claim(&claim, &[source.clone()]),
                Err(EditorialClaimError::NumericEvidenceMismatch)
            );
        }
    }

    #[test]
    fn evidence_binding_is_not_a_semantic_verification() {
        let (mut claim, source) = sample();
        claim.editorial_proposal.as_mut().unwrap().text =
            "Решение никогда не принимается за 15 дней.".into();
        // Matching provenance does not prove the meaning of a paraphrase.
        // This proposal must still be rejected by the separate human review.
        assert!(validate_editorial_claim(&claim, &[source]).is_ok());
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

/// External evidence references are separate from graph-owned internal
/// navigation; never expose target page keys as public link text.
pub fn render_verified_source_list(
    support: &[SeoVerifiedFactSupportState],
    locale: &str,
) -> String {
    let label = match locale {
        "ru" => "Источник",
        "es" => "Fuente",
        "pl" => "Źródło",
        _ => "Source",
    };
    let urls = support
        .iter()
        .filter(|entry| {
            entry.freshness_class == "fresh" && safe_verified_source_url(&entry.source_url)
        })
        .map(|entry| entry.source_url.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    urls.into_iter()
        .map(|url| format!("- [{label}: {url}]({url})"))
        .collect::<Vec<_>>()
        .join("\n")
}
