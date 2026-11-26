//! Minimal disposable infrastructure for the PostgreSQL V3 owner E2E.
//! No Qdrant, Neo4j, legacy Worker, global environment overrides or provider
//! composition belong on this package's compile path.

use anyhow::{Context, Result};
use sqlx::{postgres::PgPoolOptions, PgPool};
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions};
use url::Url;
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

pub struct TemporalHarness {
    pub temporal_url: String,
    pub namespace: String,
    _postgres_container: ContainerAsync<GenericImage>,
    _temporal_container: ContainerAsync<GenericImage>,
    _schema_container: ContainerAsync<GenericImage>,
    _namespace_container: ContainerAsync<GenericImage>,
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
        // Readiness is proven through the real connection and complete schema
        // bootstrap, not a fixed sleep after the PostgreSQL-ready log marker.
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
        let pool = pool.expect("successful PostgreSQL connection returned no pool");
        let schema = include_str!("../../../../db/schema.sql");
        sqlx::raw_sql(schema)
            .execute(&pool)
            .await
            .context("apply schema.sql to postgres harness failed")?;
        Ok(Self {
            database_url,
            pool,
            _container: container,
        })
    }
}

impl TemporalHarness {
    pub async fn start() -> Result<Self> {
        let network = format!("pg-v3-e2e-temporal-{}", Uuid::new_v4());
        let postgres_image = GenericImage::new("postgres", "18.7-alpine")
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

        // Exact vendor one-shot schema installation used by production Compose.
        let schema_image = GenericImage::new("temporalio/admin-tools", "1.32.2")
            .with_wait_for(WaitFor::message_on_stdout("TEMPORAL_SCHEMAS: OK"))
            .with_entrypoint("/bin/sh")
            .with_cmd(["-ec", "/bin/sh /scripts/setup_schema.sh && sleep 3600"])
            .with_copy_to(
                "/scripts/setup_schema.sh",
                include_str!("../../../../../infra/temporal/setup_schema.sh")
                    .as_bytes()
                    .to_vec(),
            )
            .with_env_var("DB", "postgres12")
            .with_env_var("DB_PORT", "5432")
            .with_env_var("POSTGRES_USER", "temporal")
            .with_env_var("POSTGRES_PWD", "temporal_password")
            .with_env_var("SQL_PASSWORD", "temporal_password")
            .with_env_var("POSTGRES_SEEDS", postgres_ip.to_string())
            .with_network(&network);
        let schema_container = schema_image
            .start()
            .await
            .context("initialize current Temporal schema in test Postgres")?;

        let temporal_image = GenericImage::new("temporalio/server", "1.32.2")
            .with_exposed_port(7233.tcp())
            .with_env_var("DB", "postgres12")
            .with_env_var("DB_PORT", "5432")
            .with_env_var("POSTGRES_USER", "temporal")
            .with_env_var("POSTGRES_PWD", "temporal_password")
            .with_env_var("POSTGRES_SEEDS", postgres_ip.to_string())
            .with_env_var("BIND_ON_IP", "0.0.0.0")
            .with_env_var(
                "DYNAMIC_CONFIG_FILE_PATH",
                "config/dynamicconfig/development-sql.yaml",
            )
            .with_copy_to(
                "/etc/temporal/config/dynamicconfig/development-sql.yaml",
                include_str!("../../../../../infra/temporal/dynamicconfig.yaml")
                    .as_bytes()
                    .to_vec(),
            )
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
        let namespace_image = GenericImage::new("temporalio/admin-tools", "1.32.2")
            .with_wait_for(WaitFor::message_on_stdout("TEMPORAL_NAMESPACE_READY"))
            .with_entrypoint("/bin/sh")
            .with_cmd([
                "-ec",
                "set -eu; attempt=0; until temporal operator namespace describe --namespace default --address \"$TEMPORAL_ADDRESS\" >/dev/null 2>&1; do temporal operator namespace create --namespace default --address \"$TEMPORAL_ADDRESS\" >/dev/null 2>&1 || true; attempt=$((attempt + 1)); if [ \"$attempt\" -ge 60 ]; then echo 'TEMPORAL_NAMESPACE_BOOTSTRAP_FAILED' >&2; exit 1; fi; sleep 1; done; echo TEMPORAL_NAMESPACE_READY; sleep 3600",
            ])
            .with_env_var(
                "TEMPORAL_ADDRESS",
                format!("{}:7233", temporal_container.get_bridge_ip_address().await?),
            )
            .with_network(&network);
        let namespace_container = namespace_image
            .start()
            .await
            .context("ensure default Temporal namespace in test server")?;
        probe_temporal(&temporal_url, &namespace).await?;
        Ok(Self {
            temporal_url,
            namespace,
            _postgres_container: postgres_container,
            _temporal_container: temporal_container,
            _schema_container: schema_container,
            _namespace_container: namespace_container,
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

async fn temporal_client(temporal_url: &str, namespace: &str) -> Result<Client> {
    let connection_opts = ConnectionOptions::new(Url::parse(temporal_url)?)
        .identity("integration-pg-v3-e2e".to_string())
        .build();
    let connection = Connection::connect(connection_opts).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|err| anyhow::anyhow!("{err}"))
}
