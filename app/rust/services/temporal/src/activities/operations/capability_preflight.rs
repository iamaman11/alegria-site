fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn required_retrieval_collections() -> [&'static str; 7] {
    [
        "raw_chunks_4",
        "raw_chunks_ctx",
        "kb_canonical_4",
        "verified_rules_4",
        "editorial_topics_4",
        "seo_keyword_clusters_4",
        "whole_page_advisory_prototypes",
    ]
}

fn required_graph_projections() -> [&'static str; 7] {
    [
        "keyword_cluster",
        "serp_pattern",
        "page_blueprint",
        "page_node",
        "content_gap",
        "link_recommendation",
        "page_brief",
    ]
}

async fn ensure_graph_required_for_phase(phase: &str) -> Result<(), DomainError> {
    graph_capability_adapter::ensure_graph_contract_if_required(phase)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("graph capability contract failed for `{phase}`: {err}"),
        })
}

/// These two vectors are produced for the current SEO scope. A point from a
/// different scope or a previous run must never certify a fresh publication.
fn scope_materialized_collection(collection: &str) -> bool {
    matches!(collection, "seo_keyword_clusters_4" | "editorial_topics_4")
}

// Canonical mapping and whole-page advisory can legitimately produce no
// searchable entity for a fresh scope. A valid-empty outcome requires the
// exact run's producer to have finished, plus a real Qdrant collection check.
// It cannot certify raw evidence, verified rules, or site planning vectors.
fn optional_retrieval_valid_empty(collection: &str, completed_producers: &[String]) -> bool {
    let producer = match collection {
        "kb_canonical_4" => "canonical_mapping",
        "whole_page_advisory_prototypes" => "whole_page_semantic_pass",
        _ => return false,
    };
    completed_producers.iter().any(|step| step == producer)
}

fn effective_collection_point_count(sql_count: i64, current_run_count: Option<u64>) -> i64 {
    current_run_count
        .map(|count| i64::try_from(count).unwrap_or(i64::MAX))
        .unwrap_or(sql_count)
}

#[cfg(test)]
mod scope_materialization_tests {
    use super::{effective_collection_point_count, scope_materialized_collection};

    #[test]
    fn another_site_or_run_cannot_certify_scoped_collection() {
        assert!(scope_materialized_collection("seo_keyword_clusters_4"));
        assert!(scope_materialized_collection("editorial_topics_4"));
        assert!(!scope_materialized_collection("kb_canonical_4"));
        assert_eq!(effective_collection_point_count(250, Some(0)), 0);
        assert_eq!(effective_collection_point_count(0, Some(2)), 2);
        assert_eq!(effective_collection_point_count(250, None), 250);
    }

    #[test]
    fn only_completed_optional_producers_may_certify_an_empty_collection() {
        use super::optional_retrieval_valid_empty;
        let producers = vec![
            "canonical_mapping".to_string(),
            "whole_page_semantic_pass".to_string(),
        ];
        assert!(optional_retrieval_valid_empty("kb_canonical_4", &producers));
        assert!(optional_retrieval_valid_empty(
            "whole_page_advisory_prototypes",
            &producers
        ));
        for required in [
            "raw_chunks_4",
            "raw_chunks_ctx",
            "verified_rules_4",
            "editorial_topics_4",
            "seo_keyword_clusters_4",
        ] {
            assert!(!optional_retrieval_valid_empty(required, &producers));
        }
        assert!(!optional_retrieval_valid_empty("kb_canonical_4", &[]));
        assert!(!optional_retrieval_valid_empty(
            "whole_page_advisory_prototypes",
            &["canonical_mapping".to_string()]
        ));
    }
}

async fn probe_qdrant_required_collections(
    required_collections: &[&str],
    scope_signature: &str,
    run_id: &str,
    require_materialized_projections: bool,
    probe_all_collections: bool,
) -> (bool, BTreeMap<String, bool>, BTreeMap<String, u64>) {
    let qdrant_url =
        std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let mut statuses = BTreeMap::new();
    // Missing provider results stay at zero; a global SQL counter is not a
    // substitute for a matching Qdrant point.
    let mut scope_counts = BTreeMap::new();
    if require_materialized_projections {
        for collection in required_collections
            .iter()
            .filter(|name| scope_materialized_collection(name))
        {
            scope_counts.insert((*collection).to_string(), 0);
        }
    }
    let connect = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        qdrant_client_adapter::connect_qdrant(&qdrant_url),
    )
    .await;
    let Ok(Ok(client)) = connect else {
        return (false, statuses, scope_counts);
    };
    let mut provider_ok = true;
    for collection in required_collections {
        if !probe_all_collections
            && !scope_materialized_collection(collection)
            && !matches!(*collection, "kb_canonical_4" | "whole_page_advisory_prototypes")
        {
            continue;
        }
        let exists = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.collection_exists(*collection),
        )
        .await
        {
            Ok(Ok(value)) => value,
            _ => {
                provider_ok = false;
                false
            }
        };
        statuses.insert((*collection).to_string(), exists);
        if require_materialized_projections
            && exists
            && scope_materialized_collection(collection)
        {
            use qdrant_client_adapter::{Condition, CountPointsBuilder, Filter};
            let request = CountPointsBuilder::new(*collection)
                .filter(Filter::must([
                    Condition::matches("scope_signature", scope_signature.to_string()),
                    Condition::matches("run_id", run_id.to_string()),
                ]))
                .exact(true);
            match tokio::time::timeout(std::time::Duration::from_secs(4), client.count(request))
                .await
            {
                Ok(Ok(result)) => {
                    scope_counts.insert(
                        (*collection).to_string(),
                        result.result.map(|value| value.count).unwrap_or(0),
                    );
                }
                _ => provider_ok = false,
            }
        }
    }
    (provider_ok, statuses, scope_counts)
}

