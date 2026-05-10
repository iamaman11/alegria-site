use anyhow::Result;
pub use tokio_postgres::{Client, Config, NoTls, Row};

pub async fn connect_tokio_postgres(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    dbname: &str,
) -> Result<(
    Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
)> {
    let mut cfg = Config::new();
    cfg.host(host)
        .port(port)
        .user(user)
        .password(password)
        .dbname(dbname);

    let (client, connection) = cfg.connect(NoTls).await?;
    let handle = tokio::spawn(async move { connection.await });
    Ok((client, handle))
}
