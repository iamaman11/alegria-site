//! Pure information-architecture decisions for the existing SEO planning step.
//!
//! The Temporal step translates protobuf inputs and builds wire payloads. Page
//! classification, publishable paths, section requirements and disposition
//! belong here, with no knowledge of protobuf, runtime storage or SDKs.

use crate::artifact_identity::canonical_slug;

pub struct IaScope<'a> {
    pub locale: &'a str,
    pub country_code: &'a str,
    pub visa_type: &'a str,
}

pub struct IaScopePaths {
    pub country_segment: String,
    pub country_prefix: String,
    pub canonical_prefix: String,
    pub country_hub_slug: String,
    pub visa_hub_slug: String,
}

fn locale_lang(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("und")
        .to_ascii_lowercase()
}

fn country_slug(country_code: &str) -> String {
    match country_code.to_ascii_uppercase().as_str() {
        "ES" => "spain".to_string(),
        "TH" => "thailand".to_string(),
        "PL" => "poland".to_string(),
        "FR" => "france".to_string(),
        "IT" => "italy".to_string(),
        "DE" => "germany".to_string(),
        "GR" => "greece".to_string(),
        "PT" => "portugal".to_string(),
        "CZ" => "czech-republic".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

fn slug(input: &str) -> String {
    let canonical = canonical_slug(input);
    if canonical.is_empty() {
        "page".to_string()
    } else {
        canonical
    }
}

pub fn scope_paths(scope: Option<IaScope<'_>>) -> IaScopePaths {
    let Some(scope) = scope else {
        return IaScopePaths {
            country_segment: "all".into(),
            country_prefix: "/und/visa/all".into(),
            canonical_prefix: "/und/visa/all/all".into(),
            country_hub_slug: "visa-country-hub".into(),
            visa_hub_slug: "visa-hub".into(),
        };
    };
    let country_segment = country_slug(scope.country_code);
    let country_prefix = format!("/{}/visa/{country_segment}", locale_lang(scope.locale));
    let canonical_prefix = format!("{country_prefix}/{}", slug(scope.visa_type));
    let country_hub_slug = slug(&format!("{country_segment} visa"));
    let visa_hub_slug = slug(&format!("{country_segment} {} visa", scope.visa_type));
    IaScopePaths {
        country_segment,
        country_prefix,
        canonical_prefix,
        country_hub_slug,
        visa_hub_slug,
    }
}

fn multi_section_demand(seed: &str) -> bool {
    let text = seed.to_lowercase();
    [
        ["document", "requirement", "документ"],
        ["cost", "fee", "стоим"],
        ["time", "processing", "срок"],
    ]
    .iter()
    .filter(|terms| terms.iter().any(|term| text.contains(*term)))
    .count()
        > 1
}

fn page_type_for_keyword(seed_keyword: &str) -> &'static str {
    let query = seed_keyword.to_lowercase();
    if query.contains("document") || query.contains("requirements") || query.contains("документ")
    {
        "requirement_page"
    } else if query.contains("cost")
        || query.contains("fee")
        || query.contains("price")
        || query.contains("стоим")
    {
        "fee_page"
    } else if query.contains("time") || query.contains("processing") || query.contains("срок") {
        "timeline_page"
    } else if query.contains("faq") || query.contains("вопрос") {
        "faq_page"
    } else {
        "detail_page"
    }
}

fn page_type_path_segment(page_type: &str, canonical_slug: &str) -> String {
    match page_type {
        "requirement_page" => format!("requirements/{canonical_slug}"),
        "fee_page" => format!("fees/{canonical_slug}"),
        "timeline_page" => format!("processing-times/{canonical_slug}"),
        "faq_page" => format!("faq/{canonical_slug}"),
        "checklist_page" => format!("checklist/{canonical_slug}"),
        "troubleshooting_page" => format!("problems/{canonical_slug}"),
        "comparison_page" => format!("compare/{canonical_slug}"),
        "supporting_editorial" => format!("guides/{canonical_slug}"),
        _ => canonical_slug.to_string(),
    }
}

pub fn required_sections_for_page_type(page_type: &str) -> Vec<String> {
    let roles: &[&str] = match page_type {
        "country_hub_page" => &[
            "overview",
            "who_fits",
            "where_to_apply",
            "related_pages",
            "cta_disclaimer",
        ],
        "fee_page" => &["overview", "fees", "faq", "related_pages", "cta_disclaimer"],
        "timeline_page" => &[
            "overview",
            "timing",
            "process",
            "faq",
            "related_pages",
            "cta_disclaimer",
        ],
        "requirement_page" | "checklist_page" => &[
            "overview",
            "documents",
            "faq",
            "related_pages",
            "cta_disclaimer",
        ],
        "faq_page" => &["overview", "faq", "related_pages", "cta_disclaimer"],
        _ => &[
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
        ],
    };
    roles.iter().map(|role| (*role).to_string()).collect()
}

/// Section roles and routes used for pages derived from verified evidence.
/// Valid evidence and its transport representation are selected by the caller.
pub const EVIDENCE_DERIVED_PAGES: [(&str, &str, &str); 3] = [
    ("requirement_page", "documents", "documents"),
    ("fee_page", "fees", "fees"),
    ("timeline_page", "timing", "processing-times"),
];

pub struct IaClusterCandidate<'a> {
    pub seed_keyword: &'a str,
    pub topic_keys: &'a [String],
    /// Already decoded from the storage identifier and any semantic topic.
    pub semantic_keyword: Option<&'a str>,
    pub graph_confidence: f64,
    pub dominant_intent: &'a str,
    pub has_verified_support: bool,
    pub canonical_prefix: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IaLifecycle {
    Planned,
    Blocked,
}

impl IaLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Blocked => "blocked",
        }
    }
}

