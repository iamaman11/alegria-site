use contracts::generated::alegria::temporal::v1::{
    LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState,
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

pub fn execute(input: &LinkRecommendInputPayload) -> LinkRecommendOutputPayload {
    let max_links = if input.max_links_per_page == 0 {
        3
    } else {
        input.max_links_per_page as usize
    };
    let mut link_recommendations = Vec::new();

    for source in &input.page_nodes {
        if !linkable_state(&source.lifecycle_state) {
            continue;
        }
        let mut added = 0usize;
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
            if !required_flag && added >= max_links {
                break;
            }
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
            let link_score = score(if same_scope {
                (base_score * 0.68) + (semantic * 0.2) + journey_bonus + family_bonus
            } else {
                (base_score * 0.42) + (semantic * 0.13)
            });
            link_recommendations.push(LinkRecommendationState {
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
            });
            if !required_flag {
                added += 1;
            }
        }
    }

    LinkRecommendOutputPayload {
        link_recommendations,
    }
}
