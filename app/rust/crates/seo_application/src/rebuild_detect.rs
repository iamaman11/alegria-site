use primitives::errors::DomainError;
use seo_domain::rebuild::{self, RebuildDetectRequest, RebuildDetectResult};
use seo_ports::{GraphCapabilityPort, RebuildDependencyEvidence, RebuildRepository};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

async fn ensure_graph_required_for_phase<R: GraphCapabilityPort>(
    repo: &R,
    phase: &str,
) -> Result<(), DomainError> {
    repo.ensure_graph_contract(phase)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("graph capability contract failed for rebuild phase `{phase}`: {err}"),
        })
}

pub async fn execute<R: RebuildRepository + GraphCapabilityPort>(
    repo: &R,
    input: &RebuildDetectRequest,
) -> Result<RebuildDetectResult, DomainError> {
    ensure_graph_required_for_phase(repo, "rebuild_detect").await?;
    let mut narrowed_input = input.clone();
    let mut semantic_neighbor_evidence: Vec<RebuildDependencyEvidence> = Vec::new();
    if !input.changed_truth_keys.is_empty() {
        let impacted = repo
            .narrow_rebuild_impacts(&input.changed_truth_keys)
            .await?;
        let semantic_impacted = repo
            .semantic_neighbor_impacts(&input.changed_truth_keys, &input.page_node_keys)
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
            narrowed_input.page_node_keys = input
                .page_node_keys
                .iter()
                .filter(|page| impacted_keys.contains(*page))
                .cloned()
                .collect();
        }
    }
    let output = rebuild::detect_rebuild(&narrowed_input);
    let enriched_output = annotate_semantic_neighbor_evidence(output, &semantic_neighbor_evidence);
    repo.persist_rebuild_detect_output(input, &enriched_output)
        .await?;
    Ok(enriched_output)
}

fn annotate_semantic_neighbor_evidence(
    mut output: RebuildDetectResult,
    evidence: &[RebuildDependencyEvidence],
) -> RebuildDetectResult {
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
        if let Value::Object(map) = &mut impact.reason_package {
            map.insert("semantic_neighbor_widening".to_string(), Value::Bool(true));
            map.insert(
                "semantic_neighbor_evidence".to_string(),
                Value::Array(page_evidence.clone()),
            );
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use seo_ports::{GraphCapabilityPort, RebuildDependencyEvidence};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        persisted_run_ids: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl GraphCapabilityPort for FakeRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

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
            input: &RebuildDetectRequest,
            _output: &RebuildDetectResult,
        ) -> Result<(), DomainError> {
            self.persisted_run_ids
                .lock()
                .expect("record rebuild run ID")
                .push(input.run_id.clone());
            Ok(())
        }

        async fn semantic_neighbor_impacts(
            &self,
            _changed_truth_keys: &[String],
            _page_node_keys: &[String],
        ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
            Ok(vec![RebuildDependencyEvidence {
                page_node_key: "page:a".to_string(),
                reason_package: json!({"semantic_neighbor_widening": true}),
            }])
        }
    }

    #[tokio::test]
    async fn narrows_pages_before_rebuild_detect() {
        let repo = FakeRepo::default();
        let output = execute(
            &repo,
            &RebuildDetectRequest {
                run_id: "run-1".to_string(),
                changed_truth_keys: vec!["verified.rule_instance:passport".to_string()],
                page_node_keys: vec!["page:a".to_string(), "page:b".to_string()],
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
            .reason_package["semantic_neighbor_widening"] == true));
        let semantic_reason = &output
            .impacts
            .iter()
            .find(|impact| impact.page_node_key == "page:a")
            .expect("semantic widening impact")
            .reason_package;
        assert_eq!(
            semantic_reason["semantic_neighbor_evidence"],
            json!([{"semantic_neighbor_widening":true}])
        );
        assert_eq!(
            semantic_reason["changed_truth_keys"],
            json!(["verified.rule_instance:passport"])
        );
        assert_eq!(
            *repo.persisted_run_ids.lock().expect("persisted rebuild run ID"),
            vec!["run-1".to_string()]
        );
    }
}
