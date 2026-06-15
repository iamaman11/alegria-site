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

