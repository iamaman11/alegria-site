use anyhow::{bail, Result};
use sqlx::Row;
use std::env;

use super::neo4rs_adapter::{connect_neo4j, query};
use super::sqlx_adapter::connect_pg;
use super::sqlx_context_bundle_adapter;

fn neo4j_cfg() -> (String, String, String) {
    (
        env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string()),
        env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string()),
        env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string()),
    )
}

fn pg_cfg() -> String {
    env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres_password@localhost:5433/alegria".to_string()
    })
}

pub async fn materialize_rule_instance(rule_instance_id: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((rule_instance_id, context_key, rule_type_key, concept_key, role_type, status)) =
        sqlx_context_bundle_adapter::load_rule_instance_materialization_row(&pg, rule_instance_id)
            .await?
    else {
        bail!("rule_instance not found: {rule_instance_id}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;

    let q = query(
        r#"
        MERGE (ctx:VisaContext {context_key: $context_key})
        MERGE (c:Concept {concept_key: $concept_key})
        MERGE (r:RuleInstance {rule_instance_id: $rule_instance_id})
        SET r.rule_type_key = $rule_type_key,
            r.role_type = $role_type,
            r.status = $status
        MERGE (ctx)-[:HAS_RULE]->(r)
        MERGE (r)-[:CONCERNS]->(c)
        "#,
    )
    .param("context_key", context_key)
    .param("concept_key", concept_key)
    .param("rule_instance_id", rule_instance_id)
    .param("rule_type_key", rule_type_key)
    .param("role_type", role_type)
    .param("status", status);
    graph.run(q).await?;

    Ok(())
}

pub async fn materialize_concept(concept_key: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((concept_key, concept_type, label_ru, status)) =
        sqlx_context_bundle_adapter::load_concept_materialization_row(&pg, concept_key).await?
    else {
        bail!("concept not found: {concept_key}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;
    let q = query(
        r#"
        MERGE (c:Concept {concept_key: $concept_key})
        SET c.concept_type = $concept_type,
            c.label_ru = $label_ru,
            c.status = $status
        "#,
    )
    .param("concept_key", concept_key)
    .param("concept_type", concept_type)
    .param("label_ru", label_ru)
    .param("status", status);
    graph.run(q).await?;
    Ok(())
}

pub async fn materialize_page_context(url_path: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((url_path, context_key, mapping_status)) =
        sqlx_context_bundle_adapter::load_page_context_materialization_row(&pg, url_path).await?
    else {
        bail!("page_context_map not found: {url_path}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;
    let q = query(
        r#"
        MERGE (p:Page {url_path: $url_path})
        SET p.mapping_status = $mapping_status
        MERGE (ctx:VisaContext {context_key: $context_key})
        MERGE (p)-[:ABOUT_CONTEXT]->(ctx)
        "#,
    )
    .param("url_path", url_path)
    .param("mapping_status", mapping_status)
    .param("context_key", context_key);
    graph.run(q).await?;
    Ok(())
}

pub async fn materialize_seo_artifact(artifact_type: &str, artifact_key: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;

    match artifact_type {
        "keyword_cluster" => materialize_keyword_cluster(&graph, &pg, artifact_key).await,
        "serp_pattern" => materialize_serp_pattern(&graph, &pg, artifact_key).await,
        "page_blueprint" => materialize_page_blueprint(&graph, &pg, artifact_key).await,
        "page_node" => materialize_page_node(&graph, &pg, artifact_key).await,
        "content_gap" => materialize_content_gap(&graph, &pg, artifact_key).await,
        "link_recommendation" => materialize_link_recommendation(&graph, &pg, artifact_key).await,
        "cannibalization_conflict" => {
            materialize_cannibalization_conflict(&graph, &pg, artifact_key).await
        }
        "page_brief" => materialize_page_brief(&graph, &pg, artifact_key).await,
        other => bail!("unsupported SEO artifact_type for Neo4j projection: {other}"),
    }
}

async fn materialize_keyword_cluster(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    cluster_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT cluster_key, scope_signature, seed_keyword, dominant_intent, status
        FROM site.keyword_clusters
        WHERE cluster_key = $1
        "#,
    )
    .bind(cluster_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("keyword_cluster not found: {cluster_key}");
    };

    let cluster_key: String = row.get("cluster_key");
    let scope_signature: String = row.get("scope_signature");
    let seed_keyword: String = row.get("seed_keyword");
    let dominant_intent: String = row.get("dominant_intent");
    let status: String = row.get("status");
    let q = query(
        r#"
        MERGE (kc:KeywordCluster {cluster_key: $cluster_key})
        SET kc.scope_signature = $scope_signature,
            kc.seed_keyword = $seed_keyword,
            kc.dominant_intent = $dominant_intent,
            kc.status = $status
        MERGE (intent:Intent {intent_key: $dominant_intent})
        MERGE (kc)-[:TARGETS_INTENT]->(intent)
        "#,
    )
    .param("cluster_key", cluster_key)
    .param("scope_signature", scope_signature)
    .param("seed_keyword", seed_keyword)
    .param("dominant_intent", dominant_intent.clone())
    .param("status", status);
    graph.run(q).await?;
    Ok(())
}

async fn materialize_serp_pattern(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    serp_pattern_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT serp_pattern_key, query_batch_key, scope_signature, query, pattern_type,
               dominant_intent, status
        FROM serp.serp_patterns
        WHERE serp_pattern_key = $1
        "#,
    )
    .bind(serp_pattern_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("serp_pattern not found: {serp_pattern_key}");
    };

    let q = query(
        r#"
        MERGE (sp:SERPPattern {serp_pattern_key: $serp_pattern_key})
        SET sp.query_batch_key = $query_batch_key,
            sp.scope_signature = $scope_signature,
            sp.query = $query,
            sp.pattern_type = $pattern_type,
            sp.dominant_intent = $dominant_intent,
            sp.status = $status
        MERGE (intent:Intent {intent_key: $dominant_intent})
        MERGE (sp)-[:TARGETS_INTENT]->(intent)
        "#,
    )
    .param("serp_pattern_key", row.get::<String, _>("serp_pattern_key"))
    .param("query_batch_key", row.get::<String, _>("query_batch_key"))
    .param("scope_signature", row.get::<String, _>("scope_signature"))
    .param("query", row.get::<String, _>("query"))
    .param("pattern_type", row.get::<String, _>("pattern_type"))
    .param("dominant_intent", row.get::<String, _>("dominant_intent"))
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;
    Ok(())
}

async fn materialize_page_blueprint(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    blueprint_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT blueprint_key, page_type_key, dominant_intent, scope_class,
               blueprint_version, status
        FROM site.page_blueprints
        WHERE blueprint_key = $1
        "#,
    )
    .bind(blueprint_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("page_blueprint not found: {blueprint_key}");
    };

    let q = query(
        r#"
        MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
        SET bp.page_type_key = $page_type_key,
            bp.dominant_intent = $dominant_intent,
            bp.scope_class = $scope_class,
            bp.blueprint_version = $blueprint_version,
            bp.status = $status
        MERGE (intent:Intent {intent_key: $dominant_intent})
        MERGE (bp)-[:TARGETS_INTENT]->(intent)
        "#,
    )
    .param("blueprint_key", row.get::<String, _>("blueprint_key"))
    .param("page_type_key", row.get::<String, _>("page_type_key"))
    .param("dominant_intent", row.get::<String, _>("dominant_intent"))
    .param("scope_class", row.get::<String, _>("scope_class"))
    .param(
        "blueprint_version",
        row.get::<i32, _>("blueprint_version") as i64,
    )
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;
    Ok(())
}

async fn materialize_page_node(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    page_node_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT page_node_key, scope_signature, keyword_cluster_key, blueprint_key,
               page_type_key, dominant_intent, canonical_slug, canonical_url_path,
               lifecycle_state
        FROM site.page_nodes
        WHERE page_node_key = $1
        "#,
    )
    .bind(page_node_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("page_node not found: {page_node_key}");
    };

    let page_node_key: String = row.get("page_node_key");
    let keyword_cluster_key: Option<String> = row.get("keyword_cluster_key");
    let blueprint_key: Option<String> = row.get("blueprint_key");
    let q = query(
        r#"
        MERGE (pn:PageNode {page_node_key: $page_node_key})
        SET pn.scope_signature = $scope_signature,
            pn.page_type_key = $page_type_key,
            pn.dominant_intent = $dominant_intent,
            pn.canonical_slug = $canonical_slug,
            pn.canonical_url_path = $canonical_url_path,
            pn.lifecycle_state = $lifecycle_state
        MERGE (intent:Intent {intent_key: $dominant_intent})
        MERGE (pn)-[:TARGETS_INTENT]->(intent)
        "#,
    )
    .param("page_node_key", page_node_key.clone())
    .param("scope_signature", row.get::<String, _>("scope_signature"))
    .param("page_type_key", row.get::<String, _>("page_type_key"))
    .param("dominant_intent", row.get::<String, _>("dominant_intent"))
    .param("canonical_slug", row.get::<String, _>("canonical_slug"))
    .param(
        "canonical_url_path",
        row.get::<String, _>("canonical_url_path"),
    )
    .param("lifecycle_state", row.get::<String, _>("lifecycle_state"));
    graph.run(q).await?;

    if let Some(cluster_key) = keyword_cluster_key {
        let q = query(
            r#"
            MERGE (kc:KeywordCluster {cluster_key: $cluster_key})
            MERGE (pn:PageNode {page_node_key: $page_node_key})
            MERGE (kc)-[:PROPOSES_PAGE]->(pn)
            "#,
        )
        .param("cluster_key", cluster_key)
        .param("page_node_key", page_node_key.clone());
        graph.run(q).await?;
    }
    if let Some(blueprint_key) = blueprint_key {
        let q = query(
            r#"
            MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
            MERGE (pn:PageNode {page_node_key: $page_node_key})
            MERGE (pn)-[:USES_BLUEPRINT]->(bp)
            "#,
        )
        .param("blueprint_key", blueprint_key)
        .param("page_node_key", page_node_key);
        graph.run(q).await?;
    }
    Ok(())
}

async fn materialize_content_gap(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    content_gap_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT content_gap_key, scope_signature, page_node_key, missing_topic, severity, status
        FROM site.content_gaps
        WHERE content_gap_key = $1
        "#,
    )
    .bind(content_gap_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("content_gap not found: {content_gap_key}");
    };

    let content_gap_key: String = row.get("content_gap_key");
    let page_node_key: Option<String> = row.get("page_node_key");
    let q = query(
        r#"
        MERGE (gap:ContentGap {content_gap_key: $content_gap_key})
        SET gap.scope_signature = $scope_signature,
            gap.missing_topic = $missing_topic,
            gap.severity = $severity,
            gap.status = $status
        "#,
    )
    .param("content_gap_key", content_gap_key.clone())
    .param("scope_signature", row.get::<String, _>("scope_signature"))
    .param("missing_topic", row.get::<String, _>("missing_topic"))
    .param("severity", row.get::<String, _>("severity"))
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;

    if let Some(page_node_key) = page_node_key {
        let q = query(
            r#"
            MERGE (pn:PageNode {page_node_key: $page_node_key})
            MERGE (gap:ContentGap {content_gap_key: $content_gap_key})
            MERGE (pn)-[:HAS_CONTENT_GAP]->(gap)
            "#,
        )
        .param("page_node_key", page_node_key)
        .param("content_gap_key", content_gap_key);
        graph.run(q).await?;
    }
    Ok(())
}

async fn materialize_link_recommendation(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    link_recommendation_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT link_recommendation_key, source_page_key, target_page_key, link_role,
               anchor_strategy, required_flag, score::text AS score_text, status
        FROM site.link_recommendations
        WHERE link_recommendation_key = $1
        "#,
    )
    .bind(link_recommendation_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("link_recommendation not found: {link_recommendation_key}");
    };

    let q = query(
        r#"
        MERGE (source:PageNode {page_node_key: $source_page_key})
        MERGE (target:PageNode {page_node_key: $target_page_key})
        MERGE (source)-[r:RECOMMENDS_LINK_TO {link_recommendation_key: $link_recommendation_key}]->(target)
        SET r.link_role = $link_role,
            r.anchor_strategy = $anchor_strategy,
            r.required_flag = $required_flag,
            r.score = $score,
            r.status = $status
        "#,
    )
    .param(
        "link_recommendation_key",
        row.get::<String, _>("link_recommendation_key"),
    )
    .param("source_page_key", row.get::<String, _>("source_page_key"))
    .param("target_page_key", row.get::<String, _>("target_page_key"))
    .param("link_role", row.get::<String, _>("link_role"))
    .param("anchor_strategy", row.get::<String, _>("anchor_strategy"))
    .param("required_flag", row.get::<bool, _>("required_flag"))
    .param("score", row.get::<String, _>("score_text"))
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;
    Ok(())
}

