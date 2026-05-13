use serde_json::{json, Value};
use std::collections::BTreeSet;

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
}
