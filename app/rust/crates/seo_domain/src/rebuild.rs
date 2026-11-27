use serde_json::{json, Value};
use std::collections::BTreeSet;

/// One use-case input, independent of Temporal's page and workflow payloads.
/// Rebuild depends on page identity, never on unrelated page presentation data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildDetectRequest {
    pub run_id: String,
    pub changed_truth_keys: Vec<String>,
    pub page_node_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildVerdict {
    NoRebuildRequired,
    RebuildRequired,
}

impl RebuildVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoRebuildRequired => "no_rebuild_required",
            Self::RebuildRequired => "rebuild_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RebuildImpact {
    pub page_node_key: String,
    pub trigger_type: String,
    pub reason_package: Value,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RebuildDetectResult {
    pub impacted_page_node_keys: Vec<String>,
    pub verdict: RebuildVerdict,
    pub impacts: Vec<RebuildImpact>,
}

pub const TRUTH_CHANGE: &str = "truth_change";
pub const PROFILE_APPLICABILITY_CHANGE: &str = "profile_applicability_change";
pub const SCOPE_CHANGE: &str = "scope_change";
pub const TEMPLATE_CHANGE: &str = "template_change";
pub const SERP_CHANGE: &str = "serp_change";
pub const LOCALE_CHANGE: &str = "locale_change";

pub fn classify_trigger(changed_truth_keys: &[String]) -> String {
    let mut classes = BTreeSet::new();
    for key in changed_truth_keys {
        let trimmed = key.trim();
        if trimmed.starts_with("profile_applicability:")
            || trimmed.starts_with("verified.rule_profile:")
            || trimmed.starts_with("verified.rule_exception:")
        {
            classes.insert(PROFILE_APPLICABILITY_CHANGE);
        } else if trimmed.starts_with("scope_change:") {
            classes.insert(SCOPE_CHANGE);
        } else if trimmed.starts_with("locale_change:") {
            classes.insert(LOCALE_CHANGE);
        } else if trimmed.starts_with("site.page_blueprint:")
            || trimmed.starts_with("blueprint:")
            || trimmed.starts_with("site.section_template:")
            || trimmed.starts_with("template:")
        {
            classes.insert(TEMPLATE_CHANGE);
        } else if trimmed.starts_with("serp.query:") || trimmed.starts_with("serp_pattern:") {
            classes.insert(SERP_CHANGE);
        } else {
            classes.insert(TRUTH_CHANGE);
        }
    }

    if classes.contains(TRUTH_CHANGE) {
        TRUTH_CHANGE.to_string()
    } else if classes.contains(PROFILE_APPLICABILITY_CHANGE) {
        PROFILE_APPLICABILITY_CHANGE.to_string()
    } else if classes.contains(TEMPLATE_CHANGE) {
        TEMPLATE_CHANGE.to_string()
    } else if classes.contains(SERP_CHANGE) {
        SERP_CHANGE.to_string()
    } else if classes.contains(SCOPE_CHANGE) {
        SCOPE_CHANGE.to_string()
    } else if classes.contains(LOCALE_CHANGE) {
        LOCALE_CHANGE.to_string()
    } else {
        TRUTH_CHANGE.to_string()
    }
}

pub fn trigger_priority(trigger_type: &str) -> i32 {
    match trigger_type {
        TRUTH_CHANGE | PROFILE_APPLICABILITY_CHANGE | TEMPLATE_CHANGE => 1,
        SERP_CHANGE | SCOPE_CHANGE | LOCALE_CHANGE => 2,
        _ => 2,
    }
}

pub fn lifecycle_state_for_trigger(trigger_type: &str) -> Option<&'static str> {
    match trigger_type {
        TRUTH_CHANGE | PROFILE_APPLICABILITY_CHANGE | TEMPLATE_CHANGE => Some("needs_rebuild"),
        SCOPE_CHANGE | LOCALE_CHANGE | SERP_CHANGE => None,
        _ => Some("needs_rebuild"),
    }
}

pub fn canonical_reason_package(trigger_type: &str, changed_truth_keys: &[String]) -> Value {
    json!({
        "trigger_type": trigger_type,
        "matched_dependencies": [],
        "changed_truth_keys": changed_truth_keys
            .iter()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect::<Vec<_>>(),
    })
}

