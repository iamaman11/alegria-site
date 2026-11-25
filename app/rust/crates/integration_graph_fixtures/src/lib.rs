//! Single-source Neo4j disposable fixture used by broad and isolated E2E.

#![cfg(feature = "e2e")]

use anyhow::{Context, Result};
use graph_neo4j::{query, Graph};
use testcontainers::{
    core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, GenericImage, ImageExt,
};
use tokio::time::{sleep, Duration};

pub struct Neo4jHarness {
    pub uri: String,
    pub user: String,
    pub password: String,
    _container: ContainerAsync<GenericImage>,
}

impl Neo4jHarness {
    pub async fn start() -> Result<Self> {
        // Genuine disposable Neo4j with GDS; production schema preflight is
        // verified before returning a handle to any test workload.
        let image = GenericImage::new("neo4j", "2026.10.2")
            .with_exposed_port(7687.tcp())
            .with_env_var("NEO4J_AUTH", "neo4j/neo4j_password")
            .with_env_var("NEO4J_PLUGINS", r#"["apoc", "graph-data-science"]"#)
            .with_env_var(
                "NEO4J_dbms_security_procedures_unrestricted",
                "apoc.*,gds.*",
            );
        let container = image
            .start()
            .await
            .context("start neo4j container failed")?;
        let port = container
            .get_host_port_ipv4(7687.tcp())
            .await
            .context("resolve neo4j mapped port failed")?;
        let uri = format!("127.0.0.1:{port}");
        let user = "neo4j".to_string();
        let password = "neo4j_password".to_string();
        probe_neo4j(&uri, &user, &password).await?;

        // Same managed schema installer that the broad harness used; the
        // production connect_neo4j refuses an uninitialized graph.
        let graph = Graph::new(&uri, &user, &password)?;
        graph_neo4j::schema::install_projection_schema(&graph)
            .await
            .context("install Neo4j projection uniqueness constraints")?;
        graph_neo4j::schema::verify_projection_schema(&graph)
            .await
            .context("verify Neo4j projection uniqueness constraints")?;
        Ok(Self {
            uri,
            user,
            password,
            _container: container,
        })
    }
}

async fn probe_neo4j(uri: &str, user: &str, password: &str) -> Result<()> {
    let mut last_error = None;
    for _ in 0..90 {
        match Graph::new(uri, user, password) {
            Ok(graph) => match graph.run(query("RETURN 1 AS ok")).await {
                Ok(_) => return Ok(()),
                Err(err) => last_error = Some(anyhow::anyhow!(err)),
            },
            Err(err) => last_error = Some(anyhow::anyhow!(err)),
        }
        sleep(Duration::from_secs(2)).await;
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("neo4j probe failed")))
        .context("probe neo4j harness failed")
}
