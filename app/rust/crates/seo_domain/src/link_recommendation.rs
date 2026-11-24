//! SEO link selection. Temporal page/graph payloads are decoded by the caller;
//! this module owns structural priority, semantic affinity and link budgets.

use std::collections::{BTreeMap, BTreeSet};

use crate::artifact_identity::{clamp_score, seo_artifact_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPage {
    pub key: String,
    pub scope: String,
    pub page_type: String,
    pub lifecycle_state: String,
    pub canonical_slug: String,
    pub url_path: String,
    pub url_family: String,
    pub parent_key: String,
    pub cluster_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecommendedLink {
    pub key: String,
    pub scope: String,
    pub source_key: String,
    pub target_key: String,
    pub role: &'static str,
    pub anchor_strategy: &'static str,
    pub required: bool,
    pub score: f64,
    pub status: &'static str,
    pub reason: &'static str,
    pub shared_topics: Vec<String>,
    pub graph_confidence: f64,
}

fn link_kind(
    source_page_type: &str,
    target_page_type: &str,
) -> (&'static str, &'static str, bool, f64) {
    match (source_page_type, target_page_type) {
        ("country_hub_page", "hub_page") => ("country_visa_hub", "hub_contextual", true, 0.98),
        ("hub_page", "country_hub_page") => ("visa_country_hub", "child_contextual", true, 0.97),
        ("hub_page", other) if other != "hub_page" => ("hub_child", "hub_contextual", true, 0.96),
        (other, "hub_page") if other != "hub_page" => ("child_hub", "child_contextual", true, 0.94),
        ("overview_page", "detail_page") => ("overview_detail", "journey_next_step", true, 0.9),
        ("faq_page", _) | ("checklist_page", _) => ("source_owner", "faq_source_owner", true, 0.88),
        _ => ("contextual", "descriptive", false, 0.62),
    }
}

/// Shared eligibility for structural and semantic link suggestions.
/// Reconciliation retains a separate policy for existing links.
pub fn linkable_state(state: &str) -> bool {
    !matches!(state, "blocked" | "deprecated" | "stale" | "needs_rebuild")
}

/// The single owner of link eligibility, shared by Link Recommend and
/// Global Site Reconcile. Opaque identity tokens are not public destinations.
pub fn linkable_page(page: &LinkPage) -> bool {
    linkable_state(&page.lifecycle_state)
        && !crate::page_readiness::is_opaque_topic_identifier(&page.canonical_slug)
        && !page
            .url_path
            .split('/')
            .any(crate::page_readiness::is_opaque_topic_identifier)
}

fn semantic_adjacency(source_path: &str, target_path: &str) -> f64 {
    let source_tokens: BTreeSet<&str> = source_path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let target_tokens: BTreeSet<&str> = target_path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if source_tokens.is_empty() || target_tokens.is_empty() {
        return 0.4;
    }
    source_tokens.intersection(&target_tokens).count() as f64
        / source_tokens.union(&target_tokens).count() as f64
}

// Shared scoring and stable identity; no other module synthesizes link keys.
fn recommendation_for_pair(
    source: &LinkPage,
    target: &LinkPage,
    topics_by_cluster: &BTreeMap<String, Vec<String>>,
) -> Option<RecommendedLink> {
    // Every link is materialized as a same-scope graph relationship. Earlier
    // policy allowed cross-scope *optional* candidates that the scoped Neo4j
    // owner correctly refuses; persisting them would leave endlessly pending
    // Outbox intents and block the canonical Temporal projection barrier.
    // Until an explicit cross-scope contract is researched, fail closed at the
    // producer boundary before any PostgreSQL Truth or Outbox write.
    if source.key == target.key
        || source.scope.trim().is_empty()
        || source.scope != target.scope
        || !linkable_page(source)
        || !linkable_page(target)
    {
        return None;
    }
    let (role, anchor_strategy, required, base_score) =
        link_kind(&source.page_type, &target.page_type);
    let semantic = semantic_adjacency(&source.url_path, &target.url_path);
    let journey_bonus = if source.parent_key == target.key || target.parent_key == source.key {
        0.08
    } else {
        0.0
    };
    let family_bonus = if source.url_family == target.url_family {
        0.04
    } else {
        0.0
    };
    let source_topics = topics_by_cluster
        .get(&source.cluster_key)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let target_topics = topics_by_cluster
        .get(&target.cluster_key)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let shared_topics = source_topics
        .iter()
        .filter(|topic| target_topics.contains(topic))
        .cloned()
        .collect::<Vec<_>>();
    let graph_bonus = if shared_topics.is_empty() { 0.0 } else { 0.08 };
    let score = clamp_score(
        (base_score * 0.68) + (semantic * 0.2) + journey_bonus + family_bonus + graph_bonus,
    );
    Some(RecommendedLink {
        key: seo_artifact_key(
            "link_recommendation",
            &[
                &source.scope,
                &source.key,
                &target.key,
                role,
                anchor_strategy,
                "seo_link_score@1",
            ],
        ),
        scope: source.scope.clone(),
        source_key: source.key.clone(),
        target_key: target.key.clone(),
        role,
        anchor_strategy,
        required,
        score,
        status: "candidate",
        reason: if shared_topics.is_empty() {
            "structural_link_scoring"
        } else {
            "graph_topic_adjacency"
        },
        graph_confidence: if shared_topics.is_empty() { 0.0 } else { score },
        shared_topics,
    })
}

/// Reconcile only mandatory links without generating, sorting or discarding
/// optional/semantic candidates. Both paths use the exact same policy/ID.
pub fn recommend_required_links(
    pages: &[LinkPage],
    topics_by_cluster: &BTreeMap<String, Vec<String>>,
) -> Vec<RecommendedLink> {
    let mut required = Vec::new();
    for source in pages.iter().filter(|page| linkable_page(page)) {
        for target in pages {
            if source.scope != target.scope
                || source.key == target.key
                || !link_kind(&source.page_type, &target.page_type).2
            {
                continue;
            }
            if let Some(link) = recommendation_for_pair(source, target, topics_by_cluster) {
                required.push(link);
            }
        }
    }
    required
}

/// Returns required structural links plus at most `max_optional_per_page`
/// optional links for each eligible source. Input order cannot decide ties.
pub fn recommend_links(
    pages: &[LinkPage],
    topics_by_cluster: &BTreeMap<String, Vec<String>>,
    max_optional_per_page: usize,
) -> Vec<RecommendedLink> {
    let max_optional = if max_optional_per_page == 0 {
        3
    } else {
        max_optional_per_page
    };
    let mut recommendations = Vec::new();
    for source in pages {
        if !linkable_page(source) {
            continue;
        }
        let candidates = pages
            .iter()
            .filter_map(|target| recommendation_for_pair(source, target, topics_by_cluster))
            .collect::<Vec<_>>();
        let mut optional = candidates
            .iter()
            .filter(|link| !link.required)
            .cloned()
            .collect::<Vec<_>>();
        optional.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.target_key.cmp(&b.target_key))
        });
        recommendations.extend(candidates.into_iter().filter(|link| link.required));
        recommendations.extend(optional.into_iter().take(max_optional));
    }
    recommendations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(key: &str, cluster: &str) -> LinkPage {
        LinkPage {
            key: key.into(),
            scope: "scope".into(),
            page_type: "detail_page".into(),
            lifecycle_state: "planned".into(),
            canonical_slug: key.into(),
            url_path: "/visa/spain".into(),
            url_family: "visa_type_leaf".into(),
            parent_key: String::new(),
            cluster_key: cluster.into(),
        }
    }

    #[test]
    fn graph_topic_affinity_selects_the_optional_link_and_excludes_blocked_pages() {
        let pages = vec![
            page("source", "common"),
            page("alpha", "different"),
            page("zeta", "common"),
        ];
        let topics = BTreeMap::from([
            ("common".into(), vec!["shared".into()]),
            ("different".into(), vec!["other".into()]),
        ]);
        let links = recommend_links(&pages, &topics, 1);
        let from_source = links
            .iter()
            .find(|link| link.source_key == "source")
            .unwrap();
        assert_eq!(from_source.target_key, "zeta");
        assert_eq!(from_source.reason, "graph_topic_adjacency");
        let mut blocked_pages = pages;
        blocked_pages[2].lifecycle_state = "blocked".into();
        assert_eq!(
            recommend_links(&blocked_pages, &topics, 1)[0].target_key,
            "alpha"
        );
        for state in ["blocked", "deprecated", "stale", "needs_rebuild"] {
            assert!(!linkable_state(state));
        }
        assert!(linkable_state("planned"));
        let mut opaque = page("opaque", "cluster");
        opaque.canonical_slug = "a".repeat(64);
        assert!(!linkable_page(&opaque));
    }

    #[test]
    fn required_links_are_scope_bound_and_ignore_optional_budget() {
        let mut hub = page("hub", "hub-cluster");
        hub.page_type = "hub_page".into();
        let mut other_scope = page("other", "other-cluster");
        other_scope.scope = "other_scope".into();
        let result = recommend_links(
            &[hub.clone(), page("detail", "detail"), other_scope],
            &BTreeMap::new(),
            1,
        );
        assert_eq!(
            result
                .iter()
                .filter(|link| link.source_key == "hub")
                .count(),
            1
        );
        assert!(result
            .iter()
            .any(|link| link.source_key == "hub" && link.required && link.target_key == "detail"));
    }

    #[test]
    fn optional_links_also_require_one_nonempty_scope_before_outbox_staging() {
        let source = page("source", "cluster");
        let same_scope = page("same", "cluster");
        let mut foreign = page("foreign", "cluster");
        foreign.scope = "another_scope".into();
        let links = recommend_links(
            &[source.clone(), same_scope.clone(), foreign.clone()],
            &BTreeMap::new(),
            3,
        );
        assert!(links.iter().all(|link| {
            link.scope == "scope"
                && link.source_key != foreign.key
                && link.target_key != foreign.key
        }));
        assert!(links.iter().any(|link| {
            link.source_key == source.key && link.target_key == same_scope.key && !link.required
        }));

        let mut unknown_scope_a = source;
        unknown_scope_a.scope.clear();
        let mut unknown_scope_b = same_scope;
        unknown_scope_b.scope.clear();
        assert!(
            recommend_links(&[unknown_scope_a, unknown_scope_b], &BTreeMap::new(), 3,).is_empty()
        );
    }

    #[test]
    fn reconcile_required_only_is_identical_to_link_recommend_policy() {
        let mut hub = page("hub", "hub-cluster");
        hub.page_type = "hub_page".into();
        let pages = vec![hub, page("detail", "detail-cluster")];
        let all = recommend_links(&pages, &BTreeMap::new(), 2)
            .into_iter()
            .filter(|link| link.required)
            .collect::<Vec<_>>();
        let required = recommend_required_links(&pages, &BTreeMap::new());
        assert_eq!(required, all);

        let mut rebuilding = pages;
        rebuilding[1].lifecycle_state = "needs_rebuild".into();
        assert!(recommend_required_links(&rebuilding, &BTreeMap::new()).is_empty());
    }
}
