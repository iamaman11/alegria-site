// Real Neo4j capability probe binds context identity through Bolt.

// Regression: the strict graph probe must send every Cypher parameter
// to the real Neo4j driver, not just interpolate it in diagnostic text.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn graph_capability_probe_binds_context_key() {
    let neo4j = Neo4jHarness::start().await.unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("NEO4J_URI".to_string(), neo4j.uri.clone()),
        ("NEO4J_USER".to_string(), neo4j.user.clone()),
        ("NEO4J_PASSWORD".to_string(), neo4j.password.clone()),
        ("GRAPH_CAPABILITY_REQUIRED".to_string(), "true".to_string()),
        ("GRAPH_QUERY_REQUIRED".to_string(), "true".to_string()),
        ("GRAPH_GDS_REQUIRED".to_string(), "false".to_string()),
    ]);
    graph_capability::ensure_graph_contract_if_required("graph-probe-regression")
        .await
        .expect("Neo4j query must receive the context_key parameter");
}