async fn materialize_cannibalization_conflict(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    conflict_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT conflict_key, page_key_a, page_key_b, conflict_reason, severity, status
        FROM site.cannibalization_conflicts
        WHERE conflict_key = $1
        "#,
    )
    .bind(conflict_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("cannibalization_conflict not found: {conflict_key}");
    };

    let q = query(
        r#"
        MERGE (a:PageNode {page_node_key: $page_key_a})
        MERGE (b:PageNode {page_node_key: $page_key_b})
        MERGE (a)-[r:CONFLICTS_WITH {conflict_key: $conflict_key}]->(b)
        SET r.conflict_reason = $conflict_reason,
            r.severity = $severity,
            r.status = $status
        "#,
    )
    .param("conflict_key", row.get::<String, _>("conflict_key"))
    .param("page_key_a", row.get::<String, _>("page_key_a"))
    .param("page_key_b", row.get::<String, _>("page_key_b"))
    .param("conflict_reason", row.get::<String, _>("conflict_reason"))
    .param("severity", row.get::<String, _>("severity"))
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;
    Ok(())
}

async fn materialize_page_brief(
    graph: &neo4rs::Graph,
    pg: &sqlx::PgPool,
    page_brief_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT page_brief_key, page_node_key, blueprint_key, title, meta_description, status
        FROM site.page_briefs
        WHERE page_brief_key = $1
        "#,
    )
    .bind(page_brief_key)
    .fetch_optional(pg)
    .await?
    else {
        bail!("page_brief not found: {page_brief_key}");
    };

    let q = query(
        r#"
        MERGE (brief:PageBrief {page_brief_key: $page_brief_key})
        SET brief.title = $title,
            brief.meta_description = $meta_description,
            brief.status = $status
        MERGE (pn:PageNode {page_node_key: $page_node_key})
        MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
        MERGE (pn)-[:HAS_BRIEF]->(brief)
        MERGE (brief)-[:USES_BLUEPRINT]->(bp)
        "#,
    )
    .param("page_brief_key", row.get::<String, _>("page_brief_key"))
    .param("page_node_key", row.get::<String, _>("page_node_key"))
    .param("blueprint_key", row.get::<String, _>("blueprint_key"))
    .param("title", row.get::<String, _>("title"))
    .param("meta_description", row.get::<String, _>("meta_description"))
    .param("status", row.get::<String, _>("status"));
    graph.run(q).await?;
    Ok(())
}
