//! Genuine A14 Outbox/PG transaction fencing and durable provider-delivery E2E.
//! Only the nine owner tests live here; the broad harness no longer includes
//! their source or its `outbox_worker` dev-dependency.

#[cfg(all(test, feature = "e2e"))]
#[allow(unused, warnings)]
mod tests {
    use integration_data_fixtures::{PostgresHarness, QdrantHarness};
    use integration_harness::{env_overrides, stub_servers};
    use serial_test::serial;
    use std::{
        collections::BTreeMap,
        env,
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::Duration,
    };
    use uuid::Uuid;

    struct ChildGuard {
        child: Child,
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn spawn_outbox_worker(envs: &BTreeMap<String, String>) -> ChildGuard {
        // The CI job builds and attests the exact-SHA binary as an artifact.
        // No nested Cargo invocation, stale target fallback, or synthetic
        // provider work is permitted in these mandatory owner tests.
        let binary = env::var_os("INTEGRATION_OUTBOX_WORKER_BIN")
            .map(PathBuf::from)
            .expect("INTEGRATION_OUTBOX_WORKER_BIN must point to the prebuilt exact-SHA worker");
        assert!(binary.is_file(), "outbox worker binary missing: {}", binary.display());
        eprintln!("CI_WORKER_BINARY_SOURCE kind=outbox origin=prebuilt");
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("Rust workspace root")
            .to_path_buf();
        let mut command = Command::new(binary);
        command
            .current_dir(workspace_root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("RUST_LOG", "warn")
            .env("RUST_LOG_STYLE", "never")
            .env("OUTBOX_WORKER_BUILD_ID", format!("integration-outbox-{}", Uuid::new_v4()))
            .env("OUTBOX_WORKER_ID", format!("integration-outbox-worker-{}", Uuid::new_v4()));
        for (key, value) in envs {
            command.env(key, value);
        }
        let child = command.spawn().expect("spawn prebuilt outbox_worker");
        ChildGuard { child }
    }

    include!("tests/outbox_delivery_tests.rs");
    include!("tests/pg_migration_upgrade_tests.rs");
}
