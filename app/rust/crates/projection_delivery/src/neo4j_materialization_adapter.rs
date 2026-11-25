use anyhow::{bail, Context, Result};
use sqlx::Row;
use std::env;

use graph_neo4j::{connect_neo4j, projection, Graph};
use postgres_store::sqlx_adapter::connect_pg;
use postgres_store::sqlx_projection_rows;

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
        sqlx_projection_rows::load_rule_instance_materialization_row(&pg, rule_instance_id)
            .await?
    else {
        bail!("rule_instance not found: {rule_instance_id}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;

    projection::upsert_rule_instance(
        &graph,
        projection::RuleInstanceProjection {
            rule_instance_id,
            context_key,
            rule_type_key,
            concept_key,
            role_type,
            status,
        },
    )
    .await
}

pub async fn materialize_concept(concept_key: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((concept_key, concept_type, label_ru, status)) =
        sqlx_projection_rows::load_concept_materialization_row(&pg, concept_key).await?
    else {
        bail!("concept not found: {concept_key}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;
    projection::upsert_concept(
        &graph,
        projection::ConceptProjection {
            concept_key,
            concept_type,
            label_ru,
            status,
        },
    )
    .await
}

pub async fn materialize_page_context(url_path: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((url_path, context_key, mapping_status)) =
        sqlx_projection_rows::load_page_context_materialization_row(&pg, url_path).await?
    else {
        bail!("page_context_map not found: {url_path}");
    };

    let (uri, user, password) = neo4j_cfg();
    let graph = connect_neo4j(&uri, &user, &password).await?;
    projection::upsert_page_context(
        &graph,
        projection::PageContextProjection {
            url_path,
            context_key,
            mapping_status,
        },
    )
    .await
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
    graph: &Graph,
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

    projection::upsert_keyword_cluster(
        graph,
        projection::KeywordClusterProjection {
            cluster_key: row.get("cluster_key"),
            scope_signature: row.get("scope_signature"),
            seed_keyword: row.get("seed_keyword"),
            dominant_intent: row.get("dominant_intent"),
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_serp_pattern(
    graph: &Graph,
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

    projection::upsert_serp_pattern(
        graph,
        projection::SerpPatternProjection {
            serp_pattern_key: row.get("serp_pattern_key"),
            query_batch_key: row.get("query_batch_key"),
            scope_signature: row.get("scope_signature"),
            query_text: row.get("query"),
            pattern_type: row.get("pattern_type"),
            dominant_intent: row.get("dominant_intent"),
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_page_blueprint(
    graph: &Graph,
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

    projection::upsert_page_blueprint(
        graph,
        projection::PageBlueprintProjection {
            blueprint_key: row.get("blueprint_key"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            scope_class: row.get("scope_class"),
            blueprint_version: row.get::<i32, _>("blueprint_version") as i64,
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_page_node(
    graph: &Graph,
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

    projection::upsert_page_node(
        graph,
        projection::PageNodeProjection {
            page_node_key: row.get("page_node_key"),
            scope_signature: row.get("scope_signature"),
            keyword_cluster_key: row.get("keyword_cluster_key"),
            blueprint_key: row.get("blueprint_key"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            canonical_slug: row.get("canonical_slug"),
            canonical_url_path: row.get("canonical_url_path"),
            lifecycle_state: row.get("lifecycle_state"),
        },
    )
    .await
}

async fn materialize_content_gap(
    graph: &Graph,
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

    projection::upsert_content_gap(
        graph,
        projection::ContentGapProjection {
            content_gap_key: row.get("content_gap_key"),
            scope_signature: row.get("scope_signature"),
            page_node_key: row.get("page_node_key"),
            missing_topic: row.get("missing_topic"),
            severity: row.get("severity"),
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_link_recommendation(
    graph: &Graph,
    pg: &sqlx::PgPool,
    link_recommendation_key: &str,
) -> Result<()> {
    let Some(row) = sqlx::query(
        r#"
        SELECT link_recommendation_key, scope_signature, source_page_key, target_page_key, link_role,
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

    // PostgreSQL NUMERIC is transported as text to avoid a new decimal crate,
    // but Neo4j graph readback requires a native float. Never persist a text
    // score (silently interpreted as 0 by the current graph reader).
    let score_text: String = row.get("score_text");
    let score = score_text.parse::<f64>().with_context(|| {
        format!("invalid link recommendation score for {link_recommendation_key}")
    })?;
    if !score.is_finite() || !(0.0..=1.0).contains(&score) {
        bail!(
            "invalid link recommendation score outside finite [0,1] for {link_recommendation_key}"
        );
    }

    // A link outbox event can be claimed before the PageNode events. Materialize
    // both endpoints from their authoritative SQL rows before creating the
    // relationship; otherwise MERGE makes bare Neo4j nodes without scope and
    // the strict post-planning graph readback correctly rejects the link.
    let source_page_key: String = row.get("source_page_key");
    let target_page_key: String = row.get("target_page_key");
    materialize_page_node(graph, pg, &source_page_key).await?;
    materialize_page_node(graph, pg, &target_page_key).await?;

    projection::upsert_link_recommendation(
        graph,
        projection::LinkRecommendationProjection {
            link_recommendation_key: row.get("link_recommendation_key"),
            scope_signature: row.get("scope_signature"),
            source_page_key,
            target_page_key,
            link_role: row.get("link_role"),
            anchor_strategy: row.get("anchor_strategy"),
            required_flag: row.get("required_flag"),
            score,
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_cannibalization_conflict(
    graph: &Graph,
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

    projection::upsert_cannibalization_conflict(
        graph,
        projection::CannibalizationConflictProjection {
            conflict_key: row.get("conflict_key"),
            page_key_a: row.get("page_key_a"),
            page_key_b: row.get("page_key_b"),
            conflict_reason: row.get("conflict_reason"),
            severity: row.get("severity"),
            status: row.get("status"),
        },
    )
    .await
}

async fn materialize_page_brief(
    graph: &Graph,
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

    projection::upsert_page_brief(
        graph,
        projection::PageBriefProjection {
            page_brief_key: row.get("page_brief_key"),
            page_node_key: row.get("page_node_key"),
            blueprint_key: row.get("blueprint_key"),
            title: row.get("title"),
            meta_description: row.get("meta_description"),
            status: row.get("status"),
        },
    )
    .await
}
