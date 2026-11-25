//! Real, owner-isolated PostgreSQL V3 admission and cross-queue Temporal E2E.
//!
//! Production does not depend on this crate. The `e2e` feature compiles only
//! the genuine PG effect owner, its operator admission tests and the native
//! Temporal client, not the legacy runtime or external graph/vector providers.

#[cfg(all(test, feature = "e2e"))]
mod containers;

#[cfg(all(test, feature = "e2e"))]
#[allow(unused, warnings)]
mod tests {
    use super::containers::{PostgresHarness, TemporalHarness};
    use serial_test::serial;
    use std::{
        env, fs,
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::Duration,
    };
    use temporalio_client::{
        Client, ClientOptions, Connection, ConnectionOptions, UntypedWorkflow,
        WorkflowFetchHistoryOptions, WorkflowGetResultOptions, WorkflowStartOptions,
    };
    use temporalio_common::protos::temporal::api::{
        enums::v1::EventType,
        history::v1::{history_event::Attributes, History},
    };
    use temporalio_sdk_core::Url;
    use tokio::time::timeout;
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

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root")
            .to_path_buf()
    }

    // Use the exact native client transport of the production gateway. A
    // test-only connection does not need to compile SEO/runtime domain ports.
    async fn connect_client(
        temporal_url: &str,
        identity: String,
        namespace: &str,
    ) -> anyhow::Result<Client> {
        let opts = ConnectionOptions::new(Url::parse(temporal_url)?)
            .identity(identity)
            .build();
        let connection = Connection::connect(opts).await?;
        Client::new(connection, ClientOptions::new(namespace).build())
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Reject missing/unknown scheduled-event protobuf layouts rather than
    /// interpreting them as an empty Activity sequence.
    fn scheduled_temporal_activities(history_json: &[u8]) -> Vec<String> {
        let history: History =
            serde_json::from_slice(history_json).expect("decode SDK-generated Temporal History");
        assert!(
            !history.events.is_empty(),
            "actual Temporal History must contain events"
        );
        history
            .events
            .iter()
            .filter_map(|event| {
                if event.event_type != EventType::ActivityTaskScheduled as i32 {
                    return None;
                }
                let Some(Attributes::ActivityTaskScheduledEventAttributes(attributes)) =
                    event.attributes.as_ref()
                else {
                    panic!(
                        "scheduled Temporal Activity event {} lacks matching attributes",
                        event.event_id
                    );
                };
                Some(
                    attributes
                        .activity_type
                        .as_ref()
                        .expect("scheduled Temporal Activity missing activity_type")
                        .name
                        .clone(),
                )
            })
            .collect()
    }

    include!("tests/pg_v3_truth_review_effect_tests.rs");
    include!("tests/pg_v3_admission_race_tests.rs");
    include!("tests/pg_v3_operator_cli_tests.rs");
    include!("tests/pg_v5_ordered_children_tests.rs");
    include!("tests/pg_v3_temporal_cross_queue_tests.rs");
    include!("tests/pg_v6_tenant_ingress_tests.rs");
}
