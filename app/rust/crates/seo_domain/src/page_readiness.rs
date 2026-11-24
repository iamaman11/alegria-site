//! The resolved page plan is the authority for generation and publication.
//! Transport adapters only convert inputs/selected indices; no protobuf here.

use std::collections::{BTreeMap, BTreeSet};

use crate::editorial::{factual_role, required_section_roles, role_matches_section, VerifiedSupport};

#[derive(Debug, Clone, Copy)]
pub struct PageNode<'a> {
    pub page_node_key: &'a str,
    pub lifecycle_state: &'a str,
    pub canonical_slug: &'a str,
    pub canonical_url_path: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct ContentGap<'a> {
    pub page_node_key: &'a str,
    pub status: &'a str,
    pub reason_code: &'a str,
    pub missing_topic: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct LinkRecommendation<'a> {
    pub source_page_key: &'a str,
    pub target_page_key: &'a str,
    pub link_recommendation_key: &'a str,
    pub status: &'a str,
    pub required: bool,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageAdmissibility {
    Retired,
    Blocked(String),
    Eligible,
}

/// Truth may be read before discovery, but a post-truth drafting barrier must
/// reject incomplete or non-fresh support. Temporal only supplies the fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruthSupportDefect {
    MissingMandatoryFields,
    NonFresh,
}

pub fn truth_support_defect(
    support_ref: &str,
    fragment_text: &str,
    role_type: &str,
    source_label: &str,
    freshness_class: &str,
) -> Option<TruthSupportDefect> {
    if [support_ref, fragment_text, role_type, source_label]
        .iter()
        .any(|field| field.trim().is_empty())
    {
        Some(TruthSupportDefect::MissingMandatoryFields)
    } else if freshness_class != "fresh" {
        Some(TruthSupportDefect::NonFresh)
    } else {
        None
    }
}

