use anyhow::Result;

pub mod planning_links;
pub mod projection;
pub mod schema;

pub use neo4rs::{query, Graph};

pub async fn connect_neo4j(uri: &str, user: &str, password: &str) -> Result<Graph> {
    let graph = Graph::new(uri, user, password)?;
    // Production-facing connections must never mutate an unconstrained graph:
    // concurrent MERGE can create duplicate identities before an Outbox ACK.
    // Explicit operator/harness bootstrap installs the constraints. This
    // read-only check is intentionally repeated after restarts/reconnections.
    schema::verify_projection_schema(&graph).await?;
    Ok(graph)
}

pub async fn run_cypher(graph: &Graph, cypher: &str) -> Result<()> {
    graph.run(query(cypher)).await?;
    Ok(())
}

// A live Bolt endpoint can need several seconds for its first query.
// Timeout and backend failures must never be reported as success.
pub async fn probe_ready_query(graph: &Graph) -> Result<()> {
    bounded_probe(
        std::time::Duration::from_secs(6),
        run_cypher(graph, "RETURN 1 AS ok"),
    )
    .await
}

async fn bounded_probe<F>(deadline: std::time::Duration, operation: F) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Neo4j readiness query timed out after {}s",
                deadline.as_secs()
            )
        })?
}

#[cfg(test)]
mod readiness_tests {
    use super::bounded_probe;
    use std::time::Duration;

    #[tokio::test]
    async fn success_is_not_faked_or_delayed() {
        bounded_probe(Duration::from_millis(50), async { Ok(()) })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn timed_out_query_fails_closed() {
        let result = bounded_probe(
            Duration::from_millis(1),
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn backend_query_error_is_not_hidden() {
        let result = bounded_probe(Duration::from_millis(50), async {
            anyhow::bail!("unreachable Bolt backend")
        })
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unreachable Bolt backend"));
    }
}
