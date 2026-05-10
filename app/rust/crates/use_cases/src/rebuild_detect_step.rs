use contracts::generated::alegria::temporal::v1::{
    RebuildDetectInputPayload, RebuildDetectOutputPayload,
};

pub fn execute(input: &RebuildDetectInputPayload) -> RebuildDetectOutputPayload {
    if input.changed_truth_keys.is_empty() {
        return RebuildDetectOutputPayload {
            impacted_page_node_keys: Vec::new(),
            verdict: "no_rebuild_required".to_string(),
        };
    }
    RebuildDetectOutputPayload {
        impacted_page_node_keys: input
            .page_nodes
            .iter()
            .map(|page| page.page_node_key.clone())
            .filter(|key| !key.is_empty())
            .collect(),
        verdict: "rebuild_required".to_string(),
    }
}
