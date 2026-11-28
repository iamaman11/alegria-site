use anyhow::{Context, Result};
use std::env;

use outbox_worker::{health, worker};
use postgres_store::sqlx_adapter::connect_pg;
use tracing::info;

fn parse_max_cycles(value: Option<&str>) -> Result<Option<u32>> {
    value
        .map(|raw| {
            raw.parse::<u32>()
                .context("invalid OUTBOX_MAX_CYCLES integer")
        })
        .transpose()
}

fn require_database_url(value: Option<&str>) -> Result<&str> {
    match value {
        Some(url) if !url.trim().is_empty() => Ok(url),
        _ => anyhow::bail!("DATABASE_URL must be set to a nonempty PostgreSQL DSN"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().skip(1).any(|arg| arg == "--healthcheck") {
        // The healthcheck runs inside the same container, without any extra
        // HTTP server, CLI dependency or additional network exposure.
        let ready = health::configured_file()
            .as_deref()
            .is_some_and(health::ready);
        std::process::exit(if ready { 0 } else { 1 });
    }
    tracing_subscriber::fmt().json().init();

    // A missing DSN is a deployment/configuration failure, never authority to
    // connect to the old local development database with hard-coded credentials.
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    require_database_url(Some(&database_url))?;
    let worker_id = env::var("OUTBOX_WORKER_ID")
        .unwrap_or_else(|_| format!("outbox-worker-{}", std::process::id()));
    let max_cycles = match env::var("OUTBOX_MAX_CYCLES") {
        Ok(value) => parse_max_cycles(Some(&value))?,
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("invalid OUTBOX_MAX_CYCLES setting"),
    };

    info!(worker_id, "starting outbox worker");

    // A failed PostgreSQL connection must not leave the predecessor's marker
    // looking healthy during startup. The worker loop subsequently owns the
    // marker through its RAII lease for all claim/dispatch/ACK paths.
    if let Some(file) = health::configured_file() {
        health::clear(&file)?;
    }
    let pool = connect_pg(&database_url).await?;
    worker::run_worker_loop(&pool, &database_url, &worker_id, max_cycles).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_max_cycles, require_database_url};

    #[test]
    fn invalid_cycle_limit_cannot_run_forever() {
        assert_eq!(parse_max_cycles(None).unwrap(), None);
        assert_eq!(parse_max_cycles(Some("0")).unwrap(), Some(0));
        assert_eq!(parse_max_cycles(Some("3")).unwrap(), Some(3));
        for invalid in ["", "-1", "oops", "4294967296"] {
            assert!(parse_max_cycles(Some(invalid)).is_err());
        }
    }

    #[test]
    fn missing_or_empty_database_url_does_not_fall_back_to_local_credentials() {
        for value in [None, Some(""), Some("   ")] {
            assert!(require_database_url(value).is_err());
        }
        assert_eq!(
            require_database_url(Some("postgres://user:secret@host/db")).unwrap(),
            "postgres://user:secret@host/db"
        );
    }
}
