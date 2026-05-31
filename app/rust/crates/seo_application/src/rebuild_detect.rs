use contracts::generated::alegria::temporal::v1::{
    RebuildDetectInputPayload, RebuildDetectOutputPayload, RebuildImpactState,
};
use primitives::errors::DomainError;
use seo_domain::rebuild;
use seo_ports::{RebuildDependencyEvidence, RebuildRepository};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

async fn ensure_graph_required_for_phase(phase: &str) -> Result<(), DomainError> {
    seo_steps::read_neo4j_context::read_neo4j_context(phase)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("graph capability contract failed for rebuild phase `{phase}`: {err}"),
        })
}

pub async fn execute<R: RebuildRepository>(
    repo: &R,
    input: &RebuildDetectInputPayload,
) -> Result<RebuildDetectOutputPayload, DomainError> {
    ensure_graph_required_for_phase("rebuild_detect").await?;
    let mut narrowed_input = input.clone();
    let mut semantic_neighbor_evidence: Vec<RebuildDependencyEvidence> = Vec::new();
    if !input.changed_truth_keys.is_empty() {
        let impacted = repo
            .narrow_rebuild_impacts(&input.changed_truth_keys)
            .await?;
        let semantic_impacted = repo
            .semantic_neighbor_impacts(&input.changed_truth_keys, &input.page_nodes)
            .await?;
        semantic_neighbor_evidence = semantic_impacted.clone();
        let combined = impacted
            .into_iter()
            .chain(semantic_impacted.into_iter())
            .collect::<Vec<_>>();
        if !combined.is_empty() {
            let impacted_keys = combined
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
    let enriched_output = annotate_semantic_neighbor_evidence(output, &semantic_neighbor_evidence);
    repo.persist_rebuild_detect_output(input, &enriched_output)
        .await?;
    Ok(enriched_output)
}

fn annotate_semantic_neighbor_evidence(
    mut output: RebuildDetectOutputPayload,
    evidence: &[RebuildDependencyEvidence],
) -> RebuildDetectOutputPayload {
    if evidence.is_empty() {
        return output;
    }
    let mut by_page = BTreeMap::<String, Vec<Value>>::new();
    for item in evidence {
        by_page
            .entry(item.page_node_key.clone())
            .or_default()
            .push(item.reason_package.clone());
    }
    for impact in &mut output.impacts {
        let Some(page_evidence) = by_page.get(&impact.page_node_key) else {
            continue;
        };
        let mut reason = serde_json::from_str::<Value>(&impact.reason_package_json)
            .unwrap_or_else(|_| Value::Object(Default::default()));
        if let Value::Object(map) = &mut reason {
            map.insert("semantic_neighbor_widening".to_string(), Value::Bool(true));
            map.insert(
                "semantic_neighbor_evidence".to_string(),
                Value::Array(page_evidence.clone()),
            );
        }
        impact.reason_package_json = reason.to_string();
    }
    output
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

        async fn semantic_neighbor_impacts(
            &self,
            _changed_truth_keys: &[String],
            _page_nodes: &[contracts::generated::alegria::temporal::v1::PageNodeState],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(vec![RebuildDependencyEvidence {
                page_node_key: "page:a".to_string(),
                reason_package: json!({"semantic_neighbor_widening": true}),
            }])
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
        let keys = output
            .impacted_page_node_keys
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        assert!(keys.contains("page:a"));
        assert!(keys.contains("page:b"));
        assert!(output.impacts.iter().any(|impact| impact
            .reason_package_json
            .contains("semantic_neighbor_widening")));
    }
}
