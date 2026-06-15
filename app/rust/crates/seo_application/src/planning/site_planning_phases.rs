pub async fn run_opportunity_build<
    R: PlanningRepository + GraphReasoningPort + GraphCapabilityPort,
>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &OpportunityBuildInputPayload,
) -> Result<OpportunityBuildOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "opportunity_build").await?;
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context = to_graph_planning_context_state(
        repo.load_planning_graph_context(&scope_signature, &input.run_id)
            .await?,
    );
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
                let support_ref =
                    format!("qdrant://seo_keyword_clusters_4/{}", candidate.entity_key);
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

pub async fn run_ia_build<R: PlanningRepository + GraphReasoningPort + GraphCapabilityPort>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &IaBuildInputPayload,
) -> Result<IaBuildOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "ia_build").await?;
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context = to_graph_planning_context_state(
        repo.load_planning_graph_context(&scope_signature, &input.run_id)
            .await?,
    );
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::ia_build_step::execute(&enriched);
    if voyage_retrieval_ready_or_optional()? {
        let mut cluster_owner = HashMap::<String, String>::new();
        for node in &output.page_nodes {
            if !node.keyword_cluster_key.trim().is_empty() {
                cluster_owner.insert(node.keyword_cluster_key.clone(), node.page_node_key.clone());
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

pub async fn run_link_recommend<
    R: PlanningRepository + GraphReasoningPort + GraphCapabilityPort,
    S: SemanticLinkSearchPort,
>(
    repo: &R,
    search_port: &S,
    input: &LinkRecommendInputPayload,
) -> Result<LinkRecommendOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "link_recommend").await?;
    let scope_signature = input
        .page_nodes
        .first()
        .map(|node| node.scope_signature.clone())
        .unwrap_or_default();
    let graph_context = to_graph_planning_context_state(
        repo.load_planning_graph_context(&scope_signature, &input.run_id)
            .await?,
    );
    let mut enriched = input.clone();
    enriched.graph_context = Some(graph_context);
    let mut output = seo_steps::link_recommend_step::execute(&enriched);
    enrich_semantic_link_recommendations(search_port, &enriched, &mut output).await?;
    repo.persist_link_recommend_output(&enriched, &output)
        .await?;
    Ok(output)
}

pub async fn run_global_site_reconcile<
    R: PlanningRepository + GraphReasoningPort + GraphCapabilityPort,
>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &GlobalSiteReconcileInputPayload,
) -> Result<GlobalSiteReconcileOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "global_site_reconcile").await?;
    let scope_signature = input
        .scope
        .as_ref()
        .map(|scope| scope.scope_signature.clone())
        .unwrap_or_default();
    let graph_context = to_graph_planning_context_state(
        repo.load_planning_graph_context(&scope_signature, &input.run_id)
            .await?,
    );
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

