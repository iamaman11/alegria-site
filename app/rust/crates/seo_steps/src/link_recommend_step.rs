use contracts::generated::alegria::temporal::v1::{
    GraphPlanningContextState, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    LinkRecommendationState,
};

use crate::seo_step_support::{artifact_key, score};

fn link_kind(
    source_page_type: &str,
    target_page_type: &str,
) -> Option<(&'static str, &'static str, bool, f64)> {
    match (source_page_type, target_page_type) {
        ("country_hub_page", "hub_page") => {
            Some(("country_visa_hub", "hub_contextual", true, 0.98))
        }
        ("hub_page", "country_hub_page") => {
            Some(("visa_country_hub", "child_contextual", true, 0.97))
        }
        ("hub_page", other) if other != "hub_page" => {
            Some(("hub_child", "hub_contextual", true, 0.96))
        }
        (other, "hub_page") if other != "hub_page" => {
            Some(("child_hub", "child_contextual", true, 0.94))
        }
        ("overview_page", "detail_page") => {
            Some(("overview_detail", "journey_next_step", true, 0.9))
        }
        ("faq_page", _) | ("checklist_page", _) => {
            Some(("source_owner", "faq_source_owner", true, 0.88))
        }
        _ => Some(("contextual", "descriptive", false, 0.62)),
    }
}

fn linkable_state(state: &str) -> bool {
    !matches!(state, "blocked" | "deprecated" | "stale" | "needs_rebuild")
}

fn semantic_adjacency(source_path: &str, target_path: &str) -> f64 {
    let source_tokens: std::collections::BTreeSet<&str> = source_path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let target_tokens: std::collections::BTreeSet<&str> = target_path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if source_tokens.is_empty() || target_tokens.is_empty() {
        return 0.4;
    }
    let overlap = source_tokens.intersection(&target_tokens).count() as f64;
    let union = source_tokens.union(&target_tokens).count() as f64;
    overlap / union
}

fn topic_overlap(source_topics: &[String], target_topics: &[String]) -> usize {
    let source = source_topics
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let target = target_topics
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    source.intersection(&target).count()
}