#[derive(Debug, Clone, Default)]
struct Neo4jCapabilityProbe {
    neo4j_ready: bool,
    graph_query_ready: bool,
    graph_gds_ready: bool,
    errors: Vec<String>,
}

async fn probe_neo4j_capabilities() -> Neo4jCapabilityProbe {
    let uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
    let mut probe = Neo4jCapabilityProbe::default();

    let graph = match tokio::time::timeout(
        std::time::Duration::from_secs(4),
        neo4rs_adapter::connect_neo4j(&uri, &user, &password),
    )
    .await
    {
        Ok(Ok(graph)) => {
            probe.neo4j_ready = true;
            graph
        }
        Ok(Err(err)) => {
            probe.errors.push(format!("neo4j_connect:{err}"));
            return probe;
        }
        Err(_) => {
            probe.errors.push("neo4j_connect:timeout".to_string());
            return probe;
        }
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        neo4rs_adapter::run_cypher(&graph, "RETURN 1 AS ok"),
    )
    .await
    {
        Ok(Ok(_)) => probe.graph_query_ready = true,
        Ok(Err(err)) => probe.errors.push(format!("neo4j_query:{err}")),
        Err(_) => probe.errors.push("neo4j_query:timeout".to_string()),
    }

    let gds_queries = [
        "CALL gds.version() YIELD version RETURN version",
        "CALL gds.version() YIELD gdsVersion RETURN gdsVersion",
        "CALL gds.version()",
    ];
    let mut gds_error: Option<String> = None;
    for cypher in gds_queries {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            neo4rs_adapter::run_cypher(&graph, cypher),
        )
        .await
        {
            Ok(Ok(_)) => {
                probe.graph_gds_ready = true;
                gds_error = None;
                break;
            }
            Ok(Err(err)) => {
                gds_error = Some(err.to_string());
            }
            Err(_) => {
                gds_error = Some("timeout".to_string());
            }
        }
    }
    if !probe.graph_gds_ready {
        probe.errors.push(format!(
            "neo4j_gds:{}",
            gds_error.unwrap_or_else(|| "unknown".to_string())
        ));
    }

    probe
}

// Verify materialized Neo4j state, not just SQL source rows. Shared
// blueprints and briefs are attributed through the scope's PageNode edges.
fn neo4j_projection_count_query(name: &str) -> Option<&'static str> {
    match name {
        "keyword_cluster" => Some(
            "MATCH (n:KeywordCluster {scope_signature: $scope_signature}) RETURN count(n) AS projection_count",
        ),
        "serp_pattern" => Some(
            "MATCH (n:SERPPattern {scope_signature: $scope_signature}) RETURN count(n) AS projection_count",
        ),
        "page_blueprint" => Some(
            "MATCH (pn:PageNode {scope_signature: $scope_signature})-[:USES_BLUEPRINT]->(n:PageBlueprint) RETURN count(DISTINCT n) AS projection_count",
        ),
        "page_node" => Some(
            "MATCH (n:PageNode {scope_signature: $scope_signature}) RETURN count(n) AS projection_count",
        ),
        "content_gap" => Some(
            "MATCH (n:ContentGap {scope_signature: $scope_signature}) RETURN count(n) AS projection_count",
        ),
        "link_recommendation" => Some(
            "MATCH (a:PageNode {scope_signature: $scope_signature})-[r:RECOMMENDS_LINK_TO]->(b:PageNode {scope_signature: $scope_signature}) RETURN count(r) AS projection_count",
        ),
        "page_brief" => Some(
            "MATCH (pn:PageNode {scope_signature: $scope_signature})-[:HAS_BRIEF]->(n:PageBrief) RETURN count(DISTINCT n) AS projection_count",
        ),
        _ => None,
    }
}

