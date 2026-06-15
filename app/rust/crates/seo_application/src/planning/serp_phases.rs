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

pub async fn run_serp_normalize<R: PlanningRepository + GraphCapabilityPort>(
    repo: &R,
    search_port: &impl SemanticLinkSearchPort,
    input: &SerpNormalizeInputPayload,
) -> Result<SerpNormalizeOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "serp_normalize").await?;
    let mut output = seo_steps::serp_normalize_step::execute(input);
    if voyage_retrieval_ready_or_optional()? {
        let clusters = search_port
            .cluster_demand_queries(&input.queries)
            .await
            .unwrap_or_default();
        let mut query_cluster = HashMap::<String, (String, f32)>::new();
        for cluster in clusters {
            for query in cluster.member_queries {
                query_cluster.insert(
                    query.to_ascii_lowercase(),
                    (cluster.cluster_key.clone(), cluster.confidence),
                );
            }
        }
        for pattern in &mut output.serp_patterns {
            if let Some((cluster_key, confidence)) =
                query_cluster.get(&pattern.query.to_ascii_lowercase())
            {
                let cluster_ref = format!("cluster_ref:{cluster_key}");
                if pattern.evidence_ref.trim().is_empty() {
                    pattern.evidence_ref = cluster_ref;
                } else if !pattern.evidence_ref.contains(&cluster_ref) {
                    pattern.evidence_ref = format!("{}|{}", pattern.evidence_ref, cluster_ref);
                }
                pattern.reliability_score =
                    clamp01(pattern.reliability_score + (*confidence as f64 * 0.12));
            }
            let candidates = search_port
                .search_keyword_clusters(&pattern.query, 3)
                .await
                .unwrap_or_default();
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
            pattern.reliability_score =
                clamp01(pattern.reliability_score + (best.score as f64 * 0.15));
            if pattern.status == "partial" && pattern.reliability_score >= 0.5 {
                pattern.status = "active".to_string();
            }
        }
    }
    repo.persist_serp_normalize_output(input, &output).await?;
    Ok(output)
}
