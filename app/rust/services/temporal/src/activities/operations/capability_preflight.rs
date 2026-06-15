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

async fn probe_qdrant_required_collections(
    required_collections: &[&str],
) -> (bool, BTreeMap<String, bool>) {
    let qdrant_url =
        std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let mut statuses = BTreeMap::new();
    let connect = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        qdrant_client_adapter::connect_qdrant(&qdrant_url),
    )
    .await;
    let Ok(Ok(client)) = connect else {
        return (false, statuses);
    };
    for collection in required_collections {
        let exists = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.collection_exists(*collection),
        )
        .await
        {
            Ok(Ok(value)) => value,
            _ => false,
        };
        statuses.insert((*collection).to_string(), exists);
    }
    (true, statuses)
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
