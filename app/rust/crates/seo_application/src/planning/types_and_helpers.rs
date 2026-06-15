use contracts::generated::alegria::temporal::v1::{
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, GraphPlanningContextState,
    GraphPlanningCoverageSignalState, GraphPlanningTopicSignalState,
    GraphPlanningTripleSignalState, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use runtime_models::GraphPlanningContext;
use seo_ports::{
    GraphCapabilityPort, GraphReasoningPort, PlanningRepository, SemanticLinkSearchPort,
    SerpSearchPort,
};
use std::collections::{HashMap, HashSet};

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn linkable_state(state: &str) -> bool {
    !matches!(state, "blocked" | "deprecated" | "stale" | "needs_rebuild")
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn parse_graph_confidence(value: &str) -> f64 {
    value.trim().parse::<f64>().unwrap_or(0.0)
}

fn high_similarity(score: f32) -> bool {
    score >= 0.86
}

fn voyage_retrieval_ready_or_optional() -> Result<bool, DomainError> {
    let retrieval_required = env_flag("RETRIEVAL_CAPABILITY_REQUIRED");
    if std::env::var("VOYAGE_API_KEY").is_ok() {
        return Ok(true);
    }
    if retrieval_required {
        return Err(DomainError::InfraUnavailable {
            message:
                "planning retrieval requires VOYAGE_API_KEY under hard-required retrieval contract"
                    .to_string(),
        });
    }
    Ok(false)
}

async fn ensure_graph_required_for_phase<R: GraphCapabilityPort>(
    repo: &R,
    phase: &str,
) -> Result<(), DomainError> {
    repo.ensure_graph_contract(phase)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!(
                "graph capability contract failed for planning phase `{phase}`: {err}"
            ),
        })
}

fn to_graph_planning_context_state(context: GraphPlanningContext) -> GraphPlanningContextState {
    GraphPlanningContextState {
        scope_signature: context.scope_signature,
        topic_signals: context
            .topic_signals
            .into_iter()
            .map(|signal| GraphPlanningTopicSignalState {
                topic_key: signal.topic_key,
                topic_type: signal.topic_type,
                support_refs: signal.support_refs,
                graph_confidence: signal.graph_confidence,
            })
            .collect(),
        triple_signals: context
            .triple_signals
            .into_iter()
            .map(|signal| GraphPlanningTripleSignalState {
                triple_id: signal.triple_id,
                subject_key: signal.subject_key,
                relation_type: signal.relation_type,
                object_key: signal.object_key,
                support_refs: signal.support_refs,
                graph_confidence: signal.graph_confidence,
            })
            .collect(),
        coverage_signals: context
            .coverage_signals
            .into_iter()
            .map(|signal| GraphPlanningCoverageSignalState {
                page_node_key: signal.page_node_key,
                keyword_cluster_key: signal.keyword_cluster_key,
                covered_topic_keys: signal.covered_topic_keys,
                missing_topic_keys: signal.missing_topic_keys,
                graph_confidence: signal.graph_confidence,
            })
            .collect(),
        keyword_clusters: context
            .keyword_clusters
            .into_iter()
            .map(
                |cluster| contracts::generated::alegria::temporal::v1::KeywordClusterState {
                    cluster_key: cluster.cluster_key,
                    scope_signature: cluster.scope_signature,
                    seed_keyword: cluster.seed_keyword,
                    dominant_intent: cluster.dominant_intent,
                    status: cluster.status,
                    cluster_version: cluster.cluster_version,
                    reason_code: cluster.reason_code,
                    topic_keys: cluster.topic_keys,
                    triple_refs: cluster.triple_refs,
                    graph_confidence: parse_graph_confidence(&cluster.graph_confidence),
                    support_refs: cluster.support_refs,
                },
            )
            .collect(),
        page_nodes: context
            .page_nodes
            .into_iter()
            .map(
                |node| contracts::generated::alegria::temporal::v1::PageNodeState {
                    page_node_key: node.page_node_key,
                    scope_signature: node.scope_signature,
                    keyword_cluster_key: node.keyword_cluster_key,
                    blueprint_key: node.blueprint_key,
                    page_type_key: node.page_type_key,
                    dominant_intent: node.dominant_intent,
                    canonical_slug: node.canonical_slug,
                    canonical_url_path: node.canonical_url_path,
                    lifecycle_state: node.lifecycle_state,
                    ..Default::default()
                },
            )
            .collect(),
        content_gaps: context
            .content_gaps
            .into_iter()
            .map(
                |gap| contracts::generated::alegria::temporal::v1::ContentGapState {
                    content_gap_key: gap.content_gap_key,
                    scope_signature: gap.scope_signature,
                    page_node_key: gap.page_node_key,
                    missing_topic: gap.missing_topic,
                    severity: gap.severity,
                    status: gap.status,
                    reason_code: gap.reason_code,
                    topic_keys: gap.topic_keys,
                    triple_refs: gap.triple_refs,
                    graph_confidence: parse_graph_confidence(&gap.graph_confidence),
                    support_refs: gap.support_refs,
                },
            )
            .collect(),
        link_recommendations: context
            .link_recommendations
            .into_iter()
            .map(|link| LinkRecommendationState {
                link_recommendation_key: link.link_recommendation_key,
                scope_signature: link.scope_signature,
                source_page_key: link.source_page_key,
                target_page_key: link.target_page_key,
                link_role: link.link_role,
                anchor_strategy: link.anchor_strategy,
                required_flag: link.required_flag,
                score: link.score,
                status: link.status,
                reason_code: link.reason_code,
                topic_keys: link.topic_keys,
                triple_refs: link.triple_refs,
                graph_confidence: parse_graph_confidence(&link.graph_confidence),
                support_refs: link.support_refs,
            })
            .collect(),
    }
}