/// The single deterministic rebuild decision. Application chooses which pages
/// are affected; this owner classifies the trigger and produces the reasons.
/// Neither Temporal DTOs nor storage adapters may duplicate these decisions.
pub fn detect_rebuild(input: &RebuildDetectRequest) -> RebuildDetectResult {
    if input.changed_truth_keys.is_empty() {
        return RebuildDetectResult {
            impacted_page_node_keys: Vec::new(),
            verdict: RebuildVerdict::NoRebuildRequired,
            impacts: Vec::new(),
        };
    }
    let trigger_type = classify_trigger(&input.changed_truth_keys);
    let priority = trigger_priority(&trigger_type);
    let changed_keys = input
        .changed_truth_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    let reason_package = canonical_reason_package(&trigger_type, &changed_keys);
    let impacts = input
        .page_node_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .map(|page_node_key| RebuildImpact {
            page_node_key,
            trigger_type: trigger_type.clone(),
            reason_package: reason_package.clone(),
            priority,
        })
        .collect::<Vec<_>>();
    RebuildDetectResult {
        impacted_page_node_keys: impacts
            .iter()
            .map(|impact| impact.page_node_key.clone())
            .collect(),
        verdict: RebuildVerdict::RebuildRequired,
        impacts,
    }
}

/// Independent pure rebuild impact decision. Execution and scheduling remain
/// Temporal/PG effects, explicitly outside this operation contract.
pub mod operations {
    use std::collections::{BTreeMap, BTreeSet};
    use std::convert::Infallible;