fn cluster_topics_by_key(
    graph_context: Option<&GraphPlanningContextState>,
) -> std::collections::BTreeMap<String, Vec<String>> {
    graph_context
        .map(|context| {
            context
                .keyword_clusters
                .iter()
                .map(|cluster| (cluster.cluster_key.clone(), cluster.topic_keys.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn execute(input: &LinkRecommendInputPayload) -> LinkRecommendOutputPayload {
    let max_links = if input.max_links_per_page == 0 {
        3
    } else {
        input.max_links_per_page as usize
    };
    let mut link_recommendations = Vec::new();
    let topics_by_cluster = cluster_topics_by_key(input.graph_context.as_ref());

    for source in &input.page_nodes {
        if !linkable_state(&source.lifecycle_state) {
            continue;
        }
        let mut candidates = Vec::new();
        for target in &input.page_nodes {
            if source.page_node_key == target.page_node_key {
                continue;
            }
            if !linkable_state(&target.lifecycle_state) {
                continue;
            }
            let Some((link_role, anchor_strategy, required_flag, base_score)) =
                link_kind(&source.page_type_key, &target.page_type_key)
            else {
                continue;
            };
            let same_scope = source.scope_signature == target.scope_signature;
            if !same_scope && required_flag {
                continue;
            }
            let semantic =
                semantic_adjacency(&source.canonical_url_path, &target.canonical_url_path);
            let same_family = source.canonical_url_family == target.canonical_url_family;
            let journey_bonus = if source.parent_page_node_key == target.page_node_key
                || target.parent_page_node_key == source.page_node_key
            {
                0.08
            } else {
                0.0
            };
            let family_bonus = if same_family { 0.04 } else { 0.0 };
            let source_topics = topics_by_cluster
                .get(&source.keyword_cluster_key)
                .cloned()
                .unwrap_or_default();
            let target_topics = topics_by_cluster
                .get(&target.keyword_cluster_key)
                .cloned()
                .unwrap_or_default();
            let shared_topic_count = topic_overlap(&source_topics, &target_topics);
            let graph_bonus = if shared_topic_count > 0 { 0.08 } else { 0.0 };
            let link_score = score(if same_scope {
                (base_score * 0.68) + (semantic * 0.2) + journey_bonus + family_bonus + graph_bonus
            } else {
                (base_score * 0.42) + (semantic * 0.13) + (graph_bonus * 0.5)
            });
            candidates.push(LinkRecommendationState {
                link_recommendation_key: artifact_key(
                    "link_recommendation",
                    &[
                        &source.scope_signature,
                        &source.page_node_key,
                        &target.page_node_key,
                        link_role,
                        anchor_strategy,
                        "seo_link_score@1",
                    ],
                ),
                scope_signature: source.scope_signature.clone(),
                source_page_key: source.page_node_key.clone(),
                target_page_key: target.page_node_key.clone(),
                link_role: link_role.to_string(),
                anchor_strategy: anchor_strategy.to_string(),
                required_flag,
                score: link_score,
                status: "candidate".to_string(),
                reason_code: if shared_topic_count > 0 {
                    "graph_topic_adjacency".to_string()
                } else {
                    "structural_link_scoring".to_string()
                },
                topic_keys: source_topics
                    .iter()
                    .filter(|topic| target_topics.contains(topic))
                    .cloned()
                    .collect(),
                triple_refs: Vec::new(),
                graph_confidence: if shared_topic_count > 0 {
                    link_score
                } else {
                    0.0
                },
                support_refs: Vec::new(),
            });
        }
        // Graph affinity must influence WHICH optional links are selected,
        // not merely annotate whichever first pages happened to be in the input.
        let mut optional = candidates
            .iter()
            .filter(|link| !link.required_flag)
            .cloned()
            .collect::<Vec<_>>();
        optional.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.target_page_key.cmp(&b.target_page_key))
        });
        link_recommendations.extend(candidates.into_iter().filter(|link| link.required_flag));
        link_recommendations.extend(optional.into_iter().take(max_links));
    }

    LinkRecommendOutputPayload {
        link_recommendations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_adjacent_pages_gain_graph_reasoned_link() {
        let output = execute(&LinkRecommendInputPayload {
            max_links_per_page: 3,
            graph_context: Some(GraphPlanningContextState {
                keyword_clusters: vec![
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-a".to_string(),
                        topic_keys: vec!["visa_refusal_pain_point".to_string()],
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-b".to_string(),
                        topic_keys: vec!["visa_refusal_pain_point".to_string()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            page_nodes: vec![
                contracts::generated::alegria::temporal::v1::PageNodeState {
                    page_node_key: "source".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "hub_page".to_string(),
                    keyword_cluster_key: "cluster-a".to_string(),
                    canonical_url_path: "/visa/spain/refusal".to_string(),
                    canonical_url_family: "visa_type_leaf".to_string(),
                    lifecycle_state: "planned".to_string(),
                    ..Default::default()
                },
                contracts::generated::alegria::temporal::v1::PageNodeState {
                    page_node_key: "target".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "detail_page".to_string(),
                    keyword_cluster_key: "cluster-b".to_string(),
                    canonical_url_path: "/visa/spain/refusal-appeal".to_string(),
                    canonical_url_family: "visa_type_leaf".to_string(),
                    lifecycle_state: "planned".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        assert!(output.link_recommendations.iter().any(|link| {
            link.source_page_key == "source"
                && link.target_page_key == "target"
                && link.reason_code == "graph_topic_adjacency"
        }));
    }

    #[test]
    fn changing_graph_topic_adjacency_changes_selected_optional_link() {
        use contracts::generated::alegria::temporal::v1::{KeywordClusterState, PageNodeState};

        let page = |key: &str, cluster: &str, path: &str| PageNodeState {
            page_node_key: key.to_string(),
            scope_signature: "scope".to_string(),
            page_type_key: "detail_page".to_string(),
            keyword_cluster_key: cluster.to_string(),
            canonical_url_path: path.to_string(),
            canonical_url_family: "visa_type_leaf".to_string(),
            lifecycle_state: "planned".to_string(),
            ..Default::default()
        };
        let mut input = LinkRecommendInputPayload {
            max_links_per_page: 1,
            page_nodes: vec![
                page("source", "cluster-source", "/visas/spain/start"),
                page("alpha-other", "cluster-other", "/visas/spain/other"),
                page("zeta-related", "cluster-related", "/visas/spain/related"),
            ],
            graph_context: Some(GraphPlanningContextState {
                keyword_clusters: vec![
                    KeywordClusterState {
                        cluster_key: "cluster-source".to_string(),
                        topic_keys: vec!["shared_topic".to_string()],
                        ..Default::default()
                    },
                    KeywordClusterState {
                        cluster_key: "cluster-other".to_string(),
                        topic_keys: vec!["unrelated_topic".to_string()],
                        ..Default::default()
                    },
                    KeywordClusterState {
                        cluster_key: "cluster-related".to_string(),
                        topic_keys: vec!["shared_topic".to_string()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let first = execute(&input);
        let selected = first
            .link_recommendations
            .iter()
            .filter(|link| link.source_page_key == "source")
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].target_page_key, "zeta-related");
        assert_eq!(selected[0].reason_code, "graph_topic_adjacency");

        // With the graph relationship removed, the same paths and page order
        // select the stable structural candidate instead.
        input.graph_context = None;
        let without_graph = execute(&input);
        let selected = without_graph
            .link_recommendations
            .iter()
            .filter(|link| link.source_page_key == "source")
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].target_page_key, "alpha-other");

        input.page_nodes.swap(1, 2);
        let reordered = execute(&input);
        let selected = reordered
            .link_recommendations
            .iter()
            .find(|link| link.source_page_key == "source")
            .unwrap();
        assert_eq!(selected.target_page_key, "alpha-other");
    }
}
