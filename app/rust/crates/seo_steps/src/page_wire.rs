use contracts::generated::alegria::temporal::v1::{
    ContentGapState, LinkRecommendationState, PageBlueprintState, PageNodeState,
    SeoVerifiedFactSupportState,
};
use seo_domain::editorial::role_matches_section;
use seo_domain::page_readiness::{
    self, ContentGap, LinkRecommendation, PageAdmissibility, PageNode,
};
use std::collections::BTreeSet;

use crate::editorial_wire::support_views;

/// Select transport records using the one domain-owned applicability rule.
pub fn supports_for_role<'a>(
    support: &'a [SeoVerifiedFactSupportState],
    section_role: &str,
) -> Vec<&'a SeoVerifiedFactSupportState> {
    support
        .iter()
        .filter(|entry| role_matches_section(&entry.role_type, section_role))
        .collect()
}

pub fn missing_required_factual_roles(
    blueprint: &PageBlueprintState,
    support: &[SeoVerifiedFactSupportState],
) -> Vec<String> {
    page_readiness::missing_required_factual_roles(
        &blueprint.required_sections,
        &support_views(support),
    )
}

pub fn page_verified_support(
    blueprint: &PageBlueprintState,
    support: &[SeoVerifiedFactSupportState],
) -> Vec<SeoVerifiedFactSupportState> {
    page_readiness::selected_support_indices(&blueprint.required_sections, &support_views(support))
        .into_iter()
        .map(|index| support[index].clone())
        .collect()
}

pub fn page_admissibility(
    page: &PageNodeState,
    blueprint: &PageBlueprintState,
    support: &[SeoVerifiedFactSupportState],
    gaps: &[ContentGapState],
    requires_editorial: bool,
) -> PageAdmissibility {
    let gaps = gaps
        .iter()
        .map(|gap| ContentGap {
            page_node_key: &gap.page_node_key,
            status: &gap.status,
            reason_code: &gap.reason_code,
            missing_topic: &gap.missing_topic,
        })
        .collect::<Vec<_>>();
    page_readiness::page_admissibility(
        PageNode {
            page_node_key: &page.page_node_key,
            lifecycle_state: &page.lifecycle_state,
            canonical_slug: &page.canonical_slug,
            canonical_url_path: &page.canonical_url_path,
        },
        &blueprint.required_sections,
        &support_views(support),
        &gaps,
        requires_editorial,
    )
}

pub fn required_links_for_published_targets(
    links: &[LinkRecommendationState],
    source_page_key: &str,
    published: &BTreeSet<String>,
) -> Vec<LinkRecommendationState> {
    let decisions = links
        .iter()
        .map(|link| LinkRecommendation {
            source_page_key: &link.source_page_key,
            target_page_key: &link.target_page_key,
            link_recommendation_key: &link.link_recommendation_key,
            status: &link.status,
            required: link.required_flag,
            score: link.score,
        })
        .collect::<Vec<_>>();
    page_readiness::required_link_indices(&decisions, source_page_key, published)
        .into_iter()
        .map(|index| links[index].clone())
        .collect()
}
#[cfg(test)]
mod verified_page_readiness_tests {
    use super::*;
    use seo_domain::editorial::{heading_for_role_locale, locale_from_canonical_path};
    use seo_domain::page_readiness::is_opaque_topic_identifier;

    fn support(role_type: &str) -> SeoVerifiedFactSupportState {
        SeoVerifiedFactSupportState {
            role_type: role_type.to_string(),
            support_ref: "verified-rule-1".to_string(),
            fragment_text: "Exact original source quotation".to_string(),
            freshness_class: "fresh".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn russian_locale_headings_are_reusable_across_unrelated_topics() {
        assert_eq!(locale_from_canonical_path("/ru/visa/spain/"), "ru");
        assert_eq!(
            locale_from_canonical_path("/ru/gardening/plant-care/"),
            "ru"
        );
        assert_eq!(heading_for_role_locale("fees", "ru"), "Консульский сбор");
        assert_eq!(heading_for_role_locale("documents", "ru"), "Документы");
        assert_eq!(heading_for_role_locale("fees", "en"), "Fees");
        assert_eq!(heading_for_role_locale("unknown", "ru"), "Section");
    }
    #[test]
    fn opaque_sha_and_uuid_are_never_public_slugs() {
        assert!(is_opaque_topic_identifier(&"a".repeat(64)));
        assert!(is_opaque_topic_identifier(&format!(
            "sha256:{}",
            "9".repeat(64)
        )));
        assert!(is_opaque_topic_identifier(
            "123e4567-e89b-12d3-a456-426614174000"
        ));
        assert!(!is_opaque_topic_identifier("plant-leaf-care"));
        assert!(!is_opaque_topic_identifier("spain-tourist-visa"));
    }

    #[test]
    fn page_specific_readiness_does_not_demand_unrelated_visa_blocks() {
        let blueprint = PageBlueprintState {
            required_sections: vec![
                "overview".to_string(),
                "faq".to_string(),
                "related_pages".to_string(),
            ],
            ..Default::default()
        };
        assert!(
            missing_required_factual_roles(&blueprint, &[support("eligibility_rule")]).is_empty()
        );
    }

    #[test]
    fn missing_fee_support_blocks_only_page_requiring_fee() {
        let blueprint = PageBlueprintState {
            required_sections: vec!["overview".to_string(), "fees".to_string()],
            ..Default::default()
        };
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[support("eligibility_rule")]),
            vec!["fees".to_string()]
        );
    }

