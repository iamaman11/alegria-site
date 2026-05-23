use contracts::generated::alegria::temporal::v1::{
    LinkRecommendationState, SectionTemplateBinding, SeoVerifiedFactSupportState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedContentBlock {
    pub section_role: String,
    pub block_type: String,
    pub heading: String,
    pub required: bool,
    pub deterministic_only: bool,
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