    use operation_protocol::{
        ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId,
        OperationOwnerId, OperationRef, PortId,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::{detect_rebuild, RebuildDetectRequest, RebuildVerdict};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RebuildImpactInput {
        pub run_id: String,
        pub changed_truth_keys: Vec<String>,
        pub page_node_keys: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum RebuildDecision {
        NoRebuildRequired,
        RebuildRequired,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RebuildReason {
        pub trigger_type: String,
        pub matched_dependencies: Vec<String>,
        pub changed_truth_keys: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RebuildPageImpact {
        pub page_node_key: String,
        pub trigger_type: String,
        pub reason: RebuildReason,
        pub priority: i32,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RebuildImpactOutput {
        pub impacted_page_node_keys: Vec<String>,
        pub verdict: RebuildDecision,
        pub impacts: Vec<RebuildPageImpact>,
    }

    pub struct DetectRebuildImpact;

    impl Operation for DetectRebuildImpact {
        type Input = RebuildImpactInput;
        type Output = RebuildImpactOutput;
        type Failure = Infallible;

        fn contract() -> OperationContract {
            OperationContract {
                reference: OperationRef {
                    id: OperationId::from("publishing.detect_rebuild_impact"),
                    revision: 1,
                },
                owner: OperationOwnerId::from("publishing_policy"),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    ArtifactSchema {
                        id: "publishing.rebuild_impact_input".into(),
                        revision: 1,
                    },
                )]),
                outputs: BTreeMap::from([(
                    PortId::from("output"),
                    ArtifactSchema {
                        id: "publishing.rebuild_impact_output".into(),
                        revision: 1,
                    },
                )]),
                invalidates: BTreeSet::new(),
                effects: BTreeSet::new(),
                idempotency: IdempotencyPolicy::Pure,
            }
        }
    }

    impl DetectRebuildImpact {
        pub fn evaluate(input: &RebuildImpactInput) -> RebuildImpactOutput {
            let decision = detect_rebuild(&RebuildDetectRequest {
                run_id: input.run_id.clone(),
                changed_truth_keys: input.changed_truth_keys.clone(),
                page_node_keys: input.page_node_keys.clone(),
            });
            let normalized_changes = input
                .changed_truth_keys
                .iter()
                .map(|key| key.trim().to_owned())
                .filter(|key| !key.is_empty())
                .collect::<Vec<_>>();
            RebuildImpactOutput {
                impacted_page_node_keys: decision.impacted_page_node_keys,
                verdict: match decision.verdict {
                    RebuildVerdict::NoRebuildRequired => RebuildDecision::NoRebuildRequired,
                    RebuildVerdict::RebuildRequired => RebuildDecision::RebuildRequired,
                },
                impacts: decision
                    .impacts
                    .into_iter()
                    .map(|impact| RebuildPageImpact {
                        page_node_key: impact.page_node_key,
                        trigger_type: impact.trigger_type.clone(),
                        reason: RebuildReason {
                            trigger_type: impact.trigger_type,
                            matched_dependencies: Vec::new(),
                            changed_truth_keys: normalized_changes.clone(),
                        },
                        priority: impact.priority,
                    })
                    .collect(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn independent_pure_rebuild_decision_uses_existing_owner_and_no_side_effect() {
            let contract = DetectRebuildImpact::contract();
            assert!(contract.effects.is_empty());
            assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
            let decision = DetectRebuildImpact::evaluate(&RebuildImpactInput {
                run_id: "test-run".to_owned(),
                changed_truth_keys: vec![" scope_change:ru ".to_owned()],
                page_node_keys: vec![" page:1 ".to_owned()],
            });
            assert_eq!(decision.verdict, RebuildDecision::RebuildRequired);
            assert_eq!(decision.impacted_page_node_keys, vec!["page:1"]);
            assert_eq!(
                decision.impacts[0].reason.changed_truth_keys,
                vec!["scope_change:ru"]
            );
            assert_eq!(decision.impacts[0].priority, 2);
            assert_eq!(
                DetectRebuildImpact::evaluate(&RebuildImpactInput {
                    run_id: "test-run".to_owned(),
                    changed_truth_keys: vec![],
                    page_node_keys: vec!["page:1".to_owned()],
                })
                .verdict,
                RebuildDecision::NoRebuildRequired
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_applicability_beats_scope_only() {
        let trigger = classify_trigger(&[
            "scope_change:scope:ru".to_string(),
            "profile_applicability:rule:minor".to_string(),
        ]);
        assert_eq!(trigger, PROFILE_APPLICABILITY_CHANGE);
    }

    #[test]
    fn typed_rebuild_decision_keeps_order_reasons_and_priority() {
        let decision = detect_rebuild(&RebuildDetectRequest {
            run_id: "run-1".to_string(),
            changed_truth_keys: vec![
                "scope_change:ru".to_string(),
                " verified.rule_instance:fee ".to_string(),
            ],
            page_node_keys: vec![" page:a ".to_string(), "page:b".to_string()],
        });
        assert_eq!(decision.verdict, RebuildVerdict::RebuildRequired);
        assert_eq!(
            decision.impacted_page_node_keys,
            vec!["page:a".to_string(), "page:b".to_string()]
        );
        assert_eq!(decision.impacts[0].trigger_type, TRUTH_CHANGE);
        assert_eq!(decision.impacts[0].priority, 1);
        assert_eq!(
            decision.impacts[0].reason_package,
            json!({
                "trigger_type":"truth_change",
                "matched_dependencies":[],
                "changed_truth_keys":["scope_change:ru", "verified.rule_instance:fee"]
            })
        );
        assert_eq!(
            decision.impacts[0].reason_package,
            decision.impacts[1].reason_package
        );
    }

    #[test]
    fn empty_truth_diff_never_invents_rebuild_even_with_pages() {
        let decision = detect_rebuild(&RebuildDetectRequest {
            run_id: "run-1".to_string(),
            changed_truth_keys: Vec::new(),
            page_node_keys: vec!["page:a".to_string()],
        });
        assert_eq!(decision.verdict, RebuildVerdict::NoRebuildRequired);
        assert!(decision.impacts.is_empty());
    }

    #[test]
    fn profile_applicability_change_precedes_scope_and_keeps_wire_reason() {
        let decision = detect_rebuild(&RebuildDetectRequest {
            run_id: "run".to_string(),
            changed_truth_keys: vec![
                "scope_change:scope:ru".to_string(),
                "profile_applicability:rule:minor".to_string(),
            ],
            page_node_keys: vec!["page:a".to_string()],
        });
        assert_eq!(decision.verdict.as_str(), "rebuild_required");
        assert_eq!(
            decision.impacts[0].trigger_type,
            PROFILE_APPLICABILITY_CHANGE
        );
        assert_eq!(decision.impacts[0].priority, 1);
        assert_eq!(
            decision.impacts[0].reason_package.to_string(),
            canonical_reason_package(
                PROFILE_APPLICABILITY_CHANGE,
                &[
                    "scope_change:scope:ru".to_string(),
                    "profile_applicability:rule:minor".to_string(),
                ]
            )
            .to_string()
        );
    }

    #[test]
    fn truth_change_precedes_serp_and_locale_scope_is_low_priority() {
        let truth = detect_rebuild(&RebuildDetectRequest {
            run_id: "run".to_string(),
            changed_truth_keys: vec![
                "serp.query:spain tourist visa".to_string(),
                "verified.rule_instance:fee".to_string(),
            ],
            page_node_keys: vec!["page:a".to_string()],
        });
        assert_eq!(truth.impacts[0].trigger_type, TRUTH_CHANGE);
        assert_eq!(truth.impacts[0].priority, 1);

        let locale = detect_rebuild(&RebuildDetectRequest {
            run_id: "run".to_string(),
            changed_truth_keys: vec!["locale_change:ru-RU->en".to_string()],
            page_node_keys: vec!["page:a".to_string()],
        });
        assert_eq!(locale.impacts[0].trigger_type, LOCALE_CHANGE);
        assert_eq!(locale.impacts[0].priority, 2);
    }

    #[test]
    fn a_nonempty_truth_diff_with_no_page_impacts_still_requires_rebuild() {
        let decision = detect_rebuild(&RebuildDetectRequest {
            run_id: "run".to_string(),
            changed_truth_keys: vec!["verified.rule_instance:fee".to_string()],
            page_node_keys: vec![" ".to_string()],
        });
        assert_eq!(decision.verdict, RebuildVerdict::RebuildRequired);
        assert!(decision.impacts.is_empty());
        assert!(decision.impacted_page_node_keys.is_empty());
    }
}
