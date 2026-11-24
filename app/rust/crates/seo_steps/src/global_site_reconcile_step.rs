use seo_domain::artifact_identity::ContentGapDetector;
use seo_domain::link_recommendation::{linkable_page, recommend_required_links};
use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::temporal::v1::{
    ContentGapState, GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload,
    LinkRecommendationState, PageNodeState,
};
use primitives::errors::DomainError;

pub fn execute(
    input: &GlobalSiteReconcileInputPayload,
) -> Result<GlobalSiteReconcileOutputPayload, DomainError> {
    // Only a transport mapper lives here. The link lifecycle/eligibility and
    // structural scoring have one owner in seo_domain::link_recommendation.
    let eligible_page = |page: &PageNodeState| {
        linkable_page(&crate::link_recommend_step::link_page_from_wire(page))
    };
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
            if !eligible_page(page) {
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
    let mut links = input.link_recommendations.clone();
    let link_pages = page_nodes
        .iter()
        .map(crate::link_recommend_step::link_page_from_wire)
        .collect::<Vec<_>>();
    let topics_by_cluster = input
        .graph_context
        .as_ref()
        .map(|context| {
            context
                .keyword_clusters
                .iter()
                .map(|cluster| (cluster.cluster_key.clone(), cluster.topic_keys.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    links.extend(
        recommend_required_links(&link_pages, &topics_by_cluster)
            .into_iter()
            .map(crate::link_recommend_step::wire_link),
    );

    let mut unique_links = BTreeMap::<(String, String), LinkRecommendationState>::new();
    for mut link in links {
        if !node_by_key
            .get(&link.source_page_key)
            .is_some_and(eligible_page)
            || !node_by_key
                .get(&link.target_page_key)
                .is_some_and(eligible_page)
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
    let mut deduped = unique_links.into_values().collect::<Vec<_>>();
    let active_link_count = deduped.len() as u32;

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

    // The same domain decision that selects current links and verified
    // evidence also retires superseded graph records. Persistence must write
    // this resolved result as-is, not decide which historical decisions are
    // still valid after the current plan has been assembled.
    if let Some(context) = input.graph_context.as_ref() {
        let current_keys = page_nodes
            .iter()
            .map(|page| page.page_node_key.as_str())
            .collect::<BTreeSet<_>>();
        let current_links = deduped
            .iter()
            .map(|link| link.link_recommendation_key.clone())
            .collect::<BTreeSet<_>>();
        for prior in &context.link_recommendations {
            if matches!(prior.status.as_str(), "candidate" | "accepted")
                && current_keys.contains(prior.source_page_key.as_str())
                && current_keys.contains(prior.target_page_key.as_str())
                && !current_links.contains(&prior.link_recommendation_key)
            {
                let mut expired = prior.clone();
                expired.status = "expired".into();
                deduped.push(expired);
            }
        }
        // Evidence-coverage decisions are eligible for resolution only when
        // page blueprints made factual completeness measurable in this run.
        if !input.page_blueprints.is_empty() {
            let identity = |gap: &ContentGapState| {
                ContentGapDetector::from_reason(&gap.reason_code).map(|detector| {
                    detector.key(&gap.scope_signature, &gap.page_node_key, &gap.missing_topic)
                })
            };
            let current_gaps = content_gaps
                .iter()
                .filter_map(identity)
                .collect::<BTreeSet<_>>();
            for prior in &context.content_gaps {
                if prior.status == "open"
                    && matches!(
                        prior.reason_code.as_str(),
                        "page_missing_verified_support" | "page_duplicate_verified_evidence"
                    )
                    && identity(prior).is_some_and(|key| !current_gaps.contains(&key))
                    && page_nodes.iter().any(|page| {
                        page.page_node_key == prior.page_node_key
                            && !matches!(
                                page.lifecycle_state.as_str(),
                                "blocked" | "deprecated" | "stale"
                            )
                    })
                {
                    let mut resolved = prior.clone();
                    resolved.status = "resolved".into();
                    content_gaps.push(resolved);
                }
            }
        }
    }

    Ok(GlobalSiteReconcileOutputPayload {
        page_nodes,
        updated_page_count,
        updated_link_count: active_link_count,
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
    use contracts::generated::alegria::temporal::v1::{
        GraphPlanningContextState, GraphPlanningCoverageSignalState, LinkRecommendInputPayload,
        PageBlueprintState, SeoVerifiedFactSupportState,
    };

    #[test]
    fn structural_links_have_one_domain_key_and_reconcile_respects_needs_rebuild() {
        let page = |key: &str, page_type: &str, state: &str| PageNodeState {
            page_node_key: key.into(),
            scope_signature: "scope".into(),
            page_type_key: page_type.into(),
            lifecycle_state: state.into(),
            canonical_slug: key.into(),
            canonical_url_path: format!("/visa/{key}/"),
            ..Default::default()
        };
        let pages = vec![
            page("hub", "hub_page", "planned"),
            page("detail", "detail_page", "planned"),
        ];
        let recommendations = crate::link_recommend_step::execute(&LinkRecommendInputPayload {
            page_nodes: pages.clone(),
            ..Default::default()
        })
        .link_recommendations;
        let result = execute(&GlobalSiteReconcileInputPayload {
            page_nodes: pages.clone(),
            link_recommendations: recommendations.clone(),
            ..Default::default()
        })
        .unwrap();
        for expected in recommendations.iter().filter(|link| link.required_flag) {
            let canonical = result
                .link_recommendations
                .iter()
                .filter(|actual| {
                    actual.source_page_key == expected.source_page_key
                        && actual.target_page_key == expected.target_page_key
                })
                .collect::<Vec<_>>();
            assert_eq!(canonical.len(), 1);
            assert_eq!(
                canonical[0].link_recommendation_key,
                expected.link_recommendation_key
            );
            assert_eq!(canonical[0].link_role, expected.link_role);
            assert_eq!(canonical[0].anchor_strategy, expected.anchor_strategy);
        }
        assert!(result
            .link_recommendations
            .iter()
            .all(|link| link.reason_code != "global_site_reconcile"));

        let mut rebuilding = pages;
        rebuilding[1].lifecycle_state = "needs_rebuild".into();
        let blocked = execute(&GlobalSiteReconcileInputPayload {
            page_nodes: rebuilding,
            link_recommendations: recommendations,
            ..Default::default()
        })
        .unwrap();
        assert!(blocked.link_recommendations.is_empty());
        assert_eq!(blocked.updated_link_count, 0);
    }

    #[test]
    fn superseded_links_expire_in_domain_output_but_active_link_count_is_unchanged() {
        let current = LinkRecommendationState {
            link_recommendation_key: "current-link".into(),
            source_page_key: "a".into(),
            target_page_key: "b".into(),
            status: "candidate".into(),
            score: 0.9,
            ..Default::default()
        };
        let prior = LinkRecommendationState {
            link_recommendation_key: "prior-link".into(),
            status: "accepted".into(),
            ..current.clone()
        };
        let outside_scope = LinkRecommendationState {
            link_recommendation_key: "outside-scope".into(),
            target_page_key: "other".into(),
            ..prior.clone()
        };
        let output = execute(&GlobalSiteReconcileInputPayload {
            page_nodes: ["a", "b"]
                .into_iter()
                .map(|key| PageNodeState {
                    page_node_key: key.into(),
                    canonical_url_path: format!("/{key}/"),
                    lifecycle_state: "planned".into(),
                    ..Default::default()
                })
                .collect(),
            link_recommendations: vec![current],
            graph_context: Some(GraphPlanningContextState {
                link_recommendations: vec![prior, outside_scope],
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(output.updated_link_count, 1);
        assert!(output.link_recommendations.iter().any(|link| {
            link.link_recommendation_key == "current-link" && link.status == "candidate"
        }));
        assert!(output.link_recommendations.iter().any(|link| {
            link.link_recommendation_key == "prior-link" && link.status == "expired"
        }));
        assert!(output
            .link_recommendations
            .iter()
            .all(|link| link.link_recommendation_key != "outside-scope"));
    }

    #[test]
    fn restored_page_evidence_resolves_prior_gap_but_does_not_resolve_unrelated_gaps() {
        let previous = ContentGapState {
            content_gap_key: "previous-gap".into(),
            scope_signature: "scope".into(),
            page_node_key: "documents".into(),
            missing_topic: "documents".into(),
            status: "open".into(),
            reason_code: "page_missing_verified_support".into(),
            ..Default::default()
        };
        let unrelated = ContentGapState {
            content_gap_key: "unrelated-gap".into(),
            reason_code: "global_reconcile_missing_topic".into(),
            ..previous.clone()
        };
        let output = execute(&GlobalSiteReconcileInputPayload {
            page_nodes: vec![PageNodeState {
                page_node_key: "documents".into(),
                blueprint_key: "doc".into(),
                scope_signature: "scope".into(),
                canonical_url_path: "/documents/".into(),
                page_type_key: "detail_page".into(),
                lifecycle_state: "planned".into(),
                ..Default::default()
            }],
            page_blueprints: vec![PageBlueprintState {
                blueprint_key: "doc".into(),
                required_sections: vec!["overview".into(), "documents".into()],
                ..Default::default()
            }],
            verified_support: vec![SeoVerifiedFactSupportState {
                support_ref: "verified-document".into(),
                fragment_text: "Verified document requirements".into(),
                role_type: "document_required".into(),
                freshness_class: "fresh".into(),
                ..Default::default()
            }],
            graph_context: Some(GraphPlanningContextState {
                content_gaps: vec![previous, unrelated],
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(output.page_nodes[0].lifecycle_state, "planned");
        assert_eq!(output.content_gaps.len(), 1);
        assert_eq!(output.content_gaps[0].content_gap_key, "previous-gap");
        assert_eq!(output.content_gaps[0].status, "resolved");
    }

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
