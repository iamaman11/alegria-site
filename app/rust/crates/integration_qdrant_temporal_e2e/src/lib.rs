//! Native Temporal child -> PG admission -> Qdrant E2E, including retry History proof.

#[cfg(all(test, feature = "e2e"))]
#[path = "../../integration_pg_v3_e2e/src/containers.rs"]
mod temporal_containers;

#[cfg(all(test, feature = "e2e"))]
#[allow(unused)]
mod tests {
    use super::temporal_containers::TemporalHarness;
    use integration_data_fixtures::{PostgresHarness, QdrantHarness};
    use operation_protocol::{
        QdrantKeywordProjectionCommandV1, QdrantProjectionRunV1, QDRANT_DAG_PARENT_QUEUE_V1,
        QDRANT_DAG_PARENT_TYPE_V1,
    };
    use serial_test::serial;
    use std::{
        env, fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        process::{Child, Command},
        time::Duration,
    };
    use temporalio_client::{
        UntypedSignal, UntypedWorkflow, WorkflowDescribeOptions, WorkflowExecutionInfo,
        WorkflowFetchHistoryOptions, WorkflowGetResultOptions, WorkflowSignalOptions,
        WorkflowStartOptions,
    };
    use temporalio_common::{
        data_converters::{PayloadConverter, RawValue},
        protos::temporal::api::history::v1::history_event::Attributes,
    };
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    struct ChildGuard(Option<Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn binary(variable: &str) -> PathBuf {
        let value = PathBuf::from(
            env::var_os(variable).expect("CI must supply exact-source worker binary"),
        );
        assert!(value.is_file(), "native worker binary missing: {variable}");
        value
    }

    fn secret(prefix: &str, value: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        fs::write(&path, value).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    struct ScopedEnvironment {
        old: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl ScopedEnvironment {
        fn apply(changes: &[(&str, &str)]) -> Self {
            let mut old = Vec::with_capacity(changes.len());
            for &(name, value) in changes {
                old.push((name.to_owned(), std::env::var_os(name)));
                std::env::set_var(name, value);
            }
            Self { old }
        }
    }

    impl Drop for ScopedEnvironment {
        fn drop(&mut self) {
            for (name, previous) in self.old.drain(..).rev() {
                if let Some(value) = previous {
                    std::env::set_var(&name, value);
                } else {
                    std::env::remove_var(&name);
                }
            }
        }
    }

    include!("tests/native_temporal_qdrant_tests.rs");
}
