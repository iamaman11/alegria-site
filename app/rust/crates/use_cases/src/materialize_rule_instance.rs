use anyhow::Result;
use infrastructure::adapters::neo4j_materialization_adapter;

pub async fn materialize_rule_instance(rule_instance_id: &str) -> Result<()> {
    neo4j_materialization_adapter::materialize_rule_instance(rule_instance_id).await
}
