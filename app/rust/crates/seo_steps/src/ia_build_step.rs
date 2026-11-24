use contracts::generated::alegria::temporal::v1::{
    CannibalizationConflictState, IaBuildInputPayload, IaBuildOutputPayload, PageBlueprintState,
    PageNodeState,
};
use primitives::errors::DomainError;

use crate::seo_step_support::{artifact_key, scope_signature};
use seo_domain::ia_planning::{self, IaClusterCandidate, IaClusterDisposition, IaScope};
use seo_domain::page_readiness::is_opaque_topic_identifier;

// A stable storage identifier is never a publishable topic. Resolve the
// runtime classifier at the boundary before passing semantic text to domain.
fn semantic_keyword<'a>(seed: &'a str, topic_keys: &'a [String]) -> Option<&'a str> {
    if !seed.trim().is_empty() && !is_opaque_topic_identifier(seed) {
        return Some(seed);
    }
    topic_keys
        .iter()
        .map(String::as_str)
        .find(|topic| !topic.trim().is_empty() && !is_opaque_topic_identifier(topic))
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
            required_sections: ia_planning::required_sections_for_page_type(page_type_key),
            title_pattern: format!("{{country}} {{visa_type}} {page_type_key}"),
            canonical_url_family: canonical_url_family.to_string(),
        });
    }
    blueprint_key
}

