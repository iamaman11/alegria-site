    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        CmsPublishOutputPayload, DraftNormalizeOutputPayload, DraftQaOutputPayload, EditorialBrief,
        EditorialDraftGenerateInputPayload, LinkRecommendationState, LlmDraftRequest,
        ProjectionBarrierAuditOutputPayload, RebuildDetectOutputPayload, SectionTemplateBinding,
        SeoVerifiedFactSupportState,
    };
    use infrastructure::adapters::{
        dataforseo_serp_adapter::{DataForSeoConfig, DataForSeoSerpClient},
        editorial_llm_adapter,
        proto_runtime_payload_store::RuntimeProtoPayload,
    };
    use runtime_composition::adapters::seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository;
    #[cfg(feature = "e2e")]
    use infrastructure::adapters::{
        raw_crawl_adapter,
    };
    #[cfg(feature = "e2e")]
    use temporal_gateway::connect_client;
    #[cfg(feature = "e2e")]
    use temporalio_client::{
        UntypedWorkflow, WorkflowFetchHistoryOptions, WorkflowGetResultOptions, WorkflowStartOptions,
    };
    #[cfg(feature = "e2e")]
    use temporalio_common::data_converters::RawValue;
    #[cfg(feature = "e2e")]
    use temporalio_common::protos::temporal::api::{
        enums::v1::EventType,
        history::v1::{history_event::Attributes, History},
    };
    use seo_application::{
        execution::{run_mode_for_scenario, SeoRunPolicy},
        scenario::{
            execute_site_build_scenario, SeoExecutionMode, SeoScenarioKind, SeoScenarioRequest,
        },
    };
    use seo_ports::{SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest};
    use seo_steps::seo_step_support::artifact_key;
    use serial_test::serial;
    use sqlx::{types::Json, Row};
    use std::{
        collections::{BTreeMap, BTreeSet},
        env, fs,
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    use tokio::time::timeout;
    use uuid::Uuid;

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct SemanticPageNode {
        canonical_url_path: String,
        page_type_key: String,
        dominant_intent: String,
        lifecycle_state: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct SemanticPageDraft {
        canonical_url_path: String,
        qa_verdict: String,
        body_markdown: String,
        has_truth_snapshot_ref: bool,
        support_ref_count: i64,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    struct SemanticSnapshot {
        page_nodes: Vec<SemanticPageNode>,
        page_drafts: Vec<SemanticPageDraft>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationFixture {
        fixture_id: String,
        #[serde(rename = "description")]
        _description: String,
        run_mode: String,
        business_scope: TruthCertificationScope,
        source_inputs: TruthCertificationSourceInputs,
        model_stub_inputs: TruthCertificationModelStubInputs,
        expected_outcomes: TruthCertificationExpectedOutcomes,
        invariants: TruthCertificationInvariants,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationScope {
        truth_identity_tuple: String,
        publishing_scope_tuple: String,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationSourceInputs {
        query: String,
        pages: Vec<TruthCertificationSourcePage>,
        #[serde(default)]
        support_bundle_seed: Option<TruthCertificationSupportBundleSeed>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationSourcePage {
        route: String,
        domain: String,
        title: String,
        description: String,
        html_file: String,
        #[serde(default)]
        governance_override: Option<TruthCertificationSourceGovernanceOverride>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationSourceGovernanceOverride {
        #[serde(default)]
        source_type: Option<String>,
        #[serde(default)]
        authority_class: Option<String>,
        #[serde(default)]
        independence_group_key: Option<String>,
        #[serde(default)]
        trust_level: Option<i64>,
        #[serde(default)]
        freshness_ttl_days: Option<i32>,
        #[serde(default)]
        override_eligible: Option<bool>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationModelStubInputs {
        #[serde(default)]
        extraction_responses: Vec<serde_json::Value>,
        editorial_response: serde_json::Value,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationSupportBundleSeed {
        #[serde(default)]
        concepts: Vec<TruthCertificationConceptSeed>,
        #[serde(default)]
        preverified_rules: Vec<TruthCertificationPreverifiedRuleSeed>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationConceptSeed {
        concept_key: String,
        concept_type: String,
        label_ru: String,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationPreverifiedRuleSeed {
        rule_instance_id: String,
        rule_type_key: String,
        concept_key: String,
        role_type: String,
        params: serde_json::Value,
        source_key: String,
        source_url: String,
        evidence_quote: String,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationExpectedOutcomes {
        verified_rule_set: Vec<String>,
        needs_hitl_set: Vec<String>,
        rejected_set: Vec<String>,
        contradiction_groups: Vec<String>,
        completeness_failures: Vec<String>,
        draft_verdict: String,
        publish_verdict: String,
        projection_verdict: String,
        rebuild_impact_emitted: bool,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TruthCertificationInvariants {
        no_truth_promotion_from_retrieval_or_graph: bool,
        no_source_tier_shortcut_to_verified: bool,
        no_unsupported_claim_reaches_draft_as_truth: bool,
    }

    #[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
    struct TruthCertificationFixtureReport {
        fixture_id: String,
        run_id: String,
        context_key: String,
        scope_signature: String,
        verified_rule_set: Vec<String>,
        needs_hitl_set: Vec<String>,
        rejected_set: Vec<String>,
        contradiction_groups: Vec<String>,
        completeness_failures: Vec<String>,
        draft_verdict: String,
        draft_blocking_reasons: Vec<String>,
        publish_verdict: String,
        projection_verdict: String,
        changed_truth_keys: Vec<String>,
        rebuild_impact_emitted: bool,
        rebuild_impacted_page_count: usize,
        temporal_step_multiset: BTreeMap<String, usize>,
        semantic_page_node_count: usize,
        semantic_page_draft_count: usize,
        semantic_page_draft_qa_verdicts: Vec<String>,
        truth_adjudication_decisions: Vec<String>,
        source_governance_snapshot: Vec<String>,
        required_factual_blocks_without_support: Vec<String>,
        pass: bool,
        failure_reasons: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workflow_failure_diagnostics: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct RenderedTruthCertificationPage {
        fixture_id: String,
        route: String,
        url: String,
        page: TruthCertificationSourcePage,
    }

    #[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
    struct TruthCertificationSuiteReport {
        workflow_type: String,
        fixture_count: usize,
        fixtures: Vec<TruthCertificationFixtureReport>,
    }

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

    fn truth_certification_fixtures_dir() -> PathBuf {
        // Read the sole checked-in corpus from its canonical source package;
        // this helper is compiled by multiple real E2E test owners.
        workspace_root()
            .join("crates")
            .join("integration_harness")
            .join("fixtures")
            .join("visa_truth_certification")
    }

    fn target_dir() -> PathBuf {
        env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root().join("target"))
    }

    fn nested_worker_target_dir() -> PathBuf {
        target_dir()
            .join("integration-harness-nested")
            .join("combined-workers")
    }

    fn nested_worker_binaries() -> &'static (PathBuf, PathBuf) {
        static BINARIES: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();
        BINARIES.get_or_init(|| {
            // Compile both real binaries once; their processes and runtime state
            // remain isolated. Keep this target separate from the e2e test target.
            let nested_target = nested_worker_target_dir();
            let build_started = Instant::now();
            let status = Command::new("cargo")
                .args([
                    "build",
                    "--locked",
                    "--target-dir",
                    nested_target
                        .to_str()
                        .expect("nested integration worker target dir"),
                    "-p",
                    "temporal_worker",
                    "-p",
                    "outbox_worker",
                    "--bin",
                    "temporal_worker",
                    "--bin",
                    "outbox_worker",
                ])
                .current_dir(workspace_root())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .expect("build nested integration workers");
            eprintln!(
                "CI_NESTED_WORKERS_BUILD elapsed_ms={} exit_code={}",
                build_started.elapsed().as_millis(),
                status.code().unwrap_or(-1)
            );
            assert!(status.success(), "combined integration worker build failed");
            let debug_dir = nested_target.join("debug");
            (
                debug_dir.join(format!("temporal_worker{}", env::consts::EXE_SUFFIX)),
                debug_dir.join(format!("outbox_worker{}", env::consts::EXE_SUFFIX)),
            )
        })
    }

    fn build_temporal_worker_binary() -> PathBuf {
        if let Some(binary) = env::var_os("INTEGRATION_TEMPORAL_WORKER_BIN") {
            eprintln!("CI_WORKER_BINARY_SOURCE kind=temporal origin=prebuilt");
            PathBuf::from(binary)
        } else {
            eprintln!("CI_WORKER_BINARY_SOURCE kind=temporal origin=nested");
            nested_worker_binaries().0.clone()
        }
    }

    /// Exercise the *actual* registered workflow implementation against bytes
    /// fetched from the real Temporal server. This confirms deterministic
    /// offline replay on the same source SHA; cross-version replay still needs
    /// immutable histories captured before the code changed. The subprocess
    /// uses no DB, poller, network client, metrics, or Activity adapter.
    #[cfg(feature = "e2e")]
    fn assert_saved_temporal_history_replays(history_json: &[u8]) {
        assert!(
            history_json.len() > 100,
            "Temporal server returned no meaningful workflow history"
        );
        let history_path = env::temp_dir().join(format!(
            "alegria-canonical-history-{}-{}.json",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::write(&history_path, history_json).expect("write actual Temporal history for replay");
        let output = Command::new(build_temporal_worker_binary())
            .arg("--replay-history")
            .arg(&history_path)
            .current_dir(workspace_root())
            .output();
        fs::remove_file(&history_path).expect("remove disposable Temporal history fixture");
        let output = output.expect("run exact-source Temporal history replayer");
        assert!(
            output.status.success(),
            "V1 Temporal history failed actual SDK replay: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("CI_TEMPORAL_HISTORY_REPLAY=PASS"),
            "offline Temporal replay did not report completion"
        );
    }

    /// Read actual scheduled Activity commands, preserving Temporal event order.
    /// A missing or unknown history layout is rejected, never interpreted as
    /// an empty command sequence.
    #[cfg(feature = "e2e")]
    fn scheduled_temporal_activities(history_json: &[u8]) -> Vec<String> {
        // WorkflowHistory::to_json() serializes precisely this generated
        // protobuf `History`; `HistoryEvent.attributes` is a nested oneof,
        // not an `activityTaskScheduledEventAttributes` key on the event.
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

    /// Assert the actual command stream persisted by Temporal, not merely the
    /// typed recipe or final result. Both recipes schedule the same Activities,
    /// but a different topological order is an observable runtime contract.
    #[cfg(feature = "e2e")]
    fn assert_composable_activity_order(history_json: &[u8], support_first: bool) {
        let activities = scheduled_temporal_activities(history_json);
        let support_index = activities
            .iter()
            .position(|activity| activity == "AlegriaActivities::load_verified_support_bundle")
            .unwrap_or_else(|| panic!("missing scheduled support Activity in {activities:?}"));
        let source_index = activities
            .iter()
            .position(|activity| activity == "AlegriaActivities::run_serp_ingest_step")
            .unwrap_or_else(|| panic!("missing scheduled source Activity in {activities:?}"));
        assert_eq!(
            support_index < source_index,
            support_first,
            "V2 executed a different real Activity command order: {activities:?}"
        );
    }

    fn build_outbox_worker_binary() -> PathBuf {
        if let Some(binary) = env::var_os("INTEGRATION_OUTBOX_WORKER_BIN") {
            eprintln!("CI_WORKER_BINARY_SOURCE kind=outbox origin=prebuilt");
            PathBuf::from(binary)
        } else {
            eprintln!("CI_WORKER_BINARY_SOURCE kind=outbox origin=nested");
            nested_worker_binaries().1.clone()
        }
    }

    fn spawn_temporal_worker(envs: &BTreeMap<String, String>) -> ChildGuard {
        let binary = build_temporal_worker_binary();
        let mut command = Command::new(binary);
        command
            .current_dir(workspace_root())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("RUST_LOG", "warn")
            .env(
                "WORKER_BUILD_ID",
                format!("integration-parity-{}", Uuid::new_v4()),
            )
            .env("METRICS_PORT", "0");
        for (key, value) in envs {
            command.env(key, value);
        }
        let child = command.spawn().expect("spawn temporal_worker");
        ChildGuard { child }
    }

    fn spawn_outbox_worker(envs: &BTreeMap<String, String>) -> ChildGuard {
        let binary = build_outbox_worker_binary();
        let mut command = Command::new(binary);
        command
            .current_dir(workspace_root())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("RUST_LOG", "warn")
            .env("RUST_LOG_STYLE", "never")
            .env(
                "OUTBOX_WORKER_BUILD_ID",
                format!("integration-outbox-{}", Uuid::new_v4()),
            )
            .env(
                "OUTBOX_WORKER_ID",
                format!("integration-outbox-worker-{}", Uuid::new_v4()),
            );
        for (key, value) in envs {
            command.env(key, value);
        }
        let child = command.spawn().expect("spawn outbox_worker");
        ChildGuard { child }
    }

    fn load_truth_certification_fixtures() -> Vec<TruthCertificationFixture> {
        let mut paths = fs::read_dir(truth_certification_fixtures_dir())
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                serde_json::from_slice::<TruthCertificationFixture>(&fs::read(&path).unwrap())
                    .unwrap_or_else(|err| {
                        panic!("failed to decode fixture {}: {err}", path.display())
                    })
            })
            .collect()
    }