pub fn is_opaque_topic_identifier(value: &str) -> bool {
    let trimmed = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    let hex = trimmed.replace('-', "");
    hex.len() >= 32 && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Empty explicit sections mean the default blueprint, not no requirements.
pub fn missing_required_factual_roles(
    required_sections: &[String],
    support: &[VerifiedSupport<'_>],
) -> Vec<String> {
    let required_roles = required_section_roles(required_sections);
    let mut missing = required_roles
        .into_iter()
        .filter(|role| factual_role(role))
        .filter(|role| {
            !support.iter().any(|entry| {
                role_matches_section(entry.role_type, role)
                    && !entry.support_ref.trim().is_empty()
                    && !entry.fragment_text.trim().is_empty()
                    && entry.freshness_class == "fresh"
            })
        })
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    missing
}

/// Select the indices of verified evidence owned by this page. Neither
/// overview nor FAQ may widen the bundle to unrelated facts.
pub fn selected_support_indices(
    required_sections: &[String],
    support: &[VerifiedSupport<'_>],
) -> Vec<usize> {
    let direct_roles = required_sections
        .iter()
        .filter(|role| {
            matches!(
                role.as_str(),
                "documents" | "fees" | "timing" | "where_to_apply" | "process" | "who_fits"
            )
        })
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    support
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            !entry.support_ref.trim().is_empty()
                && !entry.fragment_text.trim().is_empty()
                && entry.freshness_class == "fresh"
        })
        .filter(|(_, entry)| {
            direct_roles.is_empty()
                || direct_roles
                    .iter()
                    .any(|role| role_matches_section(entry.role_type, role))
        })
        .filter(|(_, entry)| seen.insert(entry.support_ref))
        .map(|(index, _)| index)
        .collect()
}

pub fn page_admissibility(
    page: PageNode<'_>,
    required_sections: &[String],
    support: &[VerifiedSupport<'_>],
    gaps: &[ContentGap<'_>],
    requires_editorial: bool,
) -> PageAdmissibility {
    if page.lifecycle_state == "deprecated" {
        return PageAdmissibility::Retired;
    }
    if is_opaque_topic_identifier(page.canonical_slug)
        || page
            .canonical_url_path
            .split('/')
            .any(is_opaque_topic_identifier)
    {
        return PageAdmissibility::Blocked("opaque_page_path".into());
    }
    if requires_editorial {
        let missing = missing_required_factual_roles(required_sections, support);
        if !missing.is_empty() {
            return PageAdmissibility::Blocked(format!(
                "page_missing_verified_support:roles={}",
                missing.join(",")
            ));
        }
    }
    if matches!(page.lifecycle_state, "blocked" | "stale") {
        let mut reasons = gaps
            .iter()
            .filter(|gap| {
                gap.page_node_key == page.page_node_key
                    && matches!(gap.status, "open" | "accepted")
                    && matches!(
                        gap.reason_code,
                        "page_missing_verified_support" | "page_duplicate_verified_evidence"
                    )
            })
            .map(|gap| format!("{}:{}", gap.reason_code, gap.missing_topic))
            .collect::<Vec<_>>();
        reasons.sort();
        reasons.dedup();
        return PageAdmissibility::Blocked(if reasons.is_empty() {
            "page_blocked_by_reconciliation".into()
        } else {
            reasons.join(",")
        });
    }
    PageAdmissibility::Eligible
}

/// A published target is one obligation even when supported by several graph
/// roles. Select the strongest recommendation and preserve target-key order.
pub fn required_link_indices(
    links: &[LinkRecommendation<'_>],
    source_page_key: &str,
    published: &BTreeSet<String>,
) -> Vec<usize> {
    let mut by_target = BTreeMap::<&str, usize>::new();
    for (index, link) in links.iter().enumerate().filter(|(_, link)| {
        link.required
            && link.source_page_key == source_page_key
            && link.target_page_key != source_page_key
            && published.contains(link.target_page_key)
            && !matches!(link.status, "rejected" | "expired" | "deprecated")
    }) {
        let replace = by_target.get(link.target_page_key).is_none_or(|prior| {
            let prior = &links[*prior];
            link.score.total_cmp(&prior.score).is_gt()
                || (link.score == prior.score
                    && link.link_recommendation_key < prior.link_recommendation_key)
        });
        if replace {
            by_target.insert(link.target_page_key, index);
        }
    }
    by_target.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truth_support_admissibility_has_one_typed_domain_policy() {
        assert_eq!(
            truth_support_defect("rule:1", "exact source", "fee_item", "Authority", "fresh"),
            None
        );
        for (support_ref, text, role, source) in [
            ("", "exact source", "fee_item", "Authority"),
            ("rule:1", "", "fee_item", "Authority"),
            ("rule:1", "exact source", " ", "Authority"),
            ("rule:1", "exact source", "fee_item", ""),
        ] {
            assert_eq!(
                truth_support_defect(support_ref, text, role, source, "fresh"),
                Some(TruthSupportDefect::MissingMandatoryFields)
            );
        }
        for freshness in ["watch", "stale", "", "unknown"] {
            assert_eq!(
                truth_support_defect("rule:1", "exact source", "fee_item", "Authority", freshness),
                Some(TruthSupportDefect::NonFresh)
            );
        }
    }

    fn support(role_type: &str) -> VerifiedSupport<'_> {
        VerifiedSupport {
            role_type,
            support_ref: "verified-rule-1",
            fragment_text: "Exact original source quotation",
            freshness_class: "fresh",
            source_url: "https://example.org/source",
        }
    }

    #[test]
    fn page_specific_readiness_and_default_requirements() {
        assert!(missing_required_factual_roles(
            &["overview".into(), "faq".into(), "related_pages".into()],
            &[support("eligibility_rule")],
        )
        .is_empty());
        assert_eq!(
            missing_required_factual_roles(&["fees".into()], &[support("eligibility_rule")]),
            vec!["fees"]
        );
        let missing = missing_required_factual_roles(&[], &[support("eligibility_rule")]);
        assert!(missing.contains(&"documents".to_string()));
        assert!(missing.contains(&"timing".to_string()));
    }

    #[test]
    fn stale_or_missing_evidence_never_satisfies_required_role() {
        let stale = VerifiedSupport {
            freshness_class: "stale",
            ..support("fee_item")
        };
        let empty = VerifiedSupport {
            fragment_text: "",
            ..support("fee_item")
        };
        assert_eq!(
            missing_required_factual_roles(&["fees".into()], &[stale]),
            vec!["fees"]
        );
        assert_eq!(
            missing_required_factual_roles(&["fees".into()], &[empty]),
            vec!["fees"]
        );
    }

    #[test]
    fn opaque_ids_never_enter_public_paths() {
        assert!(is_opaque_topic_identifier(&"a".repeat(64)));
        assert!(is_opaque_topic_identifier(
            "123e4567-e89b-12d3-a456-426614174000"
        ));
        assert!(!is_opaque_topic_identifier("plant-leaf-care"));
    }
}
