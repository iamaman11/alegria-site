//! Shared real-container test fixtures, independent of runtime composition.
//! Production code must not acquire a dependency on this fixture-only crate.

#![cfg(feature = "e2e")]

use anyhow::{Context, Result};
use sqlx::{postgres::PgPoolOptions, PgPool};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::time::{sleep, Duration};
use vector_qdrant::Qdrant;

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

impl PostgresHarness {
    pub async fn start() -> Result<Self> {
        let image = GenericImage::new("postgres", "18.7-alpine")
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
        // The container marker is only an initial readiness signal; the pool
        // and authentic schema bootstrap are the actual readiness contract.
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
        let image = GenericImage::new("qdrant/qdrant", "v1.20.1")
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
        // Identical real collection inventory to production's retrieval
        // bootstrap. These are empty physical Qdrant collections, not stubs.
        for collection in [
            "raw_chunks_4",
            "raw_chunks_ctx",
            "kb_canonical_4",
            "verified_rules_4",
            "editorial_topics_4",
            "seo_keyword_clusters_4",
            "whole_page_advisory_prototypes",
        ] {
            vector_qdrant::ensure_default_dense_collection(&client, collection, 1024)
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

async fn bootstrap_schema(pool: &PgPool) -> Result<()> {
    let schema = include_str!("../../../../db/schema.sql");
    sqlx::raw_sql(schema)
        .execute(pool)
        .await
        .context("apply schema.sql to postgres harness failed")?;
    Ok(())
}
