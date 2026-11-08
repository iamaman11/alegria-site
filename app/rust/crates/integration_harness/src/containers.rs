#![cfg(feature = "e2e")]

use crate::env_overrides::{btree_env, EnvOverrideGuard};
use anyhow::{Context, Result};
use neo4rs::{query, Graph};
use qdrant_client::Qdrant;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::collections::BTreeMap;
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions};
use temporalio_sdk_core::Url;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[derive(Debug)]
pub struct PostgresHarness {
    pub database_url: String,
    pub pool: PgPool,
    _container: ContainerAsync<GenericImage>,
}

pub struct QdrantHarness {
    pub qdrant_url: String,
    pub qdrant_rest_url: String,
    pub client: Qdrant,
    _container: ContainerAsync<GenericImage>,
}

pub struct Neo4jHarness {
    pub uri: String,
    pub user: String,
    pub password: String,
    _container: ContainerAsync<GenericImage>,
}

pub struct TemporalHarness {
    pub temporal_url: String,
    pub namespace: String,
    _postgres_container: ContainerAsync<GenericImage>,
    _temporal_container: ContainerAsync<GenericImage>,
}

pub struct InfraHarness {
    pub postgres: PostgresHarness,
    pub qdrant: QdrantHarness,
    pub neo4j: Neo4jHarness,
    pub temporal: TemporalHarness,
}

impl PostgresHarness {
    pub async fn start() -> Result<Self> {
        let image = GenericImage::new("postgres", "18.6-alpine")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_DB", "alegria")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres_password");
        let container = image
            .start()
            .await
            .context("start postgres container failed")?;
        let port = container
            .get_host_port_ipv4(5432.tcp())
            .await
            .context("resolve postgres mapped port failed")?;
        sleep(Duration::from_secs(10)).await;
        let database_url = format!(
            "postgres://postgres:postgres_password@localhost:{port}/alegria?sslmode=disable"
        );
        let mut pool = None;
        for i in 0..10 {
            match PgPoolOptions::new()
                .max_connections(5)
                .connect(&database_url)
                .await
            {
                Ok(p) => {
                    pool = Some(p);
                    break;
                }
                Err(e) => {
                    if i == 9 {
                        return Err(anyhow::anyhow!(e)
                            .context("connect postgres harness failed after 10 retries"));
                    }
                    sleep(Duration::from_millis(2000)).await;
                }
            }
        }
        let pool = pool.unwrap();
        bootstrap_schema(&pool).await?;
        Ok(Self {
            database_url,
            pool,
            _container: container,
        })
    }
}

impl QdrantHarness {
    pub async fn start() -> Result<Self> {
        let image = GenericImage::new("qdrant/qdrant", "v1.20.0")
            .with_exposed_port(6334.tcp())
            .with_exposed_port(6333.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Qdrant HTTP listening on 6333"));
        let container = image
            .start()
            .await
            .context("start qdrant container failed")?;
        let port = container
            .get_host_port_ipv4(6334.tcp())
            .await
            .context("resolve qdrant mapped port failed")?;
        let qdrant_url = format!("http://127.0.0.1:{port}");
        let rest_port = container
            .get_host_port_ipv4(6333.tcp())
            .await
            .context("resolve qdrant REST mapped port failed")?;
        let qdrant_rest_url = format!("http://127.0.0.1:{rest_port}");
        let client = Qdrant::from_url(&qdrant_url)
            .skip_compatibility_check()
            .build()
            .context("build qdrant harness client failed")?;
        probe_qdrant(&client).await?;
        // Match the real bootstrap_retrieval_contract_collections.py inventory.
        // A fresh scope starts with *empty* collections, not seeded fake
        // neighbors or verified facts. Collection absence must never disguise
        // itself as a semantic/transport error in downstream strict searches.
        for collection in [
            "raw_chunks_4",
            "raw_chunks_ctx",
            "kb_canonical_4",
            "verified_rules_4",
            "editorial_topics_4",
            "seo_keyword_clusters_4",
            "whole_page_advisory_prototypes",
        ] {
            infrastructure::adapters::qdrant_client_adapter::ensure_default_dense_collection(
                &client, collection, 1024,
            )
            .await
            .with_context(|| format!("bootstrap real Qdrant collection {collection}"))?;
        }
        Ok(Self {
            qdrant_url,
            qdrant_rest_url,
            client,
            _container: container,
        })
    }
}

impl Neo4jHarness {
    pub async fn start() -> Result<Self> {
        // Exercise the same Neo4j/GDS capability contract as docker-compose.
        // The plugin is installed in this existing test container, not a
        // separate synthetic graph service or a production-side bypass.
        let image = GenericImage::new("neo4j", "2026.10.2")
            .with_exposed_port(7687.tcp())
            .with_wait_for(WaitFor::seconds(45))
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
        Ok(Self {
            uri,
            user,
            password,
            _container: container,
        })
    }
}

impl InfraHarness {
    pub async fn start() -> Result<Self> {
        let postgres = PostgresHarness::start().await?;
        let qdrant = QdrantHarness::start().await?;
        let neo4j = Neo4jHarness::start().await?;
        let temporal = TemporalHarness::start().await?;
        Ok(Self {
            postgres,
            qdrant,
            neo4j,
            temporal,
        })
    }

