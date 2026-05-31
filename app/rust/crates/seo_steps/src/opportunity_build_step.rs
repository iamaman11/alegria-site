use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::temporal::v1::{
    ContentGapState, GraphPlanningContextState, GraphPlanningTopicSignalState, KeywordClusterState,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload,
};

use crate::seo_step_support::{artifact_key, scope_signature};

fn topic_tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.trim().is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn topic_overlap(query: &str, signal: &GraphPlanningTopicSignalState) -> usize {
    let query_tokens = topic_tokens(query);
    let topic_tokens = topic_tokens(&signal.topic_key);
    query_tokens.intersection(&topic_tokens).count()
}

fn best_topic_family<'a>(
    query: &str,
    graph_context: Option<&'a GraphPlanningContextState>,
) -> Option<&'a GraphPlanningTopicSignalState> {
    graph_context.and_then(|context| {
        context
            .topic_signals
            .iter()
            .filter_map(|signal| {
                let overlap = topic_overlap(query, signal);
                (overlap > 0).then_some((overlap, signal))
            })
            .max_by(|(lhs_overlap, lhs_signal), (rhs_overlap, rhs_signal)| {
                lhs_overlap.cmp(rhs_overlap).then_with(|| {
                    lhs_signal
                        .graph_confidence
                        .total_cmp(&rhs_signal.graph_confidence)
                })
            })
            .map(|(_, signal)| signal)
    })
}

fn seed_or_topic_key(
    seed_keyword: &str,
    topic_signal: Option<&GraphPlanningTopicSignalState>,
) -> String {
    topic_signal
        .map(|signal| signal.topic_key.clone())
        .unwrap_or_else(|| seed_keyword.to_string())
}

fn cluster_ref_from_evidence(evidence_ref: &str) -> Option<String> {
    evidence_ref
        .split('|')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("cluster_ref:"))
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
}

