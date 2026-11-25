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

    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres_password@localhost:5433/alegria".to_string()
    });
    let worker_id = env::var("OUTBOX_WORKER_ID")
        .unwrap_or_else(|_| format!("outbox-worker-{}", std::process::id()));
    let max_cycles = match env::var("OUTBOX_MAX_CYCLES") {
        Ok(value) => parse_max_cycles(Some(&value))?,
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("invalid OUTBOX_MAX_CYCLES setting"),
    };

    info!(worker_id, "starting outbox worker");

    let pool = connect_pg(&database_url).await?;
    worker::run_worker_loop(&pool, &database_url, &worker_id, max_cycles).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_max_cycles;

    #[test]
    fn invalid_cycle_limit_cannot_run_forever() {
        assert_eq!(parse_max_cycles(None).unwrap(), None);
        assert_eq!(parse_max_cycles(Some("0")).unwrap(), Some(0));
        assert_eq!(parse_max_cycles(Some("3")).unwrap(), Some(3));
        for invalid in ["", "-1", "oops", "4294967296"] {
            assert!(parse_max_cycles(Some(invalid)).is_err());
        }
    }
}