pub struct IaClusterPlan {
    pub topic_family_key: String,
    pub canonical_slug: String,
    pub page_type_key: String,
    pub canonical_url_path: String,
    pub dominant_intent: String,
    pub lifecycle: IaLifecycle,
}

pub enum IaClusterDisposition {
    UseExistingHub,
    Plan(IaClusterPlan),
}

pub fn plan_cluster(candidate: &IaClusterCandidate<'_>) -> IaClusterDisposition {
    let source_discovery_query = candidate
        .seed_keyword
        .split_whitespace()
        .any(|term| term.to_ascii_lowercase().starts_with("site:"));
    if candidate.has_verified_support
        && !source_discovery_query
        && candidate.semantic_keyword.is_some_and(multi_section_demand)
    {
        return IaClusterDisposition::UseExistingHub;
    }

    let ambiguous_topic_family = candidate.topic_keys.len() > 1 && candidate.graph_confidence < 0.6;
    let topic_family_key = if candidate.topic_keys.is_empty() {
        candidate.seed_keyword.to_string()
    } else {
        candidate.topic_keys.join("|")
    };
    let canonical_slug = candidate
        .semantic_keyword
        .map(slug)
        .unwrap_or_else(|| "needs-editorial-review".to_string());
    let page_type_key = candidate
        .semantic_keyword
        .map(page_type_for_keyword)
        .unwrap_or("detail_page")
        .to_string();
    let path_segment = page_type_path_segment(&page_type_key, &canonical_slug);
    let canonical_url_path = format!("{}/{path_segment}/", candidate.canonical_prefix);
    let dominant_intent = if candidate.dominant_intent.is_empty() {
        "informational".to_string()
    } else {
        candidate.dominant_intent.to_string()
    };
    IaClusterDisposition::Plan(IaClusterPlan {
        topic_family_key,
        canonical_slug,
        page_type_key,
        canonical_url_path,
        dominant_intent,
        lifecycle: if ambiguous_topic_family
            || source_discovery_query
            || candidate.semantic_keyword.is_none()
        {
            IaLifecycle::Blocked
        } else {
            IaLifecycle::Planned
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster<'a>(seed: &'a str, semantic: Option<&'a str>) -> IaClusterCandidate<'a> {
        IaClusterCandidate {
            seed_keyword: seed,
            topic_keys: &[],
            semantic_keyword: semantic,
            graph_confidence: 0.9,
            dominant_intent: "",
            has_verified_support: false,
            canonical_prefix: "/ru/visa/spain/tourist",
        }
    }

    #[test]
    fn scope_paths_keep_canonical_hub_and_fallback_identity() {
        let scope = scope_paths(Some(IaScope {
            locale: "ru-RU",
            country_code: "ES",
            visa_type: "tourist",
        }));
        assert_eq!(scope.country_segment, "spain");
        assert_eq!(scope.country_prefix, "/ru/visa/spain");
        assert_eq!(scope.canonical_prefix, "/ru/visa/spain/tourist");
        assert_eq!(scope.country_hub_slug, "spain-visa");
        assert_eq!(scope.visa_hub_slug, "spain-tourist-visa");
        let fallback = scope_paths(None);
        assert_eq!(fallback.country_prefix, "/und/visa/all");
        assert_eq!(fallback.canonical_prefix, "/und/visa/all/all");
        assert_eq!(fallback.country_hub_slug, "visa-country-hub");
        assert_eq!(fallback.visa_hub_slug, "visa-hub");
    }

    #[test]
    fn candidate_paths_and_sections_are_pure_domain_decisions() {
        let outcome = plan_cluster(&cluster(
            "Стоимость визы для семьи",
            Some("Стоимость визы для семьи"),
        ));
        let IaClusterDisposition::Plan(plan) = outcome else {
            panic!("expected planned page")
        };
        assert_eq!(plan.page_type_key, "fee_page");
        assert!(plan
            .canonical_url_path
            .contains("/fees/стоимость-визы-для-семьи/"));
        assert_eq!(plan.dominant_intent, "informational");
        assert_eq!(plan.lifecycle, IaLifecycle::Planned);
        assert_eq!(
            required_sections_for_page_type(&plan.page_type_key),
            vec!["overview", "fees", "faq", "related_pages", "cta_disclaimer"]
        );
    }

    #[test]
    fn broad_verified_query_reuses_hub_without_new_page() {
        let mut candidate = cluster(
            "документы стоимость сроки",
            Some("документы стоимость сроки"),
        );
        candidate.has_verified_support = true;
        assert!(matches!(
            plan_cluster(&candidate),
            IaClusterDisposition::UseExistingHub
        ));
    }

    #[test]
    fn source_discovery_and_unresolved_identifiers_remain_blocked() {
        let mut source = cluster(
            "site:example.org visa requirements",
            Some("site:example.org visa requirements"),
        );
        source.has_verified_support = true;
        let IaClusterDisposition::Plan(plan) = plan_cluster(&source) else {
            panic!("expected blocked page")
        };
        assert_eq!(plan.lifecycle, IaLifecycle::Blocked);

        let opaque_identifier = "a".repeat(64);
        let IaClusterDisposition::Plan(unresolved) =
            plan_cluster(&cluster(&opaque_identifier, None))
        else {
            panic!("expected blocked page")
        };
        assert_eq!(unresolved.lifecycle, IaLifecycle::Blocked);
        assert_eq!(unresolved.canonical_slug, "needs-editorial-review");
    }

    #[test]
    fn weak_multi_topic_families_require_review() {
        let topics = vec!["topic_a".to_string(), "topic_b".to_string()];
        let mut candidate = cluster("visa support", Some("visa support"));
        candidate.topic_keys = &topics;
        candidate.graph_confidence = 0.4;
        let IaClusterDisposition::Plan(plan) = plan_cluster(&candidate) else {
            panic!("expected blocked page")
        };
        assert_eq!(plan.topic_family_key, "topic_a|topic_b");
        assert_eq!(plan.lifecycle, IaLifecycle::Blocked);
    }
}
