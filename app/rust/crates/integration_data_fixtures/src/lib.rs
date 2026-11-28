//! Shared real-container test fixtures, independent of runtime composition.
//! Production code must not acquire a dependency on this fixture-only crate.

#![cfg(any(feature = "pg", feature = "qdrant"))]

use anyhow::{Context, Result};
#[cfg(feature = "pg")]
use sqlx::{postgres::PgPoolOptions, types::Json, PgPool};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::time::{sleep, Duration};
#[cfg(feature = "qdrant")]
use vector_qdrant::Qdrant;

#[cfg(feature = "pg")]
#[derive(Debug)]
pub struct PostgresHarness {
    pub database_url: String,
    pub pool: PgPool,
    _container: ContainerAsync<GenericImage>,
}

#[cfg(feature = "qdrant")]
pub struct QdrantHarness {
    pub qdrant_url: String,
    pub qdrant_rest_url: String,
    pub client: Qdrant,
    _container: ContainerAsync<GenericImage>,
}

#[cfg(feature = "pg")]
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

#[cfg(feature = "qdrant")]
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

#[cfg(feature = "qdrant")]
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

#[cfg(feature = "pg")]
async fn bootstrap_schema(pool: &PgPool) -> Result<()> {
    let schema = include_str!("../../../../db/schema.sql");
    sqlx::raw_sql(schema)
        .execute(pool)
        .await
        .context("apply schema.sql to postgres harness failed")?;
    Ok(())
}

/// Seed the same real visa context and concept rows used by Truth and Outbox
/// transaction E2E. Shared by both owners to keep the SQL contract identical.
#[cfg(feature = "pg")]
pub async fn seed_truth_context_and_concept(
    pool: &PgPool,
    context_key: &str,
    concept_key: &str,
    concept_type: &str,
    label_ru: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO kb.visa_contexts
            (context_key, country_code, visa_family, visa_subtype, citizenship_code, status)
        VALUES
            ($1, 'ES', 'tourist', NULL, 'BY', 'active')
        ON CONFLICT (context_key) DO UPDATE
        SET status = EXCLUDED.status,
            updated_at = now()
        "#,
    )
    .bind(context_key)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
        VALUES ($1, $2, $3, 'active')
        ON CONFLICT (concept_key) DO UPDATE
        SET concept_type = EXCLUDED.concept_type,
            label_ru = EXCLUDED.label_ru,
            status = EXCLUDED.status,
            updated_at = now()
        "#,
    )
    .bind(concept_key)
    .bind(concept_type)
    .bind(label_ru)
    .execute(pool)
    .await
    .unwrap();
}

/// Insert actual raw.pages and raw.sections rows for SQL Truth extraction,
/// returning their canonical persisted identities instead of invented IDs.
#[cfg(feature = "pg")]
pub async fn seed_raw_page_with_section(
    pool: &PgPool,
    url: &str,
    dtype: &str,
    heading_path: &str,
    content_md: &str,
) -> (i64, i64) {
    let raw_html = format!("<html><body><main><p>{content_md}</p></main></body></html>");
    let page_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO raw.pages
            (url, domain, dtype, status_code, final_url, title, raw_html, raw_html_bytes, content_hash, content, source_observation, processed)
        VALUES
            ($1, $2, $3, 200, $1, 'Synthetic extraction source', $4, $5, $6, $7, $8, true)
        RETURNING id
        "#,
    )
    .bind(url)
    .bind(url.split('/').nth(2).unwrap_or("example.com").to_string())
    .bind(dtype)
    .bind(&raw_html)
    .bind(raw_html.len() as i32)
    .bind(format!("seed:raw-page:{}:{}", url, raw_html.len()))
    .bind(Json(serde_json::json!({
        "markdown": content_md,
        "headings": [heading_path],
        "links": [],
    })))
    .bind(Json(serde_json::json!({
        "seeded_by": "integration_harness_truth_runtime",
        "source_url": url,
    })))
    .fetch_one(pool)
    .await
    .unwrap();

    let section_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO raw.sections
            (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
        VALUES
            ($1, $2, 1, 0, 'paragraph', $3, $4)
        RETURNING id
        "#,
    )
    .bind(page_id)
    .bind(heading_path)
    .bind(content_md)
    .bind(format!("seed:raw-section:{}:{}", url, content_md.len()))
    .fetch_one(pool)
    .await
    .unwrap();

    (page_id, section_id)
}
