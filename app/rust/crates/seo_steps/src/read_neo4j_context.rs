use anyhow::{bail, Result};
use neo4rs::{query, Graph};

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub async fn read_neo4j_context(context_key: &str) -> Result<()> {
    let graph_required = env_flag("GRAPH_CAPABILITY_REQUIRED");
    let graph_query_required = env_flag("GRAPH_QUERY_REQUIRED");
    let graph_gds_required = env_flag("GRAPH_GDS_REQUIRED");
    if !graph_required || !graph_query_required {
        return Ok(());
    }

    let uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());

    let graph = Graph::new(&uri, &user, &password)?;
    graph
        .run(
            query("RETURN $context_key AS context_key")
                .param("context_key", context_key.to_string()),
        )
        .await?;
    if graph_gds_required {
        let gds_queries = [
            "CALL gds.version() YIELD version RETURN version",
            "CALL gds.version() YIELD gdsVersion RETURN gdsVersion",
            "CALL gds.version()",
        ];
        let mut gds_ready = false;
        let mut gds_error = String::new();
        for cypher in gds_queries {
            match graph.run(query(cypher)).await {
                Ok(_) => {
                    gds_ready = true;
                    break;
                }
                Err(err) => gds_error = err.to_string(),
            }
        }
        if !gds_ready {
            bail!("neo4j gds capability unavailable: {gds_error}");
        }
    }
    Ok(())
}
