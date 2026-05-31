use contracts::generated::alegria::temporal::v1::{
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, GraphPlanningContextState,
    GraphPlanningCoverageSignalState, GraphPlanningTopicSignalState,
    GraphPlanningTripleSignalState, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{PlanningRepository, SemanticLinkSearchPort, SerpSearchPort};
use std::collections::{HashMap, HashSet};
use runtime_models::GraphPlanningContext;

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
            .map(|cluster| contracts::generated::alegria::temporal::v1::KeywordClusterState {
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
            })
            .collect(),
        page_nodes: context
            .page_nodes
            .into_iter()
            .map(|node| contracts::generated::alegria::temporal::v1::PageNodeState {
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
            })
            .collect(),
        content_gaps: context
            .content_gaps
            .into_iter()
            .map(|gap| contracts::generated::alegria::temporal::v1::ContentGapState {
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
            })
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

async fn enrich_semantic_link_recommendations<S: SemanticLinkSearchPort>(
    search_port: &S,
    input: &LinkRecommendInputPayload,
    output: &mut LinkRecommendOutputPayload,
) -> Result<(), DomainError> {
    let retrieval_required = env_flag("RETRIEVAL_CAPABILITY_REQUIRED");
    if std::env::var("VOYAGE_API_KEY").is_err() {
        if retrieval_required {
            return Err(DomainError::InfraUnavailable {
                message:
                    "semantic link retrieval requires VOYAGE_API_KEY under hard-required retrieval contract"
                        .to_string(),
            });
        }
        return Ok(());
    }
    let max_semantic_links = input.max_links_per_page.max(3) as usize;
    let node_by_key = input
        .page_nodes
        .iter()
        .cloned()
        .map(|node| (node.page_node_key.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut seen = output
        .link_recommendations
        .iter()
        .map(|link| (link.source_page_key.clone(), link.target_page_key.clone()))
        .collect::<HashSet<_>>();

    for source in &input.page_nodes {
        if !linkable_state(&source.lifecycle_state) {
            continue;
        }
        let query = format!(
            "{} {} {} {}",
            source.canonical_url_path,
            source.page_type_key,
            source.dominant_intent,
            source.menu_group
        );
        let results = search_port.search_link_targets(&query, 12).await?;
        let mut added = 0usize;
        for candidate in results {
            if added >= max_semantic_links {
                break;
            }
            let Some(target) = node_by_key.get(&candidate.entity_key) else {
                continue;
            };
            if source.page_node_key == target.page_node_key
                || !linkable_state(&target.lifecycle_state)
                || seen.contains(&(source.page_node_key.clone(), target.page_node_key.clone()))
            {
                continue;
            }
            let same_scope = source.scope_signature == target.scope_signature;
            let same_family = source.canonical_url_family == target.canonical_url_family;
            if !same_scope && !same_family {
                continue;
            }
            let journey_bonus = if source.parent_page_node_key == target.page_node_key
                || target.parent_page_node_key == source.page_node_key
            {
                0.12
            } else {
                0.0
            };
            let hub_bonus =
                if source.page_type_key.contains("hub") || target.page_type_key.contains("hub") {
                    0.08
                } else {
                    0.0
                };
            let orphan_bonus = if target.parent_page_node_key.is_empty() {
                0.05
            } else {
                0.0
            };
            let anchor_strategy = if journey_bonus > 0.0 {
                "journey_contextual"
            } else {
                "semantic_contextual"
            };
            let link_role = if same_family {
                "semantic_family"
            } else {
                "semantic_contextual"
            };
            let semantic_score = clamp01(
                (candidate.score as f64 * 0.7)
                    + if same_scope { 0.12 } else { 0.0 }
                    + if same_family { 0.08 } else { 0.0 }
                    + journey_bonus
                    + hub_bonus
                    + orphan_bonus,
            );
            output.link_recommendations.push(LinkRecommendationState {
                link_recommendation_key: primitives::seo::seo_artifact_key(
                    "link_recommendation",
                    &[
                        &source.scope_signature,
                        &source.page_node_key,
                        &target.page_node_key,
                        link_role,
                        anchor_strategy,
                        "semantic@1",
                    ],
                ),
                scope_signature: source.scope_signature.clone(),
                source_page_key: source.page_node_key.clone(),
                target_page_key: target.page_node_key.clone(),
                link_role: link_role.to_string(),
                anchor_strategy: anchor_strategy.to_string(),
                required_flag: false,
                score: semantic_score,
                status: "candidate".to_string(),
                reason_code: "semantic_link_search".to_string(),
                topic_keys: Vec::new(),
                triple_refs: Vec::new(),
                graph_confidence: semantic_score,
                support_refs: Vec::new(),
            });
            seen.insert((source.page_node_key.clone(), target.page_node_key.clone()));
            added += 1;
        }
    }

    Ok(())
}

pub async fn run_serp_ingest<R: PlanningRepository, S: SerpSearchPort>(
    repo: &R,
    search_port: &S,
    input: &SerpIngestInputPayload,
) -> Result<SerpIngestOutputPayload, DomainError> {
    let mut output = seo_steps::serp_ingest_step::execute(input);
    let locale = input.scope.as_ref().map(|scope| scope.locale.as_str());
    let mut persisted_snapshots = 0u32;
    for (idx, query) in input
        .queries
        .iter()
        .map(|query| query.trim())
        .filter(|query| !query.is_empty())
        .enumerate()
    {
        let Some(response) = search_port
            .fetch_google_organic_live_advanced(locale, query)
            .await?
        else {
            continue;
        };
        repo.persist_live_serp_query_results(
            &input.run_id,
            &output.query_batch_key,
            idx,
            query,
            &response,
        )
        .await?;
        persisted_snapshots += 1;
    }
    output.persisted_snapshot_count = persisted_snapshots;
    repo.persist_serp_ingest_output(input, &output).await?;
    Ok(output)
}

pub async fn run_serp_normalize<R: PlanningRepository>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &SerpNormalizeInputPayload,
) -> Result<SerpNormalizeOutputPayload, DomainError> {
    let mut output = seo_steps::serp_normalize_step::execute(input);
    if voyage_retrieval_ready_or_optional()? {
        for pattern in &mut output.serp_patterns {
            let candidates = search_port.search_keyword_clusters(&pattern.query, 3).await?;
            let Some(best) = candidates.first() else {
                continue;
            };
            if !best.entity_key.trim().is_empty() {
                let cluster_ref = format!("cluster_ref:{}", best.entity_key);
                if pattern.evidence_ref.trim().is_empty() {
                    pattern.evidence_ref = cluster_ref;
                } else if !pattern.evidence_ref.contains(&cluster_ref) {
                    pattern.evidence_ref = format!("{}|{}", pattern.evidence_ref, cluster_ref);
                }
            }
            pattern.reliability_score = clamp01(
                pattern.reliability_score + (best.score as f64 * 0.15),
            );
            if pattern.status == "partial" && pattern.reliability_score >= 0.5 {
                pattern.status = "active".to_string();
            }
        }
    }
    repo.persist_serp_normalize_output(input, &output).await?;
    Ok(output)
}

pub async fn run_opportunity_build<R: PlanningRepository>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &OpportunityBuildInputPayload,
) -> Result<OpportunityBuildOutputPayload, DomainError> {
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context =
        to_graph_planning_context_state(repo.load_graph_planning_context(&input.run_id, &scope_signature).await?);
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::opportunity_build_step::execute(&enriched);
    if voyage_retrieval_ready_or_optional()? {
        for cluster in &mut output.keyword_clusters {
            let query = format!("{} {}", cluster.seed_keyword, cluster.dominant_intent);
            let candidates = search_port.search_keyword_clusters(&query, 4).await?;
            let Some(best) = candidates.first() else {
                continue;
            };
            cluster.graph_confidence = cluster.graph_confidence.max(best.score as f64);
            if cluster.reason_code == "serp_seed_cluster" {
                cluster.reason_code = "voyage_cluster_affinity".to_string();
            }
            for candidate in candidates.iter().take(3) {
                let support_ref = format!(
                    "qdrant://seo_keyword_clusters_4/{}",
                    candidate.entity_key
                );
                if !cluster.support_refs.contains(&support_ref) {
                    cluster.support_refs.push(support_ref);
                }
            }
        }
    }
    repo.persist_opportunity_build_output(&enriched, &output)
        .await?;
    Ok(output)
}

pub async fn run_ia_build<R: PlanningRepository>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &IaBuildInputPayload,
) -> Result<IaBuildOutputPayload, DomainError> {
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context =
        to_graph_planning_context_state(repo.load_graph_planning_context(&input.run_id, &scope_signature).await?);
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::ia_build_step::execute(&enriched);
    if voyage_retrieval_ready_or_optional()? {
        let mut cluster_owner = HashMap::<String, String>::new();
        for node in &output.page_nodes {
            if !node.keyword_cluster_key.trim().is_empty() {
                cluster_owner.insert(
                    node.keyword_cluster_key.clone(),
                    node.page_node_key.clone(),
                );
            }
        }
        for node in &output.page_nodes {
            if node.keyword_cluster_key.trim().is_empty() {
                continue;
            }
            let query = format!(
                "{} {} {}",
                node.canonical_url_path, node.page_type_key, node.dominant_intent
            );
            let candidates = search_port.search_keyword_clusters(&query, 4).await?;
            let Some(best) = candidates.first() else {
                continue;
            };
            if !high_similarity(best.score) || best.entity_key == node.keyword_cluster_key {
                continue;
            }
            let Some(owner) = cluster_owner.get(&best.entity_key) else {
                continue;
            };
            let conflict_key = primitives::seo::seo_artifact_key(
                "cannibalization_conflict",
                &[
                    &node.scope_signature,
                    &node.page_node_key,
                    owner,
                    "semantic_cluster_owner_overlap",
                ],
            );
            if output
                .cannibalization_conflicts
                .iter()
                .any(|conflict| conflict.conflict_key == conflict_key)
            {
                continue;
            }
            output.cannibalization_conflicts.push(
                contracts::generated::alegria::temporal::v1::CannibalizationConflictState {
                    conflict_key,
                    scope_signature: node.scope_signature.clone(),
                    page_key_a: node.page_node_key.clone(),
                    page_key_b: owner.clone(),
                    conflict_reason: "semantic_cluster_owner_overlap".to_string(),
                    severity: "medium".to_string(),
                    status: "open".to_string(),
                },
            );
        }
    }
    repo.persist_ia_build_output(&enriched, &output).await?;
    Ok(output)
}

