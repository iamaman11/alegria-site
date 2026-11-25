#![cfg(feature = "e2e")]

use crate::env_overrides::{btree_env, EnvOverrideGuard};
use anyhow::{Context, Result};
pub use integration_data_fixtures::{PostgresHarness, QdrantHarness};
pub use integration_graph_fixtures::Neo4jHarness;
pub use integration_temporal_fixtures::TemporalHarness;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::Instant;

pub struct InfraHarness {
    pub postgres: PostgresHarness,
    pub qdrant: QdrantHarness,
    pub neo4j: Neo4jHarness,
    pub temporal: TemporalHarness,
}

impl InfraHarness {
    pub async fn start() -> Result<Self> {
        let started = Instant::now();
        // These four disposables have independent ports and credentials. Start
        // them concurrently: readiness is proven by each service's own probe,
        // not by completing unrelated service startup first.
        let (postgres, qdrant, neo4j, temporal) = tokio::try_join!(
            start_with_readiness("postgres", started, PostgresHarness::start()),
            start_with_readiness("qdrant", started, QdrantHarness::start()),
            start_with_readiness("neo4j", started, Neo4jHarness::start()),
            start_with_readiness("temporal", started, TemporalHarness::start()),
        )?;
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

async fn start_with_readiness<T>(
    stage: &str,
    started: Instant,
    startup: impl Future<Output = Result<T>>,
) -> Result<T> {
    let value = startup.await?;
    eprintln!(
        "CI_HARNESS_READY stage={} elapsed_ms={}",
        stage,
        started.elapsed().as_millis()
    );
    Ok(value)
}
