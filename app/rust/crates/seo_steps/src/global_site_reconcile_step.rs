use seo_domain::artifact_identity::ContentGapDetector;
use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::temporal::v1::{
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, LinkRecommendationState,
    PageNodeState,
};
use primitives::errors::DomainError;

fn link_key(
    scope_signature: &str,
    source_page_key: &str,
    target_page_key: &str,
    link_role: &str,
    anchor_strategy: &str,
) -> String {
    seo_domain::artifact_identity::seo_artifact_key(
        "link_recommendation",
        &[
            scope_signature,
            source_page_key,
            target_page_key,
            link_role,
            anchor_strategy,
            "global_site_reconcile@1",
        ],
    )
}

fn page_is_linkable(page: &PageNodeState) -> bool {
    !matches!(
        page.lifecycle_state.as_str(),
        "blocked" | "deprecated" | "stale"
    ) && !seo_domain::page_readiness::is_opaque_topic_identifier(&page.canonical_slug)
        && !page
            .canonical_url_path
            .split('/')
            .any(seo_domain::page_readiness::is_opaque_topic_identifier)
}

fn required_link(
    source: &PageNodeState,
    target: &PageNodeState,
    link_role: &str,
    anchor_strategy: &str,
    score: f64,
) -> LinkRecommendationState {
    LinkRecommendationState {
        link_recommendation_key: link_key(
            &source.scope_signature,
            &source.page_node_key,
            &target.page_node_key,
            link_role,
            anchor_strategy,
        ),
        scope_signature: source.scope_signature.clone(),
        source_page_key: source.page_node_key.clone(),
        target_page_key: target.page_node_key.clone(),
        link_role: link_role.to_string(),
        anchor_strategy: anchor_strategy.to_string(),
        required_flag: true,
        score,
        status: "candidate".to_string(),
        reason_code: "global_site_reconcile".to_string(),
        topic_keys: Vec::new(),
        triple_refs: Vec::new(),
        graph_confidence: 0.0,
        support_refs: Vec::new(),
    }
}

