fn empty_payload() -> RawValue {
    RawValue::default()
}

fn default_database_url() -> String {
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres_password@localhost:5433/alegria".into())
}

fn expert_migration_workflows_enabled() -> bool {
    std::env::var("ALLOW_EXPERT_MIGRATION_WORKFLOWS")
        .ok()
        .as_deref()
        == Some("true")
}

fn write_report(report_path: &str, payload: &Value) -> Result<PathBuf> {
    let out = PathBuf::from(report_path);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create report dir failed: {}", parent.display()))?;
    }
    fs::write(&out, serde_json::to_vec_pretty(payload)?)
        .with_context(|| format!("write report failed: {}", out.display()))?;
    Ok(out)
}

fn parse_reason_json(reason: &str) -> Value {
    serde_json::from_str::<Value>(reason).unwrap_or_else(|_| json!({ "raw_reason": reason }))
}

fn fingerprint_vector(text: &str) -> Vec<f32> {
    let digest = blake3_hex(text.as_bytes());
    let mut vector = Vec::with_capacity(16);
    for chunk in digest.as_bytes().chunks(2).take(16) {
        let Ok(hex) = std::str::from_utf8(chunk) else {
            continue;
        };
        let value = u8::from_str_radix(hex, 16).unwrap_or(0);
        vector.push((value as f32 / 127.5) - 1.0);
    }
    if vector.is_empty() {
        vector.push(0.0);
    }
    vector
}

async fn persist_seo_site_build_input(
    database_url: Option<String>,
    run_id: &str,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    queries: Vec<String>,
    query_batch_key: Option<String>,
    run_mode: String,
) -> Result<()> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    register_site_build_input(
        &repo,
        &SeoSiteBuildRegistrationRequest {
            run_id: run_id.to_string(),
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            queries,
            query_batch_key,
            run_mode: Some(normalize_run_mode(&run_mode).to_string()),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(())
}

fn env_set(name: &str) -> bool {
    env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn env_flag(name: &str) -> bool {
    env::var(name)
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

fn llm_configured() -> bool {
    env_set("VERTEX_GEMINI_PROJECT")
        || env_set("GOOGLE_CLOUD_PROJECT")
        || env_set("GCLOUD_PROJECT")
        || env_set("VERTEX_GEMINI_ACCESS_TOKEN")
        || env_set("GOOGLE_APPLICATION_CREDENTIALS")
        || env_set("OPENAI_API_KEY")
        || env_set("ANTHROPIC_API_KEY")
        || env_set("GEMINI_API_KEY")
        || env_set("GOOGLE_API_KEY")
        || env_set("SEO_LLM_LOCAL_ENDPOINT")
}

async fn probe_required_qdrant_collections(
    qdrant_url: &str,
    required_collections: &[&str],
) -> Result<BTreeMap<String, bool>> {
    let client = qdrant_client_adapter::connect_qdrant(qdrant_url).await?;
    let mut statuses = BTreeMap::new();
    for collection in required_collections {
        statuses.insert(
            (*collection).to_string(),
            client.collection_exists(*collection).await?,
        );
    }
    Ok(statuses)
}

#[derive(Debug, Clone, Default)]
struct Neo4jCapabilityProbe {
    neo4j_ready: bool,
    graph_query_ready: bool,
    graph_gds_ready: bool,
    errors: Vec<String>,
}

async fn probe_neo4j_capabilities() -> Neo4jCapabilityProbe {
    let neo4j_uri = env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let neo4j_user = env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let neo4j_password =
        env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
    let mut probe = Neo4jCapabilityProbe::default();

    let graph = match tokio::time::timeout(
        Duration::from_secs(4),
        neo4rs_adapter::connect_neo4j(&neo4j_uri, &neo4j_user, &neo4j_password),
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
        Duration::from_secs(3),
        neo4rs_adapter::run_cypher(&graph, "RETURN 1 AS ok"),
    )
    .await
    {
        Ok(Ok(())) => probe.graph_query_ready = true,
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
            Duration::from_secs(3),
            neo4rs_adapter::run_cypher(&graph, cypher),
        )
        .await
        {
            Ok(Ok(())) => {
                probe.graph_gds_ready = true;
                gds_error = None;
                break;
            }
            Ok(Err(err)) => gds_error = Some(err.to_string()),
            Err(_) => gds_error = Some("timeout".to_string()),
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
    let api_key = match env::var("VOYAGE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            return VoyageCapabilityProbe {
                errors: vec!["missing VOYAGE_API_KEY".to_string()],
                ..VoyageCapabilityProbe::default()
            }
        }
    };
    let model = env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
    let context_model =
        env::var("VOYAGE_CONTEXT_MODEL").unwrap_or_else(|_| "voyage-context-3".to_string());
    let voyage = VoyageClient::new(api_key, model);
    let mut probe = VoyageCapabilityProbe::default();

    match tokio::time::timeout(
        Duration::from_secs(5),
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
        Duration::from_secs(5),
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
        Duration::from_secs(5),
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
