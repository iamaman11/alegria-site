use anyhow::{bail, Result};
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
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres_password@localhost:5433/alegria".to_string())
}

pub async fn materialize_rule_instance(rule_instance_id: &str) -> Result<()> {
    let pg = connect_pg(&pg_cfg()).await?;
    let Some((rule_instance_id, context_key, rule_type_key, concept_key, role_type, status)) =
        sqlx_context_bundle_adapter::load_rule_instance_materialization_row(&pg, rule_instance_id).await?
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
