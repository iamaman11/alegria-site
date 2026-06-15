use primitives::errors::DomainError;

use super::neo4rs_adapter;

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

pub async fn ensure_graph_contract_if_required(context_key: &str) -> Result<(), DomainError> {
    let graph_required = env_flag("GRAPH_CAPABILITY_REQUIRED");
    let graph_query_required = env_flag("GRAPH_QUERY_REQUIRED");
    let graph_gds_required = env_flag("GRAPH_GDS_REQUIRED");
    if !graph_required || !graph_query_required {
        return Ok(());
    }

    let uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());

    let graph = neo4rs_adapter::connect_neo4j(&uri, &user, &password)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("neo4j graph contract connect failed for `{context_key}`: {err}"),
        })?;
    neo4rs_adapter::run_cypher(&graph, "RETURN $context_key AS context_key")
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("neo4j graph contract query failed for `{context_key}`: {err}"),
        })?;

    if graph_gds_required {
        let gds_queries = [
            "CALL gds.version() YIELD version RETURN version",
            "CALL gds.version() YIELD gdsVersion RETURN gdsVersion",
            "CALL gds.version()",
        ];
        let mut gds_error = String::new();
        for cypher in gds_queries {
            match neo4rs_adapter::run_cypher(&graph, cypher).await {
                Ok(_) => return Ok(()),
                Err(err) => gds_error = err.to_string(),
            }
        }
        return Err(DomainError::InfraUnavailable {
            message: format!("neo4j gds capability unavailable for `{context_key}`: {gds_error}"),
        });
    }

    Ok(())
}
