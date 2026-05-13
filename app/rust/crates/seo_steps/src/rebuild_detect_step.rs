use contracts::generated::alegria::temporal::v1::{
    RebuildDetectInputPayload, RebuildDetectOutputPayload, RebuildImpactState,
};
use seo_domain::rebuild;

pub fn execute(input: &RebuildDetectInputPayload) -> RebuildDetectOutputPayload {
    if input.changed_truth_keys.is_empty() {
        return RebuildDetectOutputPayload {
            impacted_page_node_keys: Vec::new(),
            verdict: "no_rebuild_required".to_string(),
            impacts: Vec::new(),
        };
    }
    let trigger_type = rebuild::classify_trigger(&input.changed_truth_keys);
    let priority = rebuild::trigger_priority(&trigger_type);
    let changed_keys = input
        .changed_truth_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    let impacts = input
        .page_nodes
        .iter()
        .map(|page| page.page_node_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .map(|page_node_key| RebuildImpactState {
            page_node_key: page_node_key.clone(),
            trigger_type: trigger_type.clone(),
            reason_package_json: rebuild::canonical_reason_package(&trigger_type, &changed_keys)
                .to_string(),
            priority,
        })
        .collect::<Vec<_>>();

    RebuildDetectOutputPayload {
        impacted_page_node_keys: impacts
            .iter()
            .map(|impact| impact.page_node_key.clone())
            .collect(),
        verdict: "rebuild_required".to_string(),
        impacts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::PageNodeState;

    fn page(key: &str) -> PageNodeState {
        PageNodeState {
            page_node_key: key.to_string(),
            ..PageNodeState::default()
        }
    }

    #[test]
    fn profile_applicability_change_beats_scope_only_trigger() {
        let output = execute(&RebuildDetectInputPayload {
            run_id: "run".to_string(),
            changed_truth_keys: vec![
                "scope_change:scope:ru".to_string(),
                "profile_applicability:rule:minor".to_string(),
            ],
            page_nodes: vec![page("page:a")],
        });
        assert_eq!(output.verdict, "rebuild_required");
        assert_eq!(
            output.impacts[0].trigger_type,
            "profile_applicability_change"
        );
    }

    #[test]
    fn truth_change_beats_serp_change() {
        let output = execute(&RebuildDetectInputPayload {
            run_id: "run".to_string(),
            changed_truth_keys: vec![
                "serp.query:spain tourist visa".to_string(),
                "verified.rule_instance:fee".to_string(),
            ],
            page_nodes: vec![page("page:a")],
        });
        assert_eq!(output.impacts[0].trigger_type, "truth_change");
    }

    #[test]
    fn locale_change_is_scope_only_trigger() {
        let output = execute(&RebuildDetectInputPayload {
            run_id: "run".to_string(),
            changed_truth_keys: vec!["locale_change:ru-RU->en".to_string()],
            page_nodes: vec![page("page:a")],
        });
        assert_eq!(output.impacts[0].trigger_type, "locale_change");
        assert_eq!(output.impacts[0].priority, 2);
    }
}