pub fn execute(
    input: &GlobalSiteReconcileInputPayload,
) -> Result<GlobalSiteReconcileOutputPayload, DomainError> {
    let fallback_scope = crate::seo_step_support::scope_signature(input.scope.as_ref())?;
    let mut page_nodes = input.page_nodes.clone();
    let mut orphan_page_node_keys = Vec::new();
    let mut cannibalization_conflict_keys = Vec::new();
    let mut content_gaps = Vec::new();
    let mut updated_page_count = 0u32;

    let menu_group = page_nodes
        .iter()
        .find(|page| !page.menu_group.trim().is_empty())
        .map(|page| page.menu_group.clone())
        .or_else(|| {
            input.scope.as_ref().map(|scope| {
                format!(
                    "visa:{}",
                    if scope.country_code.trim().is_empty() {
                        "all"
                    } else {
                        scope.country_code.trim()
                    }
                    .to_ascii_lowercase()
                )
            })
        })
        .unwrap_or_else(|| "visa:all".to_string());

    let country_hub_key = page_nodes
        .iter()
        .find(|page| page.page_type_key == "country_hub_page")
        .map(|page| page.page_node_key.clone());
    let hub_key = page_nodes
        .iter()
        .find(|page| page.page_type_key == "hub_page")
        .map(|page| page.page_node_key.clone());

    for page in &mut page_nodes {
        let original = page.clone();
        if page.lifecycle_state.trim().is_empty() {
            page.lifecycle_state = "planned".to_string();
        }
        if page.menu_group.trim().is_empty() {
            page.menu_group = menu_group.clone();
        }
        if page.breadcrumb_policy.trim().is_empty() {
            page.breadcrumb_policy = "path_segments".to_string();
        }

        match page.page_type_key.as_str() {
            "country_hub_page" => {
                page.parent_page_node_key.clear();
                page.hierarchy_depth = 2;
                if page.canonical_url_family.trim().is_empty() {
                    page.canonical_url_family = "visa_country_silo".to_string();
                }
            }
            "hub_page" => {
                if let Some(country_hub_key) = &country_hub_key {
                    page.parent_page_node_key = country_hub_key.clone();
                } else if page.parent_page_node_key.trim().is_empty() {
                    orphan_page_node_keys.push(page.page_node_key.clone());
                }
                page.hierarchy_depth = 3;
                if page.canonical_url_family.trim().is_empty() {
                    page.canonical_url_family = "visa_type_silo".to_string();
                }
            }
            _ => {
                if let Some(hub_key) = &hub_key {
                    page.parent_page_node_key = hub_key.clone();
                } else if page.parent_page_node_key.trim().is_empty() {
                    orphan_page_node_keys.push(page.page_node_key.clone());
                }
                page.hierarchy_depth = 4;
                if page.canonical_url_family.trim().is_empty() {
                    page.canonical_url_family = "visa_type_leaf".to_string();
                }
            }
        }
        if *page != original {
            updated_page_count += 1;
        }
    }

    // Evaluate actual page-specific verified evidence after truth admission.
    // Planning-only diagnostics without supplied blueprints retain their
    // existing behavior; they cannot manufacture factual readiness.
    if !input.page_blueprints.is_empty() {
        // Hub and detail pages may cite the same verified facts while serving
        // different page roles. Only equal evidence *within the same page
        // type* is a duplicate-page signal.
        let mut evidence_owners = BTreeMap::<(String, Vec<String>), String>::new();
        for page in &mut page_nodes {
            if !page_is_linkable(page) {
                continue;
            }
            let blueprint = input
                .page_blueprints
                .iter()
                .find(|blueprint| blueprint.blueprint_key == page.blueprint_key);
            let (mut missing, mut refs) = if let Some(blueprint) = blueprint {
                let support =
                    crate::page_wire::page_verified_support(blueprint, &input.verified_support);
                (
                    crate::page_wire::missing_required_factual_roles(blueprint, &support),
                    support
                        .into_iter()
                        .map(|entry| entry.support_ref)
                        .collect::<Vec<_>>(),
                )
            } else {
                (vec!["page_blueprint".to_string()], Vec::new())
            };
            refs.sort();
            refs.dedup();
            let evidence_identity = (page.page_type_key.clone(), refs.clone());
            let duplicate = if missing.is_empty() && !refs.is_empty() {
                evidence_owners.get(&evidence_identity).cloned()
            } else {
                None
            };
            if missing.is_empty() && duplicate.is_none() && !refs.is_empty() {
                evidence_owners.insert(evidence_identity, page.page_node_key.clone());
                continue;
            }
            let detector = if duplicate.is_some() {
                missing.push("distinct_verified_evidence".into());
                ContentGapDetector::DuplicateVerifiedEvidence
            } else {
                if missing.is_empty() {
                    missing.push("verified_evidence".into());
                }
                ContentGapDetector::MissingVerifiedSupport
            };
            page.lifecycle_state = "blocked".to_string();
            for topic in missing {
                content_gaps.push(
                    contracts::generated::alegria::temporal::v1::ContentGapState {
                        content_gap_key: detector.key(
                            &page.scope_signature,
                            &page.page_node_key,
                            &topic,
                        ),
                        scope_signature: page.scope_signature.clone(),
                        page_node_key: page.page_node_key.clone(),
                        missing_topic: topic,
                        severity: if duplicate.is_some() {
                            "medium"
                        } else {
                            "high"
                        }
                        .into(),
                        status: "open".into(),
                        reason_code: detector.reason().into(),
                        topic_keys: Vec::new(),
                        triple_refs: Vec::new(),
                        graph_confidence: 0.0,
                        support_refs: refs.clone(),
                    },
                );
            }
        }
    }

    let mut canonical_owner = BTreeMap::new();
    for page in &page_nodes {
        if let Some(previous_owner) =
            canonical_owner.insert(page.canonical_url_path.clone(), page.page_node_key.clone())
        {
            cannibalization_conflict_keys.push(seo_domain::artifact_identity::seo_artifact_key(
                "cannibalization_conflict",
                &[
                    &page.scope_signature,
                    &previous_owner,
                    &page.page_node_key,
                    "duplicate_canonical_url",
                ],
            ));
        }
    }

    let node_by_key = page_nodes
        .iter()
        .map(|page| (page.page_node_key.clone(), page.clone()))
        .collect::<BTreeMap<_, _>>();
    let country_hub = country_hub_key
        .as_ref()
        .and_then(|key| node_by_key.get(key))
        .cloned();
    let hub = hub_key
        .as_ref()
        .and_then(|key| node_by_key.get(key))
        .cloned();

    let mut links = input.link_recommendations.clone();
    if let (Some(country_hub), Some(hub)) = (&country_hub, &hub) {
        if page_is_linkable(country_hub) && page_is_linkable(hub) {
            links.push(required_link(
                country_hub,
                hub,
                "country_hub_to_visa_hub",
                "hub_contextual",
                0.99,
            ));
            links.push(required_link(
                hub,
                country_hub,
                "visa_hub_to_country_hub",
                "breadcrumb_contextual",
                0.96,
            ));
        }
    }
    if let Some(hub) = &hub {
        for page in &page_nodes {
            if page.page_node_key == hub.page_node_key || !page_is_linkable(page) {
                continue;
            }
            if page.page_type_key == "country_hub_page" {
                continue;
            }
            links.push(required_link(
                hub,
                page,
                "hub_to_child",
                "section_contextual",
                0.97,
            ));
            links.push(required_link(
                page,
                hub,
                "child_to_hub",
                "navigation_contextual",
                0.95,
            ));
        }
    }

    let mut unique_links = BTreeMap::<(String, String), LinkRecommendationState>::new();
    for mut link in links {
        if !node_by_key
            .get(&link.source_page_key)
            .is_some_and(page_is_linkable)
            || !node_by_key
                .get(&link.target_page_key)
                .is_some_and(page_is_linkable)
            || link.source_page_key == link.target_page_key
            || matches!(link.status.as_str(), "deprecated" | "rejected" | "expired")
        {
            continue;
        }
        if link.status.trim().is_empty() {
            link.status = "candidate".to_string();
        }
        let key = (link.source_page_key.clone(), link.target_page_key.clone());
        let replace = unique_links.get(&key).is_none_or(|prior| {
            (link.required_flag && !prior.required_flag)
                || (link.required_flag == prior.required_flag
                    && (link.score.total_cmp(&prior.score).is_gt()
                        || (link.score == prior.score
                            && link.link_recommendation_key < prior.link_recommendation_key)))
        });
        if replace {
            unique_links.insert(key, link);
        }
    }
    let deduped = unique_links.into_values().collect::<Vec<_>>();

    if let Some(graph_context) = input.graph_context.as_ref() {
        let mut seen = BTreeSet::new();
        for coverage in &graph_context.coverage_signals {
            let scope = if coverage.page_node_key.is_empty() {
                if graph_context.scope_signature.trim().is_empty() {
                    fallback_scope.clone()
                } else {
                    graph_context.scope_signature.clone()
                }
            } else {
                let Some(page) = node_by_key.get(&coverage.page_node_key) else {
                    continue;
                };
                if matches!(page.lifecycle_state.as_str(), "deprecated" | "stale") {
                    continue;
                }
                page.scope_signature.clone()
            };
            for missing_topic in &coverage.missing_topic_keys {
                let detector = ContentGapDetector::ReconcileCoverage;
                let gap_key = detector.key(&scope, &coverage.page_node_key, missing_topic);
                if missing_topic.trim().is_empty() || !seen.insert(gap_key.clone()) {
                    continue;
                }
                content_gaps.push(
                    contracts::generated::alegria::temporal::v1::ContentGapState {
                        content_gap_key: gap_key,
                        scope_signature: scope.clone(),
                        page_node_key: coverage.page_node_key.clone(),
                        missing_topic: missing_topic.clone(),
                        severity: "medium".to_string(),
                        status: "open".to_string(),
                        reason_code: detector.reason().to_string(),
                        topic_keys: vec![missing_topic.clone()],
                        triple_refs: Vec::new(),
                        graph_confidence: coverage.graph_confidence,
                        support_refs: vec![format!("coverage://{}", coverage.page_node_key)],
                    },
                );
            }
        }
    }

    Ok(GlobalSiteReconcileOutputPayload {
        page_nodes,
        updated_page_count,
        updated_link_count: deduped.len() as u32,
        link_recommendations: deduped,
        orphan_page_node_keys,
        cannibalization_conflict_keys,
        status: "done".to_string(),
        content_gaps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::GraphPlanningCoverageSignalState;

    #[test]
    fn emits_graph_backed_missing_topic_gap_without_upgrading_page_state() {
        let output = execute(&GlobalSiteReconcileInputPayload {
            page_blueprints: Vec::new(),
            verified_support: Vec::new(),
            graph_context: Some(
                contracts::generated::alegria::temporal::v1::GraphPlanningContextState {
                    coverage_signals: vec![GraphPlanningCoverageSignalState {
                        page_node_key: "detail".to_string(),
                        keyword_cluster_key: "cluster-a".to_string(),
                        covered_topic_keys: vec!["topic_a".to_string()],
                        missing_topic_keys: vec!["topic_b".to_string()],
                        graph_confidence: 0.73,
                    }],
                    ..Default::default()
                },
            ),
            page_nodes: vec![
                PageNodeState {
                    page_node_key: "country".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "country_hub_page".to_string(),
                    lifecycle_state: "planned".to_string(),
                    ..Default::default()
                },
                PageNodeState {
                    page_node_key: "hub".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "hub_page".to_string(),
                    lifecycle_state: "planned".to_string(),
                    ..Default::default()
                },
                PageNodeState {
                    page_node_key: "detail".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "detail_page".to_string(),
                    lifecycle_state: "planned".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(output.content_gaps.len(), 1);
        assert_eq!(
            output.content_gaps[0].reason_code,
            "global_reconcile_missing_topic"
        );
        assert!(output
            .page_nodes
            .iter()
            .all(|page| page.lifecycle_state == "planned"));
    }
}

#[cfg(test)]
mod actual_coverage_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        PageBlueprintState, SeoVerifiedFactSupportState,
    };

    #[test]
    fn missing_and_duplicate_evidence_are_real_gaps_not_publishable_pages() {
        let page = |key: &str, blueprint: &str| PageNodeState {
            page_node_key: key.into(),
            blueprint_key: blueprint.into(),
            scope_signature: "scope".into(),
            page_type_key: "detail_page".into(),
            canonical_url_path: format!("/{key}/"),
            lifecycle_state: "planned".into(),
            ..Default::default()
        };
        let blueprint = |key: &str, role: &str| PageBlueprintState {
            blueprint_key: key.into(),
            required_sections: vec!["overview".into(), role.into()],
            ..Default::default()
        };
        let output = execute(&GlobalSiteReconcileInputPayload {
            page_nodes: vec![
                page("documents", "doc"),
                page("duplicate", "doc"),
                page("cost", "fee"),
                PageNodeState {
                    page_type_key: "hub_page".into(),
                    ..page("summary", "doc")
                },
            ],
            page_blueprints: vec![blueprint("doc", "documents"), blueprint("fee", "fees")],
            verified_support: vec![SeoVerifiedFactSupportState {
                support_ref: "verified".into(),
                fragment_text: "Document required".into(),
                role_type: "document_required".into(),
                freshness_class: "fresh".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            output
                .page_nodes
                .iter()
                .filter(|page| page.lifecycle_state == "blocked")
                .count(),
            2
        );
        assert!(output
            .content_gaps
            .iter()
            .any(|gap| gap.page_node_key == "cost" && gap.missing_topic == "fees"));
        assert!(output
            .content_gaps
            .iter()
            .any(|gap| gap.page_node_key == "duplicate"
                && gap.reason_code == "page_duplicate_verified_evidence"));
        assert!(output
            .page_nodes
            .iter()
            .any(|page| page.page_node_key == "summary" && page.lifecycle_state == "planned"));
        assert!(output
            .link_recommendations
            .iter()
            .all(|link| link.target_page_key != "duplicate" && link.target_page_key != "cost"));
    }
}