pub fn execute(input: &OpportunityBuildInputPayload) -> OpportunityBuildOutputPayload {
    let fallback_scope = scope_signature(input.scope.as_ref());
    let graph_context = input.graph_context.as_ref();
    let mut keyword_clusters = Vec::new();
    let mut content_gaps = Vec::new();
    let mut seen_cluster_families = BTreeSet::new();
    let mut cluster_topics_by_family: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for pattern in &input.serp_patterns {
        let scope = if pattern.scope_signature.is_empty() {
            fallback_scope.clone()
        } else {
            pattern.scope_signature.clone()
        };
        let seed_keyword = pattern.query.trim().to_ascii_lowercase();
        if seed_keyword.is_empty() {
            continue;
        }
        let topic_signal = best_topic_family(&seed_keyword, graph_context);
        let family_key = cluster_ref_from_evidence(&pattern.evidence_ref)
            .unwrap_or_else(|| seed_or_topic_key(&seed_keyword, topic_signal));
        cluster_topics_by_family
            .entry(family_key.clone())
            .or_default()
            .extend(
                topic_signal
                    .into_iter()
                    .map(|signal| signal.topic_key.clone()),
            );
        if !seen_cluster_families.insert((scope.clone(), family_key.clone())) {
            continue;
        }
        let cluster_key = artifact_key(
            "keyword_cluster",
            &[
                &scope,
                &family_key,
                &pattern.dominant_intent,
                "seo_cluster@1",
            ],
        );
        let topic_keys = cluster_topics_by_family
            .get(&family_key)
            .cloned()
            .unwrap_or_default();
        let triple_refs = graph_context
            .into_iter()
            .flat_map(|context| context.triple_signals.iter())
            .filter(|triple| {
                topic_keys
                    .iter()
                    .any(|topic_key| triple.object_key == format!("topic:{topic_key}"))
            })
            .map(|triple| triple.triple_id.clone())
            .collect::<Vec<_>>();
        let support_refs = topic_signal
            .map(|signal| signal.support_refs.clone())
            .unwrap_or_default();
        let graph_confidence = topic_signal
            .map(|signal| signal.graph_confidence.max(pattern.reliability_score))
            .unwrap_or(pattern.reliability_score);
        keyword_clusters.push(KeywordClusterState {
            cluster_key: cluster_key.clone(),
            scope_signature: scope.clone(),
            seed_keyword: family_key.clone(),
            dominant_intent: if pattern.dominant_intent.is_empty() {
                "informational".to_string()
            } else {
                pattern.dominant_intent.clone()
            },
            status: "candidate".to_string(),
            cluster_version: 1,
            reason_code: if topic_signal.is_some() {
                "graph_topic_family_merge".to_string()
            } else if cluster_ref_from_evidence(&pattern.evidence_ref).is_some() {
                "voyage_step0_cluster_merge".to_string()
            } else {
                "serp_seed_cluster".to_string()
            },
            topic_keys,
            triple_refs,
            graph_confidence,
            support_refs,
        });
        if pattern.reliability_score < 0.5 {
            content_gaps.push(ContentGapState {
                content_gap_key: artifact_key(
                    "content_gap",
                    &[
                        &scope,
                        &family_key,
                        "low_reliability_serp",
                        "seo_content_gap@1",
                    ],
                ),
                scope_signature: scope.clone(),
                page_node_key: String::new(),
                missing_topic: family_key.clone(),
                severity: "medium".to_string(),
                status: "open".to_string(),
                reason_code: "low_reliability_serp".to_string(),
                topic_keys: Vec::new(),
                triple_refs: Vec::new(),
                graph_confidence: pattern.reliability_score,
                support_refs: Vec::new(),
            });
        }
    }

    let mut seen_gap_keys = content_gaps
        .iter()
        .map(|gap| gap.content_gap_key.clone())
        .collect::<BTreeSet<_>>();
    if let Some(graph_context) = graph_context {
        for signal in &graph_context.coverage_signals {
            for missing_topic in &signal.missing_topic_keys {
                let gap_key = artifact_key(
                    "content_gap",
                    &[
                        &fallback_scope,
                        missing_topic,
                        "graph_missing_topic_coverage",
                        "seo_content_gap@1",
                    ],
                );
                if !seen_gap_keys.insert(gap_key.clone()) {
                    continue;
                }
                content_gaps.push(ContentGapState {
                    content_gap_key: gap_key,
                    scope_signature: fallback_scope.clone(),
                    page_node_key: signal.page_node_key.clone(),
                    missing_topic: missing_topic.clone(),
                    severity: "high".to_string(),
                    status: "open".to_string(),
                    reason_code: "graph_missing_topic_coverage".to_string(),
                    topic_keys: vec![missing_topic.clone()],
                    triple_refs: graph_context
                        .triple_signals
                        .iter()
                        .filter(|triple| triple.object_key == format!("topic:{missing_topic}"))
                        .map(|triple| triple.triple_id.clone())
                        .collect(),
                    graph_confidence: signal.graph_confidence,
                    support_refs: vec![format!("coverage://{}", signal.page_node_key)],
                });
            }
        }
    }

    OpportunityBuildOutputPayload {
        keyword_clusters,
        content_gaps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        GraphPlanningCoverageSignalState, GraphPlanningTopicSignalState,
        GraphPlanningTripleSignalState,
    };

    #[test]
    fn semantically_aligned_patterns_collapse_into_one_cluster() {
        let output = execute(&OpportunityBuildInputPayload {
            serp_patterns: vec![
                contracts::generated::alegria::temporal::v1::SerpPatternState {
                    scope_signature: "scope".to_string(),
                    query: "spain visa refusal guide".to_string(),
                    dominant_intent: "informational".to_string(),
                    reliability_score: 0.7,
                    ..Default::default()
                },
                contracts::generated::alegria::temporal::v1::SerpPatternState {
                    scope_signature: "scope".to_string(),
                    query: "spain visa refusal appeal".to_string(),
                    dominant_intent: "informational".to_string(),
                    reliability_score: 0.74,
                    ..Default::default()
                },
            ],
            graph_context: Some(GraphPlanningContextState {
                topic_signals: vec![GraphPlanningTopicSignalState {
                    topic_key: "visa_refusal_pain_point".to_string(),
                    topic_type: "pain_point".to_string(),
                    support_refs: vec!["section://1".to_string()],
                    graph_confidence: 0.86,
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(output.keyword_clusters.len(), 1);
        assert_eq!(
            output.keyword_clusters[0].reason_code,
            "graph_topic_family_merge"
        );
    }

    #[test]
    fn missing_topic_coverage_emits_graph_reasoned_gap() {
        let output = execute(&OpportunityBuildInputPayload {
            graph_context: Some(GraphPlanningContextState {
                coverage_signals: vec![GraphPlanningCoverageSignalState {
                    page_node_key: "page-a".to_string(),
                    keyword_cluster_key: "cluster-a".to_string(),
                    covered_topic_keys: vec!["topic_a".to_string()],
                    missing_topic_keys: vec!["topic_b".to_string()],
                    graph_confidence: 0.77,
                }],
                triple_signals: vec![GraphPlanningTripleSignalState {
                    triple_id: "triple-1".to_string(),
                    subject_key: "section:1".to_string(),
                    relation_type: "MENTIONS_TOPIC".to_string(),
                    object_key: "topic:topic_b".to_string(),
                    support_refs: vec!["section://1".to_string()],
                    graph_confidence: 0.81,
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(output.content_gaps.len(), 1);
        assert_eq!(
            output.content_gaps[0].reason_code,
            "graph_missing_topic_coverage"
        );
        assert_eq!(
            output.content_gaps[0].triple_refs,
            vec!["triple-1".to_string()]
        );
    }
}
