use contracts::generated::alegria::temporal::v1::{
    RebuildDetectInputPayload, RebuildDetectOutputPayload, RebuildImpactState,
};
use primitives::errors::DomainError;
use seo_domain::rebuild;
use seo_ports::RebuildRepository;
use serde_json::Value;
use std::collections::HashSet;

pub async fn execute<R: RebuildRepository>(
    repo: &R,
    input: &RebuildDetectInputPayload,
) -> Result<RebuildDetectOutputPayload, DomainError> {
    let mut narrowed_input = input.clone();
    if !input.changed_truth_keys.is_empty() {
        let impacted = repo
            .narrow_rebuild_impacts(&input.changed_truth_keys)
            .await?;
        if !impacted.is_empty() {
            let impacted_keys = impacted
                .iter()
                .map(|evidence| evidence.page_node_key.clone())
                .collect::<HashSet<_>>();
            narrowed_input.page_nodes = input
                .page_nodes
                .iter()
                .filter(|page| impacted_keys.contains(&page.page_node_key))
                .cloned()
                .collect();
        }
    }
    let output = execute_rebuild_detect(&narrowed_input);
    repo.persist_rebuild_detect_output(input, &output).await?;
    Ok(output)
}

fn execute_rebuild_detect(input: &RebuildDetectInputPayload) -> RebuildDetectOutputPayload {
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
    let reason: Value = rebuild::canonical_reason_package(&trigger_type, &changed_keys);
    let impacts = input
        .page_nodes
        .iter()
        .map(|page| page.page_node_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .map(|page_node_key| RebuildImpactState {
            page_node_key: page_node_key.clone(),
            trigger_type: trigger_type.clone(),
            reason_package_json: reason.to_string(),
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
    use async_trait::async_trait;
    use seo_ports::RebuildDependencyEvidence;
    use serde_json::json;

    struct FakeRepo;

    #[async_trait]
    impl RebuildRepository for FakeRepo {
        async fn narrow_rebuild_impacts(
            &self,
            _changed_truth_keys: &[String],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(vec![RebuildDependencyEvidence {
                page_node_key: "page:b".to_string(),
                reason_package: json!({"matched_dependencies":[]}),
            }])
        }

        async fn persist_rebuild_detect_output(
            &self,
            _input: &RebuildDetectInputPayload,
            _output: &RebuildDetectOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn narrows_pages_before_rebuild_detect() {
        let repo = FakeRepo;
        let output = execute(
            &repo,
            &RebuildDetectInputPayload {
                run_id: "run-1".to_string(),
                changed_truth_keys: vec!["verified.rule_instance:passport".to_string()],
                page_nodes: vec![
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "page:a".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "page:b".to_string(),
                        ..Default::default()
                    },
                ],
            },
        )
        .await
        .unwrap();
        assert_eq!(output.impacted_page_node_keys, vec!["page:b".to_string()]);
    }
}