    pub fn runtime_env(&self) -> BTreeMap<String, String> {
        btree_env([
            (
                "DATABASE_URL".to_string(),
                self.postgres.database_url.clone(),
            ),
            ("QDRANT_URL".to_string(), self.qdrant.qdrant_url.clone()),
            (
                "QDRANT_REST_URL".to_string(),
                self.qdrant.qdrant_rest_url.clone(),
            ),
            ("NEO4J_URI".to_string(), self.neo4j.uri.clone()),
            ("NEO4J_USER".to_string(), self.neo4j.user.clone()),
            ("NEO4J_PASSWORD".to_string(), self.neo4j.password.clone()),
            (
                "TEMPORAL_URL".to_string(),
                self.temporal.temporal_url.clone(),
            ),
            (
                "TEMPORAL_NAMESPACE".to_string(),
                self.temporal.namespace.clone(),
            ),
        ])
    }

    pub fn apply_runtime_env(&self) -> EnvOverrideGuard {
        EnvOverrideGuard::apply(self.runtime_env())
    }
}

impl TemporalHarness {
    pub async fn start() -> Result<Self> {
        let network = format!("integration-harness-temporal-{}", Uuid::new_v4());
        let postgres_image = GenericImage::new("postgres", "18.6-alpine")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_DB", "temporal")
            .with_env_var("POSTGRES_USER", "temporal")
            .with_env_var("POSTGRES_PASSWORD", "temporal_password")
            .with_network(&network);
        let postgres_container = postgres_image
            .start()
            .await
            .context("start temporal postgres container failed")?;
        let postgres_ip = postgres_container
            .get_bridge_ip_address()
            .await
            .context("resolve temporal postgres bridge ip failed")?;

        let temporal_image = GenericImage::new("temporalio/auto-setup", "1.28")
            .with_exposed_port(7233.tcp())
            .with_wait_for(WaitFor::seconds(15))
            .with_env_var("DB", "postgres12")
            .with_env_var("DB_PORT", "5432")
            .with_env_var("POSTGRES_USER", "temporal")
            .with_env_var("POSTGRES_PWD", "temporal_password")
            .with_env_var("POSTGRES_SEEDS", postgres_ip.to_string())
            .with_env_var("BIND_ON_IP", "0.0.0.0")
            .with_network(&network);
        let temporal_container = temporal_image
            .start()
            .await
            .context("start temporal container failed")?;
        let port = temporal_container
            .get_host_port_ipv4(7233.tcp())
            .await
            .context("resolve temporal mapped port failed")?;
        let temporal_url = format!("http://127.0.0.1:{port}");
        let namespace = "default".to_string();
        probe_temporal(&temporal_url, &namespace).await?;
        Ok(Self {
            temporal_url,
            namespace,
            _postgres_container: postgres_container,
            _temporal_container: temporal_container,
        })
    }
}

async fn probe_temporal(temporal_url: &str, namespace: &str) -> Result<()> {
    let mut last_error = None;
    for _ in 0..20 {
        match temporal_client(temporal_url, namespace).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                last_error = Some(err);
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("temporal probe failed")))
        .context("probe temporal harness failed")
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

async fn probe_qdrant(client: &Qdrant) -> Result<()> {
    let mut last_error = None;
    for _ in 0..20 {
        match client.collection_exists("raw_chunks_4").await {
            Ok(_) => return Ok(()),
            Err(err) => last_error = Some(anyhow::anyhow!(err)),
        }
        sleep(Duration::from_secs(1)).await;
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("qdrant probe failed")))
        .context("probe qdrant harness failed")
}

async fn temporal_client(temporal_url: &str, namespace: &str) -> Result<Client> {
    let connection_opts = ConnectionOptions::new(Url::parse(temporal_url)?)
        .identity("integration-harness".to_string())
        .build();
    let connection = Connection::connect(connection_opts).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|err| anyhow::anyhow!("{err}"))
}

async fn bootstrap_schema(pool: &PgPool) -> Result<()> {
    let schema = include_str!("../../../../db/schema.sql");
    sqlx::raw_sql(schema)
        .execute(pool)
        .await
        .context("apply schema.sql to postgres harness failed")?;
    Ok(())
}