pub async fn run_link_recommend<R: PlanningRepository, S: SemanticLinkSearchPort>(
    repo: &R,
    search_port: &S,
    input: &LinkRecommendInputPayload,
) -> Result<LinkRecommendOutputPayload, DomainError> {
    let scope_signature = input
        .page_nodes
        .first()
        .map(|node| node.scope_signature.clone())
        .unwrap_or_default();
    let graph_context =
        to_graph_planning_context_state(repo.load_graph_planning_context(&input.run_id, &scope_signature).await?);
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::link_recommend_step::execute(&enriched);
    enrich_semantic_link_recommendations(search_port, &enriched, &mut output).await?;
    repo.persist_link_recommend_output(&enriched, &output).await?;
    Ok(output)
}

pub async fn run_global_site_reconcile<R: PlanningRepository>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &GlobalSiteReconcileInputPayload,
) -> Result<GlobalSiteReconcileOutputPayload, DomainError> {
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context =
        to_graph_planning_context_state(repo.load_graph_planning_context(&input.run_id, &scope_signature).await?);
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::global_site_reconcile_step::execute(&enriched);
    if voyage_retrieval_ready_or_optional()? {
        let node_by_key = output
            .page_nodes
            .iter()
            .map(|node| (node.page_node_key.clone(), node))
            .collect::<HashMap<_, _>>();
        for source in &output.page_nodes {
            if !linkable_state(&source.lifecycle_state) {
                continue;
            }
            let query = format!(
                "{} {} {}",
                source.canonical_url_path, source.page_type_key, source.dominant_intent
            );
            let candidates = search_port.search_link_targets(&query, 4).await?;
            for candidate in candidates {
                if !high_similarity(candidate.score) || candidate.entity_key == source.page_node_key
                {
                    continue;
                }
                let Some(target) = node_by_key.get(&candidate.entity_key) else {
                    continue;
                };
                if !linkable_state(&target.lifecycle_state)
                    || source.dominant_intent != target.dominant_intent
                {
                    continue;
                }
                let conflict_key = primitives::seo::seo_artifact_key(
                    "cannibalization_conflict",
                    &[
                        &source.scope_signature,
                        &source.page_node_key,
                        &target.page_node_key,
                        "semantic_neighborhood_overlap",
                    ],
                );
                if !output.cannibalization_conflict_keys.contains(&conflict_key) {
                    output.cannibalization_conflict_keys.push(conflict_key);
                }
                break;
            }
        }
    }
    repo.persist_global_site_reconcile_output(&enriched, &output)
        .await?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use seo_ports::{
        GlobalNavigationPersistReport, OrganicSerpResponse, OrganicSerpResult, PlanningRepository,
        SemanticLinkCandidate, SemanticLinkSearchPort, SerpSearchPort,
    };

    #[derive(Default)]
    struct FakePlanningRepo;

    #[async_trait]
    impl PlanningRepository for FakePlanningRepo {
        async fn load_graph_planning_context(
            &self,
            _run_id: &str,
            _scope_signature: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(runtime_models::GraphPlanningContext::default())
        }

        async fn persist_serp_ingest_output(
            &self,
            _input: &SerpIngestInputPayload,
            _output: &SerpIngestOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_live_serp_query_results(
            &self,
            _run_id: &str,
            _query_batch_key: &str,
            _ordinal: usize,
            _query: &str,
            _response: &OrganicSerpResponse,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_serp_normalize_output(
            &self,
            _input: &SerpNormalizeInputPayload,
            _output: &SerpNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_opportunity_build_output(
            &self,
            _input: &OpportunityBuildInputPayload,
            _output: &OpportunityBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_ia_build_output(
            &self,
            _input: &IaBuildInputPayload,
            _output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_link_recommend_output(
            &self,
            _input: &LinkRecommendInputPayload,
            _output: &LinkRecommendOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_global_site_reconcile_output(
            &self,
            _input: &GlobalSiteReconcileInputPayload,
            _output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<GlobalNavigationPersistReport, DomainError> {
            Ok(GlobalNavigationPersistReport::default())
        }
    }

    struct FakeSerpSearchPort;

    #[async_trait]
    impl SerpSearchPort for FakeSerpSearchPort {
        async fn fetch_google_organic_live_advanced(
            &self,
            _locale: Option<&str>,
            _query: &str,
        ) -> Result<Option<OrganicSerpResponse>, DomainError> {
            Ok(Some(OrganicSerpResponse {
                raw_payload_utf8: "{}".to_string(),
                organic_results: vec![OrganicSerpResult {
                    rank: 1,
                    title: "Example".to_string(),
                    url: "https://example.com/visa".to_string(),
                    url_norm: "example.com/visa".to_string(),
                    domain_norm: "example.com".to_string(),
                    source_tier: "official".to_string(),
                    snippet: "snippet".to_string(),
                }],
            }))
        }
    }

    struct FakeSemanticSearchPort;

    #[async_trait]
    impl SemanticLinkSearchPort for FakeSemanticSearchPort {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "target".to_string(),
                score: 0.92,
            }])
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "cluster:spain-tourist".to_string(),
                score: 0.89,
            }])
        }
    }

    struct RecordingPlanningRepo {
        graph_context: runtime_models::GraphPlanningContext,
        opportunity_input: Mutex<Option<OpportunityBuildInputPayload>>,
        opportunity_output: Mutex<Option<OpportunityBuildOutputPayload>>,
        reconcile_output: Mutex<Option<GlobalSiteReconcileOutputPayload>>,
    }

    impl RecordingPlanningRepo {
        fn new(graph_context: runtime_models::GraphPlanningContext) -> Self {
            Self {
                graph_context,
                opportunity_input: Mutex::new(None),
                opportunity_output: Mutex::new(None),
                reconcile_output: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl PlanningRepository for RecordingPlanningRepo {
        async fn load_graph_planning_context(
            &self,
            _run_id: &str,
            _scope_signature: &str,
        ) -> Result<runtime_models::GraphPlanningContext, DomainError> {
            Ok(self.graph_context.clone())
        }

        async fn persist_serp_ingest_output(
            &self,
            _input: &SerpIngestInputPayload,
            _output: &SerpIngestOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_live_serp_query_results(
            &self,
            _run_id: &str,
            _query_batch_key: &str,
            _ordinal: usize,
            _query: &str,
            _response: &OrganicSerpResponse,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_serp_normalize_output(
            &self,
            _input: &SerpNormalizeInputPayload,
            _output: &SerpNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_opportunity_build_output(
            &self,
            input: &OpportunityBuildInputPayload,
            output: &OpportunityBuildOutputPayload,
        ) -> Result<(), DomainError> {
            *self.opportunity_input.lock().unwrap() = Some(input.clone());
            *self.opportunity_output.lock().unwrap() = Some(output.clone());
            Ok(())
        }

        async fn persist_ia_build_output(
            &self,
            _input: &IaBuildInputPayload,
            _output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_link_recommend_output(
            &self,
            _input: &LinkRecommendInputPayload,
            _output: &LinkRecommendOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_global_site_reconcile_output(
            &self,
            _input: &GlobalSiteReconcileInputPayload,
            output: &GlobalSiteReconcileOutputPayload,
        ) -> Result<GlobalNavigationPersistReport, DomainError> {
            *self.reconcile_output.lock().unwrap() = Some(output.clone());
            Ok(GlobalNavigationPersistReport::default())
        }
    }

    #[tokio::test]
    async fn planning_serp_ingest_runs_without_sql_adapter() {
        let repo = FakePlanningRepo;
        let search = FakeSerpSearchPort;
        let output = run_serp_ingest(
            &repo,
            &search,
            &SerpIngestInputPayload {
                run_id: "run-1".to_string(),
                query_batch_key: "batch-1".to_string(),
                queries: vec!["spain tourist visa".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(output.persisted_snapshot_count, 1);
    }

    #[tokio::test]
    async fn planning_link_recommend_can_enrich_links() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let repo = FakePlanningRepo;
        let search = FakeSemanticSearchPort;
        let output = run_link_recommend(
            &repo,
            &search,
            &LinkRecommendInputPayload {
                max_links_per_page: 3,
                page_nodes: vec![
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "source".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "hub".to_string(),
                        dominant_intent: "requirements".to_string(),
                        canonical_url_path: "/visa/spain".to_string(),
                        menu_group: "visa".to_string(),
                        canonical_url_family: "/visa".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "target".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "documents".to_string(),
                        canonical_url_path: "/visa/spain/documents".to_string(),
                        menu_group: "visa".to_string(),
                        canonical_url_family: "/visa".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(output.link_recommendations.len() >= 1);
        assert!(output
            .link_recommendations
            .iter()
            .any(|link| { link.source_page_key == "source" && link.target_page_key == "target" }));
    }

    #[tokio::test]
    async fn run_opportunity_build_loads_graph_context_and_persists_reason_payload() {
        let repo = RecordingPlanningRepo::new(runtime_models::GraphPlanningContext {
            scope_signature: "scope".to_string(),
            topic_signals: vec![runtime_models::GraphPlanningTopicSignal {
                topic_key: "spain-tourist-fee".to_string(),
                topic_type: "fee".to_string(),
                support_refs: vec!["editorial://topic/fee".to_string()],
                graph_confidence: 0.84,
            }],
            coverage_signals: vec![runtime_models::GraphPlanningCoverageSignal {
                page_node_key: String::new(),
                keyword_cluster_key: String::new(),
                covered_topic_keys: Vec::new(),
                missing_topic_keys: vec!["processing-time".to_string()],
                graph_confidence: 0.8,
            }],
            ..Default::default()
        });
        let output = run_opportunity_build(
            &repo,
            &FakeSemanticSearchPort,
            &OpportunityBuildInputPayload {
                run_id: "run-1".to_string(),
                scope: Some(contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    scope_signature: "scope".to_string(),
                    ..Default::default()
                }),
                serp_patterns: vec![
                    contracts::generated::alegria::temporal::v1::SerpPatternState {
                        serp_pattern_key: "p1".to_string(),
                        scope_signature: "scope".to_string(),
                        query: "Spain tourist visa fee".to_string(),
                        dominant_intent: "fee".to_string(),
                        reliability_score: 0.6,
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::SerpPatternState {
                        serp_pattern_key: "p2".to_string(),
                        scope_signature: "scope".to_string(),
                        query: "Spain tourist visa costs".to_string(),
                        dominant_intent: "fee".to_string(),
                        reliability_score: 0.58,
                        ..Default::default()
                    },
                ],
                graph_context: None,
            },
        )
        .await
        .unwrap();

        let persisted_input = repo.opportunity_input.lock().unwrap().clone().unwrap();
        assert!(persisted_input.graph_context.is_some());
        let persisted_output = repo.opportunity_output.lock().unwrap().clone().unwrap();
        assert_eq!(persisted_output.keyword_clusters.len(), 1);
        assert!(persisted_output
            .keyword_clusters
            .iter()
            .all(|cluster| cluster.reason_code == "graph_topic_family_merge"));
        assert!(persisted_output.content_gaps.iter().any(|gap| {
            gap.reason_code == "graph_missing_topic_coverage"
                && gap.topic_keys.iter().any(|topic| topic == "processing-time")
        }));
        assert_eq!(output.keyword_clusters.len(), persisted_output.keyword_clusters.len());
    }

    #[tokio::test]
    async fn run_global_site_reconcile_emits_graph_reasoned_gap_without_truth_side_effects() {
        let repo = RecordingPlanningRepo::new(runtime_models::GraphPlanningContext {
            scope_signature: "scope".to_string(),
            coverage_signals: vec![runtime_models::GraphPlanningCoverageSignal {
                page_node_key: "page-spain".to_string(),
                keyword_cluster_key: String::new(),
                covered_topic_keys: Vec::new(),
                missing_topic_keys: vec!["insurance".to_string()],
                graph_confidence: 0.72,
            }],
            ..Default::default()
        });
        let output = run_global_site_reconcile(
            &repo,
            &FakeSemanticSearchPort,
            &GlobalSiteReconcileInputPayload {
                run_id: "run-2".to_string(),
                scope: Some(contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    scope_signature: "scope".to_string(),
                    ..Default::default()
                }),
                page_nodes: vec![contracts::generated::alegria::temporal::v1::PageNodeState {
                    page_node_key: "page-spain".to_string(),
                    scope_signature: "scope".to_string(),
                    page_type_key: "hub_page".to_string(),
                    dominant_intent: "requirements".to_string(),
                    canonical_url_path: "/visa/spain".to_string(),
                    canonical_url_family: "/visa".to_string(),
                    lifecycle_state: "active".to_string(),
                    ..Default::default()
                }],
                link_recommendations: Vec::new(),
                reconcile_reason: "test@1".to_string(),
                graph_context: None,
            },
        )
        .await
        .unwrap();

        let persisted_output = repo.reconcile_output.lock().unwrap().clone().unwrap();
        assert!(persisted_output.content_gaps.iter().any(|gap| {
            gap.reason_code == "global_reconcile_missing_topic"
                && gap.topic_keys.iter().any(|topic| topic == "insurance")
        }));
        assert_eq!(output.page_nodes[0].lifecycle_state, "active");
    }

    struct OverlapSemanticSearchPort;

    #[async_trait]
    impl SemanticLinkSearchPort for OverlapSemanticSearchPort {
        async fn search_link_targets(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "target".to_string(),
                score: 0.91,
            }])
        }

        async fn search_keyword_clusters(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
            Ok(vec![SemanticLinkCandidate {
                entity_key: "cluster-b".to_string(),
                score: 0.9,
            }])
        }
    }

    #[tokio::test]
    async fn run_ia_build_adds_semantic_overlap_conflict_when_cluster_owner_diverges() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let repo = FakePlanningRepo;
        let output = run_ia_build(
            &repo,
            &OverlapSemanticSearchPort,
            &IaBuildInputPayload {
                run_id: "run-ia".to_string(),
                scope: Some(contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    scope_signature: "scope".to_string(),
                    locale: "ru-RU".to_string(),
                    country_code: "ES".to_string(),
                    visa_type: "tourist".to_string(),
                    ..Default::default()
                }),
                keyword_clusters: vec![
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-a".to_string(),
                        scope_signature: "scope".to_string(),
                        seed_keyword: "spain visa requirements".to_string(),
                        dominant_intent: "informational".to_string(),
                        status: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::KeywordClusterState {
                        cluster_key: "cluster-b".to_string(),
                        scope_signature: "scope".to_string(),
                        seed_keyword: "spain visa cost".to_string(),
                        dominant_intent: "informational".to_string(),
                        status: "active".to_string(),
                        ..Default::default()
                    },
                ],
                graph_context: None,
            },
        )
        .await
        .unwrap();
        assert!(output
            .cannibalization_conflicts
            .iter()
            .any(|conflict| conflict.conflict_reason == "semantic_cluster_owner_overlap"),
            "conflicts={:?}",
            output
                .cannibalization_conflicts
                .iter()
                .map(|conflict| (&conflict.conflict_key, &conflict.conflict_reason))
                .collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn run_global_site_reconcile_adds_semantic_overlap_conflict_key() {
        std::env::set_var("VOYAGE_API_KEY", "test");
        let repo = FakePlanningRepo;
        let output = run_global_site_reconcile(
            &repo,
            &OverlapSemanticSearchPort,
            &GlobalSiteReconcileInputPayload {
                run_id: "run-global".to_string(),
                scope: Some(contracts::generated::alegria::temporal::v1::SeoScopePayload {
                    scope_signature: "scope".to_string(),
                    ..Default::default()
                }),
                page_nodes: vec![
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "source".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "detail_page".to_string(),
                        dominant_intent: "informational".to_string(),
                        canonical_url_path: "/ru/visa/spain/source/".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                    contracts::generated::alegria::temporal::v1::PageNodeState {
                        page_node_key: "target".to_string(),
                        scope_signature: "scope".to_string(),
                        page_type_key: "detail_page".to_string(),
                        dominant_intent: "informational".to_string(),
                        canonical_url_path: "/ru/visa/spain/target/".to_string(),
                        lifecycle_state: "active".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!output.cannibalization_conflict_keys.is_empty());
    }
}
