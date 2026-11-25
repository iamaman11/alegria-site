// Real Neo4j provider projection constraints; moved from the broad harness.

#[cfg(feature = "e2e")]
#[tokio::test]
async fn graph_capability_link_projection_rejects_cross_scope_and_missing_endpoints() {
    use graph_neo4j::projection::{
        upsert_link_recommendation, upsert_page_node, LinkRecommendationProjection,
        PageNodeProjection,
    };

    async fn count_links(graph: &graph_neo4j::Graph, key: &str) -> i64 {
        let mut rows = graph
            .execute(
                graph_neo4j::query(
                    "MATCH ()-[r:RECOMMENDS_LINK_TO {link_recommendation_key: $key}]->() \
                     RETURN count(r) AS link_count",
                )
                .param("key", key.to_string()),
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<i64>("link_count")
            .unwrap()
    }

    async fn count_page_nodes(graph: &graph_neo4j::Graph, key: &str) -> i64 {
        let mut rows = graph
            .execute(
                graph_neo4j::query(
                    "MATCH (p:PageNode {page_node_key: $key}) RETURN count(p) AS node_count",
                )
                .param("key", key.to_string()),
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<i64>("node_count")
            .unwrap()
    }

    let harness = Neo4jHarness::start().await.unwrap();
    let graph = graph_neo4j::Graph::new(&harness.uri, &harness.user, &harness.password).unwrap();
    let marker = Uuid::new_v4();
    let source_key = format!("source-{marker}");
    let same_scope_key = format!("same-scope-{marker}");
    let foreign_scope_key = format!("foreign-scope-{marker}");
    let source_scope = format!("scope-a-{marker}");
    let foreign_scope = format!("scope-b-{marker}");

    for (page_node_key, scope_signature) in [
        (&source_key, &source_scope),
        (&same_scope_key, &source_scope),
        (&foreign_scope_key, &foreign_scope),
    ] {
        upsert_page_node(
            &graph,
            PageNodeProjection {
                page_node_key: page_node_key.to_string(),
                scope_signature: scope_signature.to_string(),
                keyword_cluster_key: None,
                blueprint_key: None,
                page_type_key: "country_hub_page".into(),
                dominant_intent: "informational".into(),
                canonical_slug: page_node_key.to_string(),
                canonical_url_path: format!("/{page_node_key}/"),
                lifecycle_state: "approved".into(),
            },
        )
        .await
        .unwrap();
    }

    let make_link = |key: &str, target_page_key: String| LinkRecommendationProjection {
        link_recommendation_key: key.to_string(),
        scope_signature: source_scope.clone(),
        source_page_key: source_key.clone(),
        target_page_key,
        link_role: "contextual".into(),
        anchor_strategy: "contextual".into(),
        required_flag: true,
        score: 0.8,
        status: "active".into(),
    };

    // Both endpoints exist, but their PostgreSQL scopes differ. The actual
    // Neo4j writer must fail, so the leased Outbox event cannot be ACKed.
    let cross_scope_key = format!("cross-scope-link-{marker}");
    let error = upsert_link_recommendation(
        &graph,
        make_link(&cross_scope_key, foreign_scope_key),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("expected one same-scope"));
    assert_eq!(count_links(&graph, &cross_scope_key).await, 0);

    // MATCH of a missing endpoint must also fail, not return an accepted
    // zero-row Bolt result without writing the relationship.
    let missing_key = format!("missing-page-link-{marker}");
    let error = upsert_link_recommendation(
        &graph,
        make_link(&missing_key, format!("absent-page-{marker}")),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("expected one same-scope"));
    assert_eq!(count_links(&graph, &missing_key).await, 0);

    let valid_key = format!("valid-link-{marker}");
    upsert_link_recommendation(&graph, make_link(&valid_key, same_scope_key))
        .await
        .unwrap();
    assert_eq!(count_links(&graph, &valid_key).await, 1);

    // The production read model expects an f64, not the textual NUMERIC
    // representation. Verify the actual Bolt readback, not only query text.
    let planned = graph_neo4j::planning_links::load_planning_links(&graph, &source_scope)
        .await
        .unwrap();
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].source_page_key, source_key);
    assert_eq!(planned[0].score, 0.8);

    // The real Temporal Activity and a separate Outbox Worker may MERGE the
    // same graph identities concurrently. Neo4j MERGE by itself does not
    // guarantee uniqueness in that race. The harness installed the owner's
    // constraints before any writes, so a competing transaction may report
    // a safe conflict, but it must never create a duplicate node or edge.
    let concurrent_page_key = format!("concurrent-page-{marker}");
    let concurrent_page = PageNodeProjection {
        page_node_key: concurrent_page_key.clone(),
        scope_signature: source_scope.clone(),
        keyword_cluster_key: None,
        blueprint_key: None,
        page_type_key: "country_hub_page".into(),
        dominant_intent: "informational".into(),
        canonical_slug: concurrent_page_key.clone(),
        canonical_url_path: format!("/{concurrent_page_key}/"),
        lifecycle_state: "approved".into(),
    };
    let node_results = tokio::join!(
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page.clone()),
        upsert_page_node(&graph, concurrent_page),
    );
    assert!(
        [
            node_results.0.is_ok(),
            node_results.1.is_ok(),
            node_results.2.is_ok(),
            node_results.3.is_ok(),
            node_results.4.is_ok(),
            node_results.5.is_ok(),
            node_results.6.is_ok(),
            node_results.7.is_ok(),
        ]
        .iter()
        .any(|ok| *ok),
        "every concurrent PageNode writer failed"
    );
    assert_eq!(count_page_nodes(&graph, &concurrent_page_key).await, 1);

    let concurrent_link_key = format!("concurrent-link-{marker}");
    let concurrent_link = make_link(&concurrent_link_key, concurrent_page_key);
    let link_results = tokio::join!(
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link.clone()),
        upsert_link_recommendation(&graph, concurrent_link),
    );
    assert!(
        [
            link_results.0.is_ok(),
            link_results.1.is_ok(),
            link_results.2.is_ok(),
            link_results.3.is_ok(),
            link_results.4.is_ok(),
            link_results.5.is_ok(),
            link_results.6.is_ok(),
            link_results.7.is_ok(),
        ]
        .iter()
        .any(|ok| *ok),
        "every concurrent LinkRecommendation writer failed"
    );
    assert_eq!(count_links(&graph, &concurrent_link_key).await, 1);

    for invalid_score in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let invalid_key = format!("invalid-score-link-{marker}-{invalid_score}");
        let mut row = make_link(&invalid_key, format!("same-scope-{marker}"));
        row.score = invalid_score;
        let error = upsert_link_recommendation(&graph, row).await.unwrap_err();
        assert!(error.to_string().contains("finite number between 0 and 1"));
        assert_eq!(count_links(&graph, &invalid_key).await, 0);
    }

    // Migration preflight must also fail closed against historical data that
    // already violates an identity. This deliberately removes one constraint
    // ONLY after every writer assertion, on this test's disposable container.
    // No bootstrap operation may repair ambiguous identities on its own.
    graph
        .run(graph_neo4j::query(
            "DROP CONSTRAINT alegria_pagenode_page_node_key_unique IF EXISTS",
        ))
        .await
        .unwrap();
    let historical_key = format!("historical-duplicate-{marker}");
    graph
        .run(
            graph_neo4j::query(
                "CREATE (:PageNode {page_node_key: $key}), (:PageNode {page_node_key: $key})",
            )
            .param("key", historical_key.clone()),
        )
        .await
        .unwrap();
    assert_eq!(count_page_nodes(&graph, &historical_key).await, 2);
    let error = graph_neo4j::schema::verify_projection_schema(&graph)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("alegria_pagenode_page_node_key_unique"));
    let error = graph_neo4j::connect_neo4j(&harness.uri, &harness.user, &harness.password)
        .await
        .err()
        .expect("production Neo4j connection must reject missing identity constraint");
    assert!(error.to_string().contains("alegria_pagenode_page_node_key_unique"));
    let error = graph_neo4j::schema::install_projection_schema(&graph)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("alegria_pagenode_page_node_key_unique"));
    assert_eq!(count_page_nodes(&graph, &historical_key).await, 2);
}
