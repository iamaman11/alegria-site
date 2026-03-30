use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, PgPool};

pub type AlegriaPgPool = PgPool;

pub async fn connect_pg(database_url: &str) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?)
}
