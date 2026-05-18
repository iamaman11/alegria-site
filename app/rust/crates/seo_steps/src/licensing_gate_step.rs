use contracts::generated::alegria::temporal::v1::{
    DraftState, SeoPublishBlockerState, SeoTraceabilityEntryState,
};

fn blocker(reason_code: &str, required_next_action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason_code.to_string(),
        task_type: "publish_gate_blocker".to_string(),
        required_next_action: required_next_action.to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}

fn blocked_license_entry(entry: &SeoTraceabilityEntryState) -> bool {
    entry.traceability_label == "restricted_redistribution_source"
        || entry.traceability_label == "non_redistributable_source"
        || entry.validation_verdict == "blocked"
}

pub fn evaluate(draft: &DraftState) -> Vec<SeoPublishBlockerState> {
    let mut blockers = Vec::new();
    let mut found_restricted_source = false;

    for entry in &draft.traceability_entries {
        if blocked_license_entry(entry) {
            found_restricted_source = true;
            blockers.push(blocker(
                "restricted_redistribution_source",
                "Replace restricted source context with a redistributable source or remove the downstream publish path.",
            ));
        }
    }

    if found_restricted_source {
        blockers.sort_by(|left, right| left.reason_code.cmp(&right.reason_code));
        blockers.dedup_by(|left, right| left.reason_code == right.reason_code);
    }

    blockers
}

pub fn has_license_block(draft: &DraftState) -> bool {
    !evaluate(draft).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{DraftState, SeoTraceabilityEntryState};

    #[test]
    fn blocks_restricted_redistribution_source() {
        let blockers = evaluate(&DraftState {
            traceability_entries: vec![SeoTraceabilityEntryState {
                fragment_key: "license:block".to_string(),
                fragment_text: "restricted".to_string(),
                fragment_kind: "policy".to_string(),
                traceability_label: "restricted_redistribution_source".to_string(),
                support_refs: Vec::new(),
                validation_verdict: "blocked".to_string(),
            }],
            ..DraftState::default()
        });

        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].reason_code, "restricted_redistribution_source");
    }

    #[test]
    fn allows_clean_draft() {
        assert!(evaluate(&DraftState::default()).is_empty());
    }
}
