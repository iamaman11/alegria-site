use contracts::generated::alegria::temporal::v1::{
    CannibalizationConflictState, IaBuildInputPayload, IaBuildOutputPayload, PageBlueprintState,
    PageNodeState,
};

use crate::seo_step_support::{artifact_key, scope_signature, slug};

fn locale_lang(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("und")
        .to_ascii_lowercase()
}

// A stable storage key or SHA digest is not a publishable semantic URL.
// An existing topic key may supply a readable alternative; otherwise the
// page stays blocked until a reviewer supplies meaningful topic metadata.
fn opaque_keyword(value: &str) -> bool {
    let trimmed = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    let hex = trimmed.replace('-', "");
    hex.len() >= 32 && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn semantic_keyword<'a>(seed: &'a str, topic_keys: &'a [String]) -> Option<&'a str> {
    if !seed.trim().is_empty() && !opaque_keyword(seed) {
        return Some(seed);
    }
    topic_keys
        .iter()
        .map(String::as_str)
        .find(|topic| !topic.trim().is_empty() && !opaque_keyword(topic))
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

fn required_sections_for_page_type(page_type: &str) -> Vec<String> {
    let roles: &[&str] = match page_type {
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
            "who_fits",
            "documents",
            "process",
            "where_to_apply",
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

fn canonical_prefix(input: &IaBuildInputPayload) -> String {
    let Some(scope) = input.scope.as_ref() else {
        return "/und/visa/all/all".to_string();
    };
    format!(
        "/{}/visa/{}/{}",
        locale_lang(&scope.locale),
        country_slug(&scope.country_code),
        slug(&scope.visa_type)
    )
}

fn country_prefix(input: &IaBuildInputPayload) -> String {
    let Some(scope) = input.scope.as_ref() else {
        return "/und/visa/all".to_string();
    };
    format!(
        "/{}/visa/{}",
        locale_lang(&scope.locale),
        country_slug(&scope.country_code)
    )
}

fn push_blueprint(
    page_blueprints: &mut Vec<PageBlueprintState>,
    page_type_key: &str,
    dominant_intent: &str,
    scope_class: &str,
    canonical_url_family: &str,
) -> String {
    let blueprint_key = artifact_key(
        "page_blueprint",
        &[page_type_key, dominant_intent, scope_class, "1"],
    );
    if !page_blueprints
        .iter()
        .any(|blueprint| blueprint.blueprint_key == blueprint_key)
    {
        page_blueprints.push(PageBlueprintState {
            blueprint_key: blueprint_key.clone(),
            page_type_key: page_type_key.to_string(),
            dominant_intent: dominant_intent.to_string(),
            scope_class: scope_class.to_string(),
            blueprint_version: 1,
            status: "active".to_string(),
            required_sections: required_sections_for_page_type(page_type_key),
            title_pattern: format!("{{country}} {{visa_type}} {page_type_key}"),
            canonical_url_family: canonical_url_family.to_string(),
        });
    }
    blueprint_key
}

pub fn execute(input: &IaBuildInputPayload) -> IaBuildOutputPayload {
    let fallback_scope = scope_signature(input.scope.as_ref());
    let mut page_nodes = Vec::new();
    let mut page_blueprints = Vec::new();
    let mut canonical_owner: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut cannibalization_conflicts = Vec::new();
    let mut seen_topic_families = std::collections::BTreeMap::<String, String>::new();

    let country_prefix = country_prefix(input);
    let canonical_prefix = canonical_prefix(input);
    let country_segment = input
        .scope
        .as_ref()
        .map(|scope| country_slug(&scope.country_code))
        .unwrap_or_else(|| "all".to_string());
    let country_hub_slug = input
        .scope
        .as_ref()
        .map(|scope| slug(&format!("{} visa", country_slug(&scope.country_code))))
        .unwrap_or_else(|| "visa-country-hub".to_string());
    let visa_hub_slug = input
        .scope
        .as_ref()
        .map(|scope| {
            slug(&format!(
                "{} {} visa",
                country_slug(&scope.country_code),
                scope.visa_type
            ))
        })
        .unwrap_or_else(|| "visa-hub".to_string());

    let country_blueprint_key = push_blueprint(
        &mut page_blueprints,
        "country_hub_page",
        "informational",
        "country_visa_scope",
        "visa_country_silo",
    );
    let country_page_node_key = artifact_key(
        "page_node",
        &[
            &fallback_scope,
            "country_hub_page",
            "informational",
            &country_hub_slug,
        ],
    );
    page_nodes.push(PageNodeState {
        page_node_key: country_page_node_key.clone(),
        scope_signature: fallback_scope.clone(),
        keyword_cluster_key: String::new(),
        blueprint_key: country_blueprint_key,
        page_type_key: "country_hub_page".to_string(),
        dominant_intent: "informational".to_string(),
        canonical_slug: country_hub_slug,
        canonical_url_path: format!("{country_prefix}/"),
        lifecycle_state: "planned".to_string(),
        parent_page_node_key: String::new(),
        hierarchy_depth: 2,
        menu_group: format!("visa:{country_segment}"),
        breadcrumb_policy: "path_segments".to_string(),
        canonical_url_family: "visa_country_silo".to_string(),
    });

    let hub_blueprint_key = push_blueprint(
        &mut page_blueprints,
        "hub_page",
        "informational",
        "visa_scope",
        "visa_type_silo",
    );
    let hub_page_node_key = artifact_key(
        "page_node",
        &[&fallback_scope, "hub_page", "informational", &visa_hub_slug],
    );
    page_nodes.push(PageNodeState {
        page_node_key: hub_page_node_key.clone(),
        scope_signature: fallback_scope.clone(),
        keyword_cluster_key: String::new(),
        blueprint_key: hub_blueprint_key,
        page_type_key: "hub_page".to_string(),
        dominant_intent: "informational".to_string(),
        canonical_slug: visa_hub_slug,
        canonical_url_path: format!("{canonical_prefix}/"),
        lifecycle_state: "planned".to_string(),
        parent_page_node_key: country_page_node_key,
        hierarchy_depth: 3,
        menu_group: format!("visa:{country_segment}"),
        breadcrumb_policy: "path_segments".to_string(),
        canonical_url_family: "visa_type_silo".to_string(),
    });

    for cluster in &input.keyword_clusters {
        let scope = if cluster.scope_signature.is_empty() {
            fallback_scope.clone()
        } else {
            cluster.scope_signature.clone()
        };
        let graph_confidence = cluster.graph_confidence;
        let ambiguous_topic_family = cluster.topic_keys.len() > 1 && graph_confidence < 0.6;
        // Source-discovery operators must never become publishable landing pages.
        let source_discovery_query = cluster
            .seed_keyword
            .split_whitespace()
            .any(|term| term.to_ascii_lowercase().starts_with("site:"));
        let topic_family_key = if cluster.topic_keys.is_empty() {
            cluster.seed_keyword.clone()
        } else {
            cluster.topic_keys.join("|")
        };
        let semantic_seed = semantic_keyword(&cluster.seed_keyword, &cluster.topic_keys);
        let canonical_slug = semantic_seed
            .map(slug)
            .unwrap_or_else(|| "needs-editorial-review".to_string());
        let page_type_key = semantic_seed
            .map(page_type_for_keyword)
            .unwrap_or("detail_page")
            .to_string();
        let path_segment = page_type_path_segment(&page_type_key, &canonical_slug);
        let canonical_url_path = format!("{canonical_prefix}/{path_segment}/");
        let dominant_intent = if cluster.dominant_intent.is_empty() {
            "informational".to_string()
        } else {
            cluster.dominant_intent.clone()
        };
        let blueprint_key = push_blueprint(
            &mut page_blueprints,
            &page_type_key,
            &dominant_intent,
            "visa_scope",
            "visa_type_leaf",
        );
        let page_node_key = artifact_key(
            "page_node",
            &[&scope, &page_type_key, &dominant_intent, &canonical_slug],
        );
        if let Some(existing_owner) =
            seen_topic_families.insert(topic_family_key.clone(), page_node_key.clone())
        {
            cannibalization_conflicts.push(CannibalizationConflictState {
                conflict_key: artifact_key(
                    "cannibalization_conflict",
                    &[
                        &scope,
                        &existing_owner,
                        &page_node_key,
                        "duplicate_topic_family",
                    ],
                ),
                scope_signature: scope.clone(),
                page_key_a: existing_owner,
                page_key_b: page_node_key.clone(),
                conflict_reason: "duplicate_topic_family".to_string(),
                severity: "high".to_string(),
                status: "open".to_string(),
            });
            continue;
        }

        if let Some(existing_owner) =
            canonical_owner.insert(canonical_url_path.clone(), page_node_key.clone())
        {
            cannibalization_conflicts.push(CannibalizationConflictState {
                conflict_key: artifact_key(
                    "cannibalization_conflict",
                    &[
                        &scope,
                        &existing_owner,
                        &page_node_key,
                        "duplicate_canonical_url",
                    ],
                ),
                scope_signature: scope.clone(),
                page_key_a: existing_owner,
                page_key_b: page_node_key.clone(),
                conflict_reason: "duplicate_canonical_url".to_string(),
                severity: "blocking".to_string(),
                status: "open".to_string(),
            });
        }
        page_nodes.push(PageNodeState {
            page_node_key,
            scope_signature: scope,
            keyword_cluster_key: cluster.cluster_key.clone(),
            blueprint_key,
            page_type_key,
            dominant_intent,
            canonical_slug,
            canonical_url_path,
            lifecycle_state: if ambiguous_topic_family
                || source_discovery_query
                || semantic_seed.is_none()
            {
                "blocked".to_string()
            } else {
                "planned".to_string()
            },
            parent_page_node_key: hub_page_node_key.clone(),
            hierarchy_depth: 4,
            menu_group: format!("visa:{country_segment}"),
            breadcrumb_policy: "path_segments".to_string(),
            canonical_url_family: "visa_type_leaf".to_string(),
        });
    }

    IaBuildOutputPayload {
        page_nodes,
        page_blueprints,
        cannibalization_conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_topic_family_does_not_create_duplicate_page_nodes() {
        let output = execute(&IaBuildInputPayload {
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    locale: "ru-RU".to_string(),
                    country_code: "ES".to_string(),
                    visa_type: "tourist".to_string(),
                    scope_signature: "scope".to_string(),
                    ..Default::default()
                },
            ),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-a".to_string(),
                    scope_signature: "scope".to_string(),
                    seed_keyword: "visa refusal guide".to_string(),
                    dominant_intent: "informational".to_string(),
                    topic_keys: vec!["visa_refusal_pain_point".to_string()],
                    graph_confidence: 0.9,
                    ..Default::default()
                },
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-b".to_string(),
                    scope_signature: "scope".to_string(),
                    seed_keyword: "visa refusal appeal".to_string(),
                    dominant_intent: "informational".to_string(),
                    topic_keys: vec!["visa_refusal_pain_point".to_string()],
                    graph_confidence: 0.91,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let detail_pages = output
            .page_nodes
            .iter()
            .filter(|page| page.page_type_key == "detail_page")
            .count();
        assert_eq!(detail_pages, 1);
        assert_eq!(output.cannibalization_conflicts.len(), 1);
    }

    #[test]
    fn unicode_keyword_produces_human_slug_and_research_query_is_blocked() {
        let output = execute(&IaBuildInputPayload {
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    locale: "ru-RU".into(),
                    country_code: "ES".into(),
                    visa_type: "tourist".into(),
                    scope_signature: "scope".into(),
                    ..Default::default()
                },
            ),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-1".into(),
                    scope_signature: "scope".into(),
                    seed_keyword: "Стоимость визы в Испанию для белорусов".into(),
                    ..Default::default()
                },
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-2".into(),
                    scope_signature: "scope".into(),
                    seed_keyword: "site:exteriores.gob.es визовая страница".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let fee = output
            .page_nodes
            .iter()
            .find(|node| node.keyword_cluster_key == "cluster-1")
            .unwrap();
        assert_eq!(fee.page_type_key, "fee_page");
        assert!(fee.canonical_url_path.contains("/fees/"));
        assert_eq!(fee.lifecycle_state, "planned");
        assert!(fee.canonical_slug.contains("стоимость"));
        let source = output
            .page_nodes
            .iter()
            .find(|node| node.keyword_cluster_key == "cluster-2")
            .unwrap();
        assert_eq!(source.lifecycle_state, "blocked");
    }

    #[test]
    fn specialized_pages_keep_distinct_canonical_paths_per_topic() {
        let output = execute(&IaBuildInputPayload {
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    locale: "ru-RU".into(),
                    country_code: "ES".into(),
                    visa_type: "tourist".into(),
                    scope_signature: "scope".into(),
                    ..Default::default()
                },
            ),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-documents".into(),
                    scope_signature: "scope".into(),
                    seed_keyword: "Документы на туристическую визу".into(),
                    dominant_intent: "informational".into(),
                    ..Default::default()
                },
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-requirements".into(),
                    scope_signature: "scope".into(),
                    seed_keyword: "Visa requirements for tourists".into(),
                    dominant_intent: "informational".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let pages = output
            .page_nodes
            .iter()
            .filter(|node| node.page_type_key == "requirement_page")
            .collect::<Vec<_>>();
        assert_eq!(pages.len(), 2);
        assert_ne!(pages[0].canonical_url_path, pages[1].canonical_url_path);
        assert!(pages
            .iter()
            .all(|page| page.canonical_url_path.contains("/requirements/")));
        assert!(output.cannibalization_conflicts.is_empty());
    }

    #[test]
    fn opaque_seed_uses_existing_semantic_topic_for_url() {
        let digest = "f".repeat(64);
        let output = execute(&IaBuildInputPayload {
            keyword_clusters: vec![contracts::generated::alegria::temporal::v1::KeywordClusterState {
                cluster_key: "cluster-opaque".to_string(),
                seed_keyword: digest.clone(),
                topic_keys: vec!["visa_refusal_appeal".to_string()],
                graph_confidence: 0.9,
                ..Default::default()
            }],
            ..Default::default()
        });
        let page = output.page_nodes.iter().find(|node| node.keyword_cluster_key == "cluster-opaque").unwrap();
        assert!(!page.canonical_url_path.contains(&digest));
        assert!(page.canonical_slug.contains("visa"));
        assert_eq!(page.lifecycle_state, "planned");
    }

    #[test]
    fn opaque_seed_without_semantic_topic_cannot_be_planned_for_publication() {
        let digest = "a".repeat(64);
        let output = execute(&IaBuildInputPayload {
            keyword_clusters: vec![contracts::generated::alegria::temporal::v1::KeywordClusterState {
                cluster_key: "cluster-unresolved".to_string(),
                seed_keyword: digest.clone(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let page = output.page_nodes.iter().find(|node| node.keyword_cluster_key == "cluster-unresolved").unwrap();
        assert!(!page.canonical_url_path.contains(&digest));
        assert_eq!(page.lifecycle_state, "blocked");
    }

    #[test]
    fn ambiguous_topic_family_is_blocked() {
        let output = execute(&IaBuildInputPayload {
            scope: Some(
                contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    locale: "ru-RU".to_string(),
                    country_code: "ES".to_string(),
                    visa_type: "tourist".to_string(),
                    scope_signature: "scope".to_string(),
                    ..Default::default()
                },
            ),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-a".to_string(),
                    scope_signature: "scope".to_string(),
                    seed_keyword: "visa support".to_string(),
                    dominant_intent: "informational".to_string(),
                    topic_keys: vec!["topic_a".to_string(), "topic_b".to_string()],
                    graph_confidence: 0.4,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        assert!(output.page_nodes.iter().any(
            |page| page.keyword_cluster_key == "cluster-a" && page.lifecycle_state == "blocked"
        ));
    }
}
