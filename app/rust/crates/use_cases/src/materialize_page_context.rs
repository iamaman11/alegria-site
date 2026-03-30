use anyhow::Result;
use infrastructure::adapters::neo4j_materialization_adapter;

pub async fn materialize_page_context(url_path: &str) -> Result<()> {
    neo4j_materialization_adapter::materialize_page_context(url_path).await
}
