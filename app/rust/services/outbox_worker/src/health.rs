//! Docker readiness is published only after the independent Outbox worker has
//! successfully queried PostgreSQL. Temporal must never treat a container
//! that merely started its process as an active projection-delivery owner.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const MAX_HEARTBEAT_AGE: Duration = Duration::from_secs(180);

/// The setting is opt-in: multiple E2E workers may share the test host.
/// In Compose it points to a container-private, ephemeral file.
pub fn configured_file() -> Option<PathBuf> {
    std::env::var_os("OUTBOX_READY_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Owns the readiness marker for exactly one worker run. Its destructor
/// revokes readiness even when a PostgreSQL claim/ACK returns early with an
/// error (the previous explicit shutdown-only cleanup missed those paths).
pub struct ReadinessLease {
    file: Option<PathBuf>,
}

impl ReadinessLease {
    pub fn acquire(file: Option<PathBuf>) -> Result<Self> {
        if let Some(path) = file.as_deref() {
            clear(path)?;
        }
        Ok(Self { file })
    }

    pub fn publish(&self) -> Result<()> {
        if let Some(path) = self.file.as_deref() {
            publish(path)?;
        }
        Ok(())
    }
}

impl Drop for ReadinessLease {
    fn drop(&mut self) {
        if let Some(path) = self.file.as_deref() {
            if let Err(error) = clear(path) {
                tracing::warn!(%error, path = ?path, "failed to revoke Outbox readiness");
            }
        }
    }
}

pub fn clear(file: &Path) -> Result<()> {
    match fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("clear Outbox readiness {file:?}")),
    }
}

fn now_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before UNIX epoch")?
        .as_millis())
}

/// Atomically publish a successful DB poll. A half-written readiness marker
/// must never cause a consumer to start without a live delivery owner.
pub fn publish(file: &Path) -> Result<()> {
    let timestamp = now_ms()?;
    let temporary = file.with_extension(format!("ready-{}", std::process::id()));
    fs::write(&temporary, timestamp.to_string())
        .with_context(|| format!("write Outbox readiness {temporary:?}"))?;
    fs::rename(&temporary, file).with_context(|| format!("publish Outbox readiness {file:?}"))
}

fn fresh(timestamp: u128, now: u128) -> bool {
    timestamp <= now && now - timestamp <= MAX_HEARTBEAT_AGE.as_millis()
}

pub fn ready(file: &Path) -> bool {
    fs::read_to_string(file)
        .ok()
        .and_then(|content| content.parse::<u128>().ok())
        .and_then(|timestamp| now_ms().ok().map(|now| fresh(timestamp, now)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{clear, fresh, publish, ready, ReadinessLease, MAX_HEARTBEAT_AGE};
    use std::fs;
    use std::time::Duration;

    #[test]
    fn health_marker_is_fail_closed_and_atomic() {
        let path = std::env::temp_dir().join(format!(
            "alegria-outbox-health-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        assert!(!ready(&path), "missing worker is never ready");
        publish(&path).unwrap();
        assert!(
            ready(&path),
            "successful first PG poll may make worker ready"
        );
        fs::write(&path, "not a timestamp").unwrap();
        assert!(!ready(&path), "corrupt health cannot unblock Temporal");
        clear(&path).unwrap();
        clear(&path).unwrap();
        assert!(!ready(&path), "shutdown must revoke readiness");
    }

    #[test]
    fn stale_or_future_heartbeats_cannot_prove_readiness() {
        let now = 1_000_000_u128;
        assert!(fresh(now, now));
        assert!(fresh(now - MAX_HEARTBEAT_AGE.as_millis(), now));
        assert!(!fresh(
            now - MAX_HEARTBEAT_AGE.as_millis() - Duration::from_millis(1).as_millis(),
            now
        ));
        assert!(!fresh(now + 1, now));
    }

    #[test]
    fn readiness_is_revoked_on_early_error_and_reacquired_on_restart() {
        let path = std::env::temp_dir().join(format!(
            "alegria-outbox-early-return-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        // A predecessor's marker cannot make the next process initially ready.
        publish(&path).unwrap();
        let worker_attempt = || -> anyhow::Result<()> {
            let lease = ReadinessLease::acquire(Some(path.clone()))?;
            assert!(!ready(&path));
            lease.publish()?;
            assert!(ready(&path));
            anyhow::bail!("simulated PostgreSQL ACK error");
        };
        assert!(worker_attempt().is_err());
        assert!(
            !ready(&path),
            "early error must revoke readiness immediately"
        );

        let lease = ReadinessLease::acquire(Some(path.clone())).unwrap();
        assert!(!ready(&path));
        lease.publish().unwrap();
        assert!(ready(&path));
        drop(lease);
        assert!(!ready(&path), "normal shutdown must also revoke readiness");
    }
}
