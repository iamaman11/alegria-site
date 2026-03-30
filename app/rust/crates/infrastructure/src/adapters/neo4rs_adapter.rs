use anyhow::Result;

pub use neo4rs::{query, Graph};

pub async fn connect_neo4j(uri: &str, user: &str, password: &str) -> Result<Graph> {
    Ok(Graph::new(uri, user, password)?)
}

pub async fn run_cypher(graph: &Graph, cypher: &str) -> Result<()> {
    graph.run(query(cypher)).await?;
    Ok(())
}

pub async fn init_constraints(_graph: &Graph) -> Result<()> {
    Ok(())
}
