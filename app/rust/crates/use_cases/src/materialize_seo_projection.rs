use anyhow::Result;
use infrastructure::adapters::neo4j_materialization_adapter;

pub async fn materialize_seo_artifact(artifact_type: &str, artifact_key: &str) -> Result<()> {
    neo4j_materialization_adapter::materialize_seo_artifact(artifact_type, artifact_key).await
}
