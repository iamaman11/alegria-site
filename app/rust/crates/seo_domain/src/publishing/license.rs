//! The editorial licensing policy. Traceability comes from external adapters;
//! a source that cannot legally be redistributed cannot pass the publish gate.

pub const RESTRICTED_SOURCE_REASON: &str = "restricted_redistribution_source";
pub const RESTRICTED_SOURCE_ACTION: &str =
    "Replace restricted source context with a redistributable source or remove the downstream publish path.";

pub fn source_is_restricted(traceability_label: &str, validation_verdict: &str) -> bool {
    matches!(
        traceability_label,
        "restricted_redistribution_source" | "non_redistributable_source"
    ) || validation_verdict == "blocked"
}

/// Returns a single licensing decision regardless of how many restricted
/// fragments are present. The transport-specific blocker is assembled by QA.
pub fn has_restricted_source<'a>(sources: impl IntoIterator<Item = (&'a str, &'a str)>) -> bool {
    sources
        .into_iter()
        .any(|(label, verdict)| source_is_restricted(label, verdict))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_license_or_blocked_validation_denies_publication() {
        for (label, verdict) in [
            ("restricted_redistribution_source", "supported"),
            ("non_redistributable_source", "supported"),
            ("verified_fact", "blocked"),
        ] {
            assert!(source_is_restricted(label, verdict));
        }
        assert!(!source_is_restricted("verified_fact", "supported"));
    }

    #[test]
    fn restricted_source_decision_does_not_duplicate_blockers() {
        assert!(!has_restricted_source([]));
        assert!(!has_restricted_source([("verified_summary", "supported")]));
        assert!(has_restricted_source([
            ("verified_summary", "supported"),
            ("non_redistributable_source", "supported"),
            ("restricted_redistribution_source", "blocked"),
        ]));
    }
}