pub fn execute(input: &IaBuildInputPayload) -> Result<IaBuildOutputPayload, DomainError> {
    let fallback_scope = scope_signature(input.scope.as_ref())?;
    let mut page_nodes = Vec::new();
    let mut page_blueprints = Vec::new();
    let mut canonical_owner: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut cannibalization_conflicts = Vec::new();
    let mut seen_topic_families = std::collections::BTreeMap::<String, String>::new();

    let paths = ia_planning::scope_paths(input.scope.as_ref().map(|scope| IaScope {
        locale: &scope.locale,
        country_code: &scope.country_code,
        visa_type: &scope.visa_type,
    }));
    let country_prefix = &paths.country_prefix;
    let canonical_prefix = &paths.canonical_prefix;
    let country_segment = &paths.country_segment;
    let country_hub_slug = paths.country_hub_slug;
    let visa_hub_slug = paths.visa_hub_slug;

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
        // The wire adapter resolves opaque storage IDs into a semantic seed.
        // The domain owns page classification, hierarchy and disposition.
        let semantic_seed = semantic_keyword(&cluster.seed_keyword, &cluster.topic_keys);
        let IaClusterDisposition::Plan(plan) = ia_planning::plan_cluster(&IaClusterCandidate {
            seed_keyword: &cluster.seed_keyword,
            topic_keys: &cluster.topic_keys,
            semantic_keyword: semantic_seed,
            graph_confidence: cluster.graph_confidence,
            dominant_intent: &cluster.dominant_intent,
            has_verified_support: !input.verified_support.is_empty(),
            canonical_prefix,
        }) else {
            continue;
        };
        let ia_planning::IaClusterPlan {
            topic_family_key,
            canonical_slug,
            page_type_key,
            canonical_url_path,
            dominant_intent,
            lifecycle,
        } = plan;
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
            lifecycle_state: lifecycle.as_str().to_string(),
            parent_page_node_key: hub_page_node_key.clone(),
            hierarchy_depth: 4,
            menu_group: format!("visa:{country_segment}"),
            breadcrumb_policy: "path_segments".to_string(),
            canonical_url_family: "visa_type_leaf".to_string(),
        });
    }

    // Evidence-derived plans are not fabricated SERP keywords or ranks.
    for (page_type, factual_section, route) in ia_planning::EVIDENCE_DERIVED_PAGES {
        if page_nodes
            .iter()
            .any(|page| page.page_type_key == page_type && page.lifecycle_state == "planned")
            || !input.verified_support.iter().any(|entry| {
                entry.freshness_class == "fresh"
                    && !entry.support_ref.trim().is_empty()
                    && !entry.fragment_text.trim().is_empty()
                    && seo_domain::editorial::role_matches_section(
                        &entry.role_type,
                        factual_section,
                    )
            })
        {
            continue;
        }
        let blueprint_key = push_blueprint(
            &mut page_blueprints,
            page_type,
            "informational",
            "visa_scope",
            "visa_type_leaf",
        );
        let page_node_key = artifact_key(
            "page_node",
            &[&fallback_scope, page_type, "informational", route],
        );
        let canonical_url_path = format!("{canonical_prefix}/{route}/");
        if page_nodes
            .iter()
            .any(|page| page.canonical_url_path == canonical_url_path)
        {
            continue;
        }
        page_nodes.push(PageNodeState {
            page_node_key,
            scope_signature: fallback_scope.clone(),
            keyword_cluster_key: String::new(),
            blueprint_key,
            page_type_key: page_type.into(),
            dominant_intent: "informational".into(),
            canonical_slug: route.into(),
            canonical_url_path,
            lifecycle_state: "planned".into(),
            parent_page_node_key: hub_page_node_key.clone(),
            hierarchy_depth: 4,
            menu_group: format!("visa:{country_segment}"),
            breadcrumb_policy: "path_segments".into(),
            canonical_url_family: "visa_type_leaf".into(),
        });
    }
    Ok(IaBuildOutputPayload {
        page_nodes,
        page_blueprints,
        cannibalization_conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_topic_family_does_not_create_duplicate_page_nodes() {
        let output = execute(&IaBuildInputPayload {
            verified_support: Vec::new(),
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
        })
        .unwrap();
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
            verified_support: Vec::new(),
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
        })
        .unwrap();
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
            verified_support: Vec::new(),
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
        })
        .unwrap();
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
            verified_support: Vec::new(),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-opaque".to_string(),
                    seed_keyword: digest.clone(),
                    topic_keys: vec!["visa_refusal_appeal".to_string()],
                    graph_confidence: 0.9,
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
        let page = output
            .page_nodes
            .iter()
            .find(|node| node.keyword_cluster_key == "cluster-opaque")
            .unwrap();
        assert!(!page.canonical_url_path.contains(&digest));
        assert!(page.canonical_slug.contains("visa"));
        assert_eq!(page.lifecycle_state, "planned");
    }

    #[test]
    fn opaque_seed_without_semantic_topic_cannot_be_planned_for_publication() {
        let digest = "a".repeat(64);
        let output = execute(&IaBuildInputPayload {
            verified_support: Vec::new(),
            keyword_clusters: vec![
                contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: "cluster-unresolved".to_string(),
                    seed_keyword: digest.clone(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
        let page = output
            .page_nodes
            .iter()
            .find(|node| node.keyword_cluster_key == "cluster-unresolved")
            .unwrap();
        assert!(!page.canonical_url_path.contains(&digest));
        assert_eq!(page.lifecycle_state, "blocked");
    }

    #[test]
    fn ambiguous_topic_family_is_blocked() {
        let output = execute(&IaBuildInputPayload {
            verified_support: Vec::new(),
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
        })
        .unwrap();
        assert!(output.page_nodes.iter().any(
            |page| page.keyword_cluster_key == "cluster-a" && page.lifecycle_state == "blocked"
        ));
    }

    #[test]
    fn three_duplicate_topic_candidates_keep_original_conflict_owner_order() {
        let keywords = [
            "visa refusal guide",
            "visa refusal appeal",
            "visa refusal checklist",
        ];
        let clusters = keywords
            .into_iter()
            .enumerate()
            .map(
                |(index, seed)| contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: format!("cluster-{index}"),
                    seed_keyword: seed.into(),
                    topic_keys: vec!["visa_refusal".into()],
                    ..Default::default()
                },
            )
            .collect();
        let output = execute(&IaBuildInputPayload {
            keyword_clusters: clusters,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(output.page_nodes.len(), 3); // country + visa hubs + first topic
        assert_eq!(output.cannibalization_conflicts.len(), 2);
        assert_eq!(
            output.cannibalization_conflicts[0].conflict_reason,
            "duplicate_topic_family"
        );
        // A suppressed second page still becomes the later conflict owner;
        // preserve the established observable identity and ordering.
        assert_eq!(
            output.cannibalization_conflicts[1].page_key_a,
            output.cannibalization_conflicts[0].page_key_b
        );
    }

    #[test]
    fn two_topic_families_with_one_canonical_path_keep_blocking_conflict() {
        let clusters = ["family_a", "family_b"]
            .into_iter()
            .map(
                |family| contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: family.into(),
                    seed_keyword: "visa fees".into(),
                    topic_keys: vec![family.into()],
                    ..Default::default()
                },
            )
            .collect();
        let output = execute(&IaBuildInputPayload {
            keyword_clusters: clusters,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(output.page_nodes.len(), 4);
        assert_eq!(output.cannibalization_conflicts.len(), 1);
        let conflict = &output.cannibalization_conflicts[0];
        assert_eq!(conflict.conflict_reason, "duplicate_canonical_url");
        assert_eq!(conflict.severity, "blocking");
        assert_eq!(conflict.status, "open");
    }
}

#[cfg(test)]
mod evidence_derived_ia_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        KeywordClusterState, SeoScopePayload, SeoVerifiedFactSupportState,
    };

    #[test]
    fn broad_query_becomes_hub_and_evidence_creates_readable_focused_paths() {
        let support = |role: &str| SeoVerifiedFactSupportState {
            support_ref: role.into(),
            role_type: role.into(),
            fragment_text: "Verified example".into(),
            freshness_class: "fresh".into(),
            ..Default::default()
        };
        let output = execute(&IaBuildInputPayload {
            scope: Some(SeoScopePayload {
                country_code: "ES".into(),
                visa_type: "tourist".into(),
                locale: "ru-RU".into(),
                ..Default::default()
            }),
            keyword_clusters: vec![KeywordClusterState {
                seed_keyword: "виза документы стоимость сроки".into(),
                cluster_key: "cluster".into(),
                ..Default::default()
            }],
            verified_support: vec![
                support("document_required"),
                support("fee_item"),
                support("timeline_item"),
            ],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(output.page_nodes.len(), 5);
        assert!(output
            .page_nodes
            .iter()
            .any(|page| page.canonical_url_path.ends_with("/documents/")));
        assert!(output
            .page_nodes
            .iter()
            .any(|page| page.canonical_url_path.ends_with("/fees/")));
        assert!(output
            .page_nodes
            .iter()
            .any(|page| page.canonical_url_path.ends_with("/processing-times/")));
        assert!(!output
            .page_nodes
            .iter()
            .any(|page| page.keyword_cluster_key == "cluster"));
    }
}
