use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicabilityRuleCandidate {
    pub rule_instance_id: String,
    pub has_profile_overrides: bool,
    pub has_apply_profile: bool,
    pub has_conditional_profile: bool,
    pub has_exclude_profile: bool,
    pub has_waive_exception: bool,
    pub has_remove_exception: bool,
    pub has_replace_exception: bool,
    pub has_add_requirement_exception: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportResolutionDiagnostic {
    pub rule_instance_id: String,
    pub reason_code: String,
    pub detail: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportResolutionDecision {
    Include,
    Excluded(&'static str),
    Unresolved(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedVerifiedSupportBundle<T> {
    pub included: Vec<T>,
    pub excluded_rules: Vec<SupportResolutionDiagnostic>,
    pub unresolved_rules: Vec<SupportResolutionDiagnostic>,
    pub applied_overrides: Vec<SupportResolutionDiagnostic>,
}

impl<T> Default for ResolvedVerifiedSupportBundle<T> {
    fn default() -> Self {
        Self {
            included: Vec::new(),
            excluded_rules: Vec::new(),
            unresolved_rules: Vec::new(),
            applied_overrides: Vec::new(),
        }
    }
}

fn candidate_flags(candidate: &ApplicabilityRuleCandidate) -> Value {
    json!({
        "has_profile_overrides": candidate.has_profile_overrides,
        "has_apply_profile": candidate.has_apply_profile,
        "has_conditional_profile": candidate.has_conditional_profile,
        "has_exclude_profile": candidate.has_exclude_profile,
        "has_waive_exception": candidate.has_waive_exception,
        "has_remove_exception": candidate.has_remove_exception,
        "has_replace_exception": candidate.has_replace_exception,
        "has_add_requirement_exception": candidate.has_add_requirement_exception,
    })
}

pub fn decide_support_resolution(
    candidate: &ApplicabilityRuleCandidate,
) -> SupportResolutionDecision {
    if candidate.has_exclude_profile {
        return SupportResolutionDecision::Excluded("profile_excluded");
    }
    if candidate.has_waive_exception {
        return SupportResolutionDecision::Excluded("waived_by_exception");
    }
    if candidate.has_remove_exception {
        return SupportResolutionDecision::Excluded("removed_by_exception");
    }
    if candidate.has_replace_exception || candidate.has_add_requirement_exception {
        return SupportResolutionDecision::Unresolved("unresolved_exception_override");
    }
    if candidate.has_conditional_profile {
        return SupportResolutionDecision::Unresolved("unresolved_conditional_applicability");
    }
    if candidate.has_profile_overrides && !candidate.has_apply_profile {
        return SupportResolutionDecision::Excluded("profile_not_applicable");
    }
    SupportResolutionDecision::Include
}

pub fn resolve_support_candidates<T: Clone>(
    applicant_profile: &str,
    candidates: Vec<(ApplicabilityRuleCandidate, T)>,
) -> ResolvedVerifiedSupportBundle<T> {
    let mut resolution = ResolvedVerifiedSupportBundle::default();
    for (candidate, item) in candidates {
        match decide_support_resolution(&candidate) {
            SupportResolutionDecision::Include => resolution.included.push(item),
            SupportResolutionDecision::Excluded(reason_code) => {
                resolution.excluded_rules.push(SupportResolutionDiagnostic {
                    rule_instance_id: candidate.rule_instance_id.clone(),
                    reason_code: reason_code.to_string(),
                    detail: json!({
                        "applicant_profile": applicant_profile,
                        "flags": candidate_flags(&candidate),
                    }),
                });
            }
            SupportResolutionDecision::Unresolved(reason_code) => {
                let detail = json!({
                    "applicant_profile": applicant_profile,
                    "policy": "unsupported_or_conditional_rules_do_not_auto_support",
                    "flags": candidate_flags(&candidate),
                });
                let diagnostic = SupportResolutionDiagnostic {
                    rule_instance_id: candidate.rule_instance_id.clone(),
                    reason_code: reason_code.to_string(),
                    detail: detail.clone(),
                };
                if candidate.has_replace_exception || candidate.has_add_requirement_exception {
                    resolution.applied_overrides.push(diagnostic.clone());
                }
                resolution.unresolved_rules.push(diagnostic);
            }
        }
    }
    resolution
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(rule_instance_id: &str) -> ApplicabilityRuleCandidate {
        ApplicabilityRuleCandidate {
            rule_instance_id: rule_instance_id.to_string(),
            has_profile_overrides: false,
            has_apply_profile: false,
            has_conditional_profile: false,
            has_exclude_profile: false,
            has_waive_exception: false,
            has_remove_exception: false,
            has_replace_exception: false,
            has_add_requirement_exception: false,
        }
    }

    #[test]
    fn conditional_rules_do_not_auto_support() {
        let mut conditional = candidate("rule:1");
        conditional.has_profile_overrides = true;
        conditional.has_conditional_profile = true;
        let resolved =
            resolve_support_candidates("minor", vec![(conditional, "support".to_string())]);
        assert!(resolved.included.is_empty());
        assert_eq!(resolved.unresolved_rules.len(), 1);
    }
}
