use contracts::generated::alegria::temporal::v1::{
    LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState,
};
#[cfg(test)]
use contracts::generated::alegria::temporal::v1::GraphPlanningContextState;
use seo_domain::link_recommendation::{recommend_links, LinkPage};

/// Temporal protobuf adaptation. Link eligibility, scoring and prioritization
/// belong exclusively to the SEO domain.
pub fn execute(input: &LinkRecommendInputPayload) -> LinkRecommendOutputPayload {
    let pages = input
        .page_nodes
        .iter()
        .map(|page| LinkPage {
            key: page.page_node_key.clone(),
            scope: page.scope_signature.clone(),
            page_type: page.page_type_key.clone(),
            lifecycle_state: page.lifecycle_state.clone(),
            url_path: page.canonical_url_path.clone(),
            url_family: page.canonical_url_family.clone(),
            parent_key: page.parent_page_node_key.clone(),
            cluster_key: page.keyword_cluster_key.clone(),
        })
        .collect::<Vec<_>>();
    let topics_by_cluster = input
        .graph_context
        .as_ref()
        .map(|context| {
            context
                .keyword_clusters
                .iter()
                .map(|cluster| (cluster.cluster_key.clone(), cluster.topic_keys.clone()))
                .collect()
        })
        .unwrap_or_default();

    LinkRecommendOutputPayload {
        link_recommendations: recommend_links(
            &pages,
            &topics_by_cluster,
            input.max_links_per_page as usize,
        )
        .into_iter()
        .map(|link| LinkRecommendationState {
            link_recommendation_key: link.key,
            scope_signature: link.scope,
            source_page_key: link.source_key,
            target_page_key: link.target_key,
            link_role: link.role.to_string(),
            anchor_strategy: link.anchor_strategy.to_string(),
            required_flag: link.required,
            score: link.score,
            status: link.status.to_string(),
            reason_code: link.reason.to_string(),
            topic_keys: link.shared_topics,
            triple_refs: Vec::new(),
            graph_confidence: link.graph_confidence,
            support_refs: Vec::new(),
        })
        .collect(),
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
