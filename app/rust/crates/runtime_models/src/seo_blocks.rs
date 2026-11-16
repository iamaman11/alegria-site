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
    fn opaque_sha_and_uuid_are_never_public_slugs() {
        assert!(is_opaque_topic_identifier(&"a".repeat(64)));
        assert!(is_opaque_topic_identifier(&format!("sha256:{}", "9".repeat(64))));
        assert!(is_opaque_topic_identifier("123e4567-e89b-12d3-a456-426614174000"));
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