async fn read_neo4j_scope_projection_counts(
    scope_signature: &str,
    names: &[&str],
) -> Result<BTreeMap<String, i64>, DomainError> {
    let uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
    let readback = async {
        let graph = neo4rs_adapter::connect_neo4j(&uri, &user, &password).await?;
        let mut counts = BTreeMap::new();
        for name in names {
            let cypher = neo4j_projection_count_query(name)
                .ok_or_else(|| anyhow::anyhow!("unsupported Neo4j projection: {name}"))?;
            let mut rows = graph
                .execute(
                    neo4rs_adapter::query(cypher)
                        .param("scope_signature", scope_signature.to_string()),
                )
                .await?;
            let row = rows
                .next()
                .await?
                .ok_or_else(|| anyhow::anyhow!("Neo4j count missing: {name}"))?;
            let count: i64 = row.get("projection_count")?;
            counts.insert((*name).to_string(), count);
        }
        Ok::<_, anyhow::Error>(counts)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(20), readback).await {
        Ok(Ok(counts)) => Ok(counts),
        Ok(Err(error)) => Err(DomainError::InfraUnavailable {
            message: format!("Neo4j scope projection readback failed: {error}"),
        }),
        Err(_) => Err(DomainError::InfraUnavailable {
            message: "Neo4j scope projection readback timed out".to_string(),
        }),
    }
}

#[cfg(test)]
mod neo4j_projection_readback_tests {
    use super::{neo4j_projection_count_query, required_graph_projections};

    #[test]
    fn every_required_projection_has_scope_bound_readback() {
        for name in required_graph_projections() {
            let cypher = neo4j_projection_count_query(name).expect("required projection query");
            assert!(cypher.contains("$scope_signature"), "{name}");
        }
        assert!(neo4j_projection_count_query("unknown").is_none());
    }
}

#[derive(Debug, Clone, Default)]
struct VoyageCapabilityProbe {
    embeddings_ready: bool,
    contextualized_ready: bool,
    rerank_ready: bool,
    errors: Vec<String>,
}

async fn probe_voyage_capabilities() -> VoyageCapabilityProbe {
    let api_key = match std::env::var("VOYAGE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            return VoyageCapabilityProbe {
                errors: vec!["missing VOYAGE_API_KEY".to_string()],
                ..VoyageCapabilityProbe::default()
            }
        }
    };
    let model = std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
    let context_model =
        std::env::var("VOYAGE_CONTEXT_MODEL").unwrap_or_else(|_| "voyage-context-3".to_string());
    let voyage = VoyageClient::new(api_key, model);
    let mut probe = VoyageCapabilityProbe::default();

    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        voyage.embed_batch_with_settings(
            &["voyage_capability_probe"],
            &VoyageEmbeddingOptions {
                input_type: Some(VoyageInputType::Query),
                output_dimension: Some(1024),
                output_dtype: Some(VoyageOutputDtype::Float),
                truncation: Some(false),
            },
        ),
    )
    .await
    {
        Ok(Ok(_)) => probe.embeddings_ready = true,
        Ok(Err(err)) => probe.errors.push(format!("embeddings:{err}")),
        Err(_) => probe.errors.push("embeddings:timeout".to_string()),
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        voyage.contextualized_embed(
            &[vec!["voyage_contextual_probe"]],
            &VoyageEmbeddingOptions {
                input_type: Some(VoyageInputType::Query),
                output_dimension: Some(1024),
                output_dtype: Some(VoyageOutputDtype::Float),
                truncation: Some(false),
            },
            Some(&context_model),
        ),
    )
    .await
    {
        Ok(Ok(_)) => probe.contextualized_ready = true,
        Ok(Err(err)) => probe.errors.push(format!("contextualized:{err}")),
        Err(_) => probe.errors.push("contextualized:timeout".to_string()),
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        voyage.rerank(
            "voyage rerank probe",
            &["voyage rerank probe document"],
            &VoyageRerankOptions {
                top_k: Some(1),
                truncation: Some(false),
                return_documents: false,
            },
            None,
        ),
    )
    .await
    {
        Ok(Ok(_)) => probe.rerank_ready = true,
        Ok(Err(err)) => probe.errors.push(format!("rerank:{err}")),
        Err(_) => probe.errors.push("rerank:timeout".to_string()),
    }

    probe
}
impl_json_runtime_payload_local!(
    TruthAdjudicationSweepInput,
    "alegria.runtime.json.TruthAdjudicationSweepInput"
);
impl_json_runtime_payload_local!(
    TruthAdjudicationSweepOutput,
    "alegria.runtime.json.TruthAdjudicationSweepOutput"
);
impl_json_runtime_payload_local!(
    VerifiedTruthWriteInput,
    "alegria.runtime.json.VerifiedTruthWriteInput"
);
impl_json_runtime_payload_local!(
    VerifiedTruthWriteOutput,
    "alegria.runtime.json.VerifiedTruthWriteOutput"
);
