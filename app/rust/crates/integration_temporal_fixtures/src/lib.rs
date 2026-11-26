//! Authentic Temporal E2E server, with vendor PostgreSQL schema and namespace.
//! Kept independent of application providers and the broad integration harness.
#![cfg(feature = "e2e")]

use anyhow::{Context, Result};
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::time::{sleep, Duration};
use url::Url;
use uuid::Uuid;

pub struct TemporalHarness {
    pub temporal_url: String,
    pub namespace: String,
    _postgres_container: ContainerAsync<GenericImage>,
    _temporal_container: ContainerAsync<GenericImage>,
    _schema_container: ContainerAsync<GenericImage>,
    _namespace_container: ContainerAsync<GenericImage>,
}

impl TemporalHarness {
    pub async fn start() -> Result<Self> {
        let network = format!("integration-harness-temporal-{}", Uuid::new_v4());
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

        // Same vendor schema initialization as supported Compose, with the
        // actual script bytes. Docker's copied script may not be executable.
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
            .with_env_var("TEMPORAL_ADDRESS", format!("{}:7233", temporal_container.get_bridge_ip_address().await?))
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
        .identity("integration-harness".to_string())
        .build();
    let connection = Connection::connect(connection_opts).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|err| anyhow::anyhow!("{err}"))
}