    #[test]
    fn empty_or_stale_support_cannot_satisfy_required_role() {
        let blueprint = PageBlueprintState {
            required_sections: vec!["fees".to_string()],
            ..Default::default()
        };
        let mut stale = support("fee_item");
        stale.freshness_class = "stale".to_string();
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[stale]),
            vec!["fees"]
        );
        let mut empty = support("fee_item");
        empty.fragment_text.clear();
        assert_eq!(
            missing_required_factual_roles(&blueprint, &[empty]),
            vec!["fees"]
        );
    }

    #[test]
    fn default_blueprint_requires_real_factual_section_support() {
        let missing = missing_required_factual_roles(
            &PageBlueprintState::default(),
            &[support("eligibility_rule")],
        );
        assert!(missing.contains(&"documents".to_string()));
        assert!(missing.contains(&"fees".to_string()));
        assert!(missing.contains(&"timing".to_string()));
    }
}

#[cfg(test)]
mod page_evidence_selection_tests {
    use super::*;

    #[test]
    fn specialized_page_does_not_receive_unrelated_truth() {
        let support = vec![
            SeoVerifiedFactSupportState {
                support_ref: "fee".into(),
                role_type: "fee_item".into(),
                fragment_text: "Fee amount".into(),
                freshness_class: "fresh".into(),
                ..Default::default()
            },
            SeoVerifiedFactSupportState {
                support_ref: "document".into(),
                role_type: "document_required".into(),
                fragment_text: "Document required".into(),
                freshness_class: "fresh".into(),
                ..Default::default()
            },
        ];
        let blueprint = PageBlueprintState {
            required_sections: vec!["overview".into(), "fees".into(), "faq".into()],
            ..Default::default()
        };
        let selected = page_verified_support(&blueprint, &support);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].support_ref, "fee");
        assert!(missing_required_factual_roles(&blueprint, &selected).is_empty());
        let mut stale = support[0].clone();
        stale.freshness_class = "stale".into();
        assert!(page_verified_support(&blueprint, &[stale]).is_empty());
    }

    #[test]
    fn repeated_graph_roles_do_not_create_duplicate_links_or_future_targets() {
        let link = LinkRecommendationState {
            source_page_key: "source".into(),
            target_page_key: "parent".into(),
            required_flag: true,
            score: 0.8,
            link_recommendation_key: "a".into(),
            status: "candidate".into(),
            ..Default::default()
        };
        let better = LinkRecommendationState {
            score: 0.95,
            link_recommendation_key: "b".into(),
            ..link.clone()
        };
        let future = LinkRecommendationState {
            target_page_key: "future".into(),
            ..link.clone()
        };
        let selected = required_links_for_published_targets(
            &[link, better, future],
            "source",
            &std::collections::BTreeSet::from(["parent".into()]),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].link_recommendation_key, "b");
    }
}

#[cfg(test)]
mod page_admissibility_tests {
    use super::*;

    #[test]
    fn mixed_plan_retains_every_live_blocker_and_excludes_retired_history() {
        let blueprint = PageBlueprintState {
            required_sections: vec!["fees".into()],
            ..Default::default()
        };
        let support = vec![SeoVerifiedFactSupportState {
            support_ref: "fee".into(),
            role_type: "fee_item".into(),
            fragment_text: "Confirmed fee".into(),
            freshness_class: "fresh".into(),
            ..Default::default()
        }];
        let node = |key: &str, state: &str| PageNodeState {
            page_node_key: key.into(),
            lifecycle_state: state.into(),
            canonical_slug: key.into(),
            canonical_url_path: format!("/{key}/"),
            ..Default::default()
        };
        let gap = ContentGapState {
            page_node_key: "duplicate".into(),
            reason_code: "page_duplicate_verified_evidence".into(),
            missing_topic: "distinct_verified_evidence".into(),
            status: "open".into(),
            ..Default::default()
        };
        assert_eq!(
            page_admissibility(&node("ready", "planned"), &blueprint, &support, &[], true),
            PageAdmissibility::Eligible
        );
        assert_eq!(
            page_admissibility(&node("missing", "blocked"), &blueprint, &[], &[], true),
            PageAdmissibility::Blocked("page_missing_verified_support:roles=fees".into())
        );
        assert_eq!(
            page_admissibility(
                &node("duplicate", "blocked"),
                &blueprint,
                &support,
                &[gap],
                true
            ),
            PageAdmissibility::Blocked(
                "page_duplicate_verified_evidence:distinct_verified_evidence".into()
            )
        );
        assert_eq!(
            page_admissibility(&node("old", "deprecated"), &blueprint, &[], &[], true),
            PageAdmissibility::Retired
        );
        assert_eq!(
            page_admissibility(&node("unknown", "blocked"), &blueprint, &support, &[], true),
            PageAdmissibility::Blocked("page_blocked_by_reconciliation".into())
        );
    }
}
