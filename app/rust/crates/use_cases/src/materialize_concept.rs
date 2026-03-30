use anyhow::Result;
use infrastructure::adapters::neo4j_materialization_adapter;

pub async fn materialize_concept(concept_key: &str) -> Result<()> {
    neo4j_materialization_adapter::materialize_concept(concept_key).await
}
