use contracts::generated::alegria::temporal::v1::{
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    LinkRecommendationState, OpportunityBuildInputPayload, OpportunityBuildOutputPayload,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{PlanningRepository, SemanticLinkSearchPort, SerpSearchPort};
use std::collections::{HashMap, HashSet};

fn linkable_state(state: &str) -> bool {
    !matches!(state, "blocked" | "deprecated" | "stale" | "needs_rebuild")
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

async fn enrich_semantic_link_recommendations<S: SemanticLinkSearchPort>(
    search_port: &S,
    input: &LinkRecommendInputPayload,
    output: &mut LinkRecommendOutputPayload,
) -> Result<(), DomainError> {
    if std::env::var("VOYAGE_API_KEY").is_err() {
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
    input: &SerpNormalizeInputPayload,
) -> Result<SerpNormalizeOutputPayload, DomainError> {
    let output = seo_steps::serp_normalize_step::execute(input);
    repo.persist_serp_normalize_output(input, &output).await?;
    Ok(output)
}

pub async fn run_opportunity_build<R: PlanningRepository>(
    repo: &R,
    input: &OpportunityBuildInputPayload,
) -> Result<OpportunityBuildOutputPayload, DomainError> {
    let output = seo_steps::opportunity_build_step::execute(input);
    repo.persist_opportunity_build_output(input, &output)
        .await?;
    Ok(output)
}

pub async fn run_ia_build<R: PlanningRepository>(
    repo: &R,
    input: &IaBuildInputPayload,
) -> Result<IaBuildOutputPayload, DomainError> {
    let output = seo_steps::ia_build_step::execute(input);
    repo.persist_ia_build_output(&output).await?;
    Ok(output)
}

pub async fn run_link_recommend<R: PlanningRepository, S: SemanticLinkSearchPort>(
    repo: &R,
    search_port: &S,
    input: &LinkRecommendInputPayload,
) -> Result<LinkRecommendOutputPayload, DomainError> {
    let mut output = seo_steps::link_recommend_step::execute(input);
    enrich_semantic_link_recommendations(search_port, input, &mut output).await?;
    repo.persist_link_recommend_output(&output).await?;
    Ok(output)
}

pub async fn run_global_site_reconcile<R: PlanningRepository>(
    repo: &R,
    input: &GlobalSiteReconcileInputPayload,
) -> Result<GlobalSiteReconcileOutputPayload, DomainError> {
    let output = seo_steps::global_site_reconcile_step::execute(input);
    repo.persist_global_site_reconcile_output(input, &output)
        .await?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use seo_ports::{
        GlobalNavigationPersistReport, OrganicSerpResponse, OrganicSerpResult, PlanningRepository,
        SemanticLinkCandidate, SemanticLinkSearchPort, SerpSearchPort,
    };

    #[derive(Default)]
    struct FakePlanningRepo;

    #[async_trait]
    impl PlanningRepository for FakePlanningRepo {
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
            _output: &IaBuildOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_link_recommend_output(
            &self,
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
        std::env::remove_var("VOYAGE_API_KEY");
        assert!(output.link_recommendations.len() >= 1);
        assert!(output
            .link_recommendations
            .iter()
            .any(|link| { link.source_page_key == "source" && link.target_page_key == "target" }));
    }
}
