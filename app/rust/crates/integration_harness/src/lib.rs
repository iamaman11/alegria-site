use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(feature = "e2e")]
pub mod containers;
pub mod env_overrides;
pub mod stub_servers;

#[derive(Debug)]
pub struct IntegrationCtx {
    pub dataforseo: stub_servers::StubServerHandle,
    pub openai: stub_servers::StubServerHandle,
}

impl IntegrationCtx {
    pub async fn start(dataforseo_payload: Value, openai_payload: Value) -> Result<Self> {
        let dataforseo = stub_servers::spawn_json_stub(
            "/v3/serp/google/organic/live/advanced",
            dataforseo_payload,
        )
        .await
        .context("start dataforseo stub failed")?;
        let openai = stub_servers::spawn_json_stub("/v1/chat/completions", openai_payload)
            .await
            .context("start openai stub failed")?;
        Ok(Self { dataforseo, openai })
    }

    pub fn dataforseo_env(&self) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "DATAFORSEO_ENDPOINT".to_string(),
                format!(
                    "{}/v3/serp/google/organic/live/advanced",
                    self.dataforseo.base_url
                ),
            ),
            ("DATAFORSEO_LOGIN".to_string(), "stub-login".to_string()),
            (
                "DATAFORSEO_PASSWORD".to_string(),
                "stub-password".to_string(),
            ),
            ("DATAFORSEO_LANGUAGE_CODE".to_string(), "en".to_string()),
            ("DATAFORSEO_LOCATION_CODE".to_string(), "2840".to_string()),
            ("DATAFORSEO_DEPTH".to_string(), "10".to_string()),
        ])
    }

    pub fn local_editorial_env(&self) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "SEO_LLM_PROVIDER".to_string(),
                "local_compatible".to_string(),
            ),
            (
                "SEO_LLM_LOCAL_ENDPOINT".to_string(),
                format!("{}/v1/chat/completions", self.openai.base_url),
            ),
            (
                "SEO_LLM_LOCAL_MODEL".to_string(),
                "stub-editorial-model".to_string(),
            ),
        ])
    }

    pub fn provider_env(&self) -> BTreeMap<String, String> {
        let mut vars = self.dataforseo_env();
        vars.extend(self.local_editorial_env());
        vars
    }

    pub fn apply_provider_env(&self) -> env_overrides::EnvOverrideGuard {
        env_overrides::EnvOverrideGuard::apply(self.provider_env())
    }
}

#[cfg(feature = "e2e")]
pub use containers::{InfraHarness, Neo4jHarness, PostgresHarness, QdrantHarness, TemporalHarness};

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        CmsPublishOutputPayload, DraftNormalizeOutputPayload, DraftQaOutputPayload, EditorialBrief,
        EditorialDraftGenerateInputPayload, LinkRecommendationState, LlmDraftRequest,
        ProjectionBarrierAuditOutputPayload, RebuildDetectOutputPayload, SectionTemplateBinding,
        SeoVerifiedFactSupportState,
    };
    use infrastructure::adapters::{
        dataforseo_serp_adapter::{DataForSeoConfig, DataForSeoSerpClient},
        editorial_llm_adapter, raw_crawl_adapter,
        proto_runtime_payload_store::RuntimeProtoPayload,
        seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
        temporalio_sdk_adapter::{
            connect_client, RawValue, UntypedWorkflow, WorkflowGetResultOptions,
            WorkflowStartOptions,
        },
    };
    use seo_steps::seo_step_support::artifact_key;
    use seo_application::{
        execution::{run_mode_for_scenario, SeoRunPolicy},
        registration::register_site_build_input,
        scenario::{
            execute_site_build_scenario, SeoExecutionMode, SeoScenarioKind, SeoScenarioRequest,
        },
    };
    use seo_ports::SeoSiteBuildRegistrationRequest;
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
    use std::sync::OnceLock;

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
        required_factual_blocks_without_support: Vec<String>,
        pass: bool,
        failure_reasons: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workflow_failure_diagnostics: Option<String>,
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("visa_truth_certification")
    }

    fn target_dir() -> PathBuf {
        env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root().join("target"))
    }

    fn build_temporal_worker_binary() -> PathBuf {
        static BIN: OnceLock<PathBuf> = OnceLock::new();
        BIN.get_or_init(|| {
            let binary = target_dir()
                .join("debug")
                .join(format!("temporal_worker{}", env::consts::EXE_SUFFIX));
            if binary.exists() {
                return binary;
            }
            let workspace = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "temporal_worker", "--bin", "temporal_worker"])
                .current_dir(&workspace)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .expect("build temporal_worker");
            assert!(status.success(), "cargo build -p temporal_worker failed");
            binary
        })
        .clone()
    }

    fn build_outbox_worker_binary() -> PathBuf {
        static BIN: OnceLock<PathBuf> = OnceLock::new();
        BIN.get_or_init(|| {
            let binary = target_dir()
                .join("debug")
                .join(format!("outbox_worker{}", env::consts::EXE_SUFFIX));
            if binary.exists() {
                return binary;
            }
            let workspace = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "outbox_worker", "--bin", "outbox_worker"])
                .current_dir(&workspace)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .expect("build outbox_worker");
            assert!(status.success(), "cargo build -p outbox_worker failed");
            binary
        })
        .clone()
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
        paths.into_iter()
            .map(|path| {
                serde_json::from_slice::<TruthCertificationFixture>(&fs::read(&path).unwrap())
                    .unwrap_or_else(|err| panic!("failed to decode fixture {}: {err}", path.display()))
            })
            .collect()
    }

    async fn render_fixture_source_pages(
        fixtures: &[TruthCertificationFixture],
    ) -> (
        Vec<stub_servers::StubServerHandle>,
        Vec<(String, serde_json::Value)>,
    ) {
        let mut source_servers = Vec::new();
        let mut serp_sequence = Vec::new();
        for fixture in fixtures {
            let mut items = Vec::new();
            for (idx, page) in fixture.source_inputs.pages.iter().enumerate() {
                let body = fs::read_to_string(truth_certification_fixtures_dir().join(&page.html_file))
                    .unwrap_or_else(|err| {
                        panic!(
                            "failed to read source HTML {} for fixture {}: {err}",
                            page.html_file, fixture.fixture_id
                        )
                    });
                let server =
                    stub_servers::spawn_text_stub(&page.route, body, "text/html; charset=utf-8")
                        .await
                        .unwrap();
                let url = format!("{}{}", server.base_url, page.route);
                source_servers.push(server);
                items.push(serde_json::json!({
                    "type": "organic",
                    "rank_group": idx + 1,
                    "rank_absolute": idx + 1,
                    "title": page.title,
                    "url": url,
                    "domain": page.domain,
                    "description": page.description
                }));
            }
            serp_sequence.push((
                fixture.fixture_id.clone(),
                serde_json::json!({
                    "tasks": [{
                        "result": [{
                            "items": items
                        }]
                    }]
                }),
            ));
        }
        (source_servers, serp_sequence)
    }

    fn truth_certification_editorial_sequence(
        fixtures: &[TruthCertificationFixture],
    ) -> Vec<serde_json::Value> {
        fixtures
            .iter()
            .map(|fixture| {
                serde_json::json!({
                    "id": format!("chatcmpl-cert-editorial-{}", fixture.fixture_id),
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": fixture.model_stub_inputs.editorial_response.to_string()
                        }
                    }]
                })
            })
            .collect()
    }

    fn truth_certification_truth_sequence(
        fixtures: &[TruthCertificationFixture],
    ) -> Vec<serde_json::Value> {
        fixtures
            .iter()
            .flat_map(|fixture| {
                let responses = if fixture.model_stub_inputs.extraction_responses.is_empty() {
                    vec![serde_json::json!({ "candidates": [] }); fixture.source_inputs.pages.len()]
                } else {
                    assert_eq!(
                        fixture.model_stub_inputs.extraction_responses.len(),
                        fixture.source_inputs.pages.len(),
                        "fixture {} must provide one extraction response per source page",
                        fixture.fixture_id
                    );
                    fixture.model_stub_inputs.extraction_responses.clone()
                };
                responses
                    .into_iter()
                    .map(|payload| {
                        if payload.get("choices").is_some() {
                            payload
                        } else {
                            local_truth_response(payload)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn truth_certification_env(
        serp_endpoint: &str,
        truth_endpoint: &str,
        editorial_endpoint: &str,
    ) -> BTreeMap<String, String> {
        let mut envs = env_overrides::btree_env([
            ("DATAFORSEO_ENDPOINT".to_string(), serp_endpoint.to_string()),
            ("DATAFORSEO_LOGIN".to_string(), "stub-login".to_string()),
            ("DATAFORSEO_PASSWORD".to_string(), "stub-password".to_string()),
            ("DATAFORSEO_LANGUAGE_CODE".to_string(), "en".to_string()),
            ("DATAFORSEO_LOCATION_CODE".to_string(), "2840".to_string()),
            ("DATAFORSEO_DEPTH".to_string(), "10".to_string()),
            ("SEO_LLM_PROVIDER".to_string(), "local_compatible".to_string()),
            (
                "SEO_LLM_LOCAL_ENDPOINT".to_string(),
                editorial_endpoint.to_string(),
            ),
            (
                "SEO_LLM_LOCAL_MODEL".to_string(),
                "stub-editorial-model".to_string(),
            ),
        ]);
        envs.extend(local_truth_env(truth_endpoint));
        envs.insert(
            "QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(),
            "true".to_string(),
        );
        envs
    }

    async fn reset_truth_certification_state(
        pool: &sqlx::PgPool,
        context_key: &str,
    ) {
        let site_tables = sqlx::query_scalar::<_, String>(
            r#"
            SELECT tablename
            FROM pg_tables
            WHERE schemaname = 'site'
              AND tablename NOT IN ('section_templates', 'page_blueprints')
            ORDER BY tablename
            "#,
        )
        .fetch_all(pool)
        .await
        .unwrap();
        if !site_tables.is_empty() {
            let statement = format!(
                "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
                site_tables
                    .iter()
                    .map(|table| format!("site.{table}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            sqlx::query(&statement).execute(pool).await.unwrap();
        }

        sqlx::query("TRUNCATE TABLE monitoring.seo_rebuild_backlog RESTART IDENTITY")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM verified.rule_instances WHERE context_key = $1")
            .bind(context_key)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM extracted.rule_candidates WHERE context_key = $1")
            .bind(context_key)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM raw.section_context_candidates WHERE context_key = $1")
            .bind(context_key)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_truth_certification_support_bundle(
        pool: &sqlx::PgPool,
        context_key: &str,
        fixture: &TruthCertificationFixture,
    ) {
        let Some(seed) = fixture.source_inputs.support_bundle_seed.as_ref() else {
            return;
        };

        for concept in &seed.concepts {
            seed_truth_context_and_concept(
                pool,
                context_key,
                &concept.concept_key,
                &concept.concept_type,
                &concept.label_ru,
            )
            .await;
        }

        for rule in &seed.preverified_rules {
            seed_admissible_verified_rule(
                pool,
                context_key,
                &rule.rule_instance_id,
                &rule.rule_type_key,
                &rule.concept_key,
                &rule.role_type,
                rule.params.clone(),
                &rule.source_key,
                &rule.source_url,
                &rule.evidence_quote,
            )
            .await;
        }
    }

    async fn register_truth_certification_input(
        repo: &SqlxSeoRuntimeRepository<'_>,
        run_id: String,
        query_batch_key: String,
        query: String,
        run_mode: String,
    ) -> contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload {
        register_site_build_input(
            repo,
            &SeoSiteBuildRegistrationRequest {
                run_id,
                context_key: None,
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec![query],
                query_batch_key: Some(query_batch_key),
                run_mode: Some(run_mode),
            },
        )
        .await
        .unwrap()
    }

    async fn latest_output_payload_bytes(
        pool: &sqlx::PgPool,
        run_id: &str,
        step_name: &str,
    ) -> Vec<Vec<u8>> {
        sqlx::query(
            r#"
            SELECT payload_bytes
            FROM pipeline.step_payload_blobs
            WHERE run_id = $1
              AND step_name = $2
              AND payload_kind = 'output'
            ORDER BY created_at ASC
            "#,
        )
        .bind(Uuid::parse_str(run_id).unwrap())
        .bind(step_name)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<Vec<u8>, _>("payload_bytes"))
        .collect()
    }

    async fn latest_output_payload_json(
        pool: &sqlx::PgPool,
        run_id: &str,
        step_name: &str,
    ) -> Vec<serde_json::Value> {
        latest_output_payload_bytes(pool, run_id, step_name)
            .await
            .into_iter()
            .map(|payload| serde_json::from_slice::<serde_json::Value>(&payload).unwrap())
            .collect()
    }

    async fn latest_proto_outputs<T: RuntimeProtoPayload>(
        pool: &sqlx::PgPool,
        run_id: &str,
        step_name: &str,
    ) -> Vec<T> {
        latest_output_payload_bytes(pool, run_id, step_name)
            .await
            .into_iter()
            .map(|payload| T::decode_payload_bytes(&payload).unwrap())
            .collect()
    }

    fn stable_rule_key(rule_type_key: &str, concept_key: &str, params: &serde_json::Value) -> String {
        format!(
            "{}|{}|{}",
            rule_type_key,
            concept_key,
            serde_json::to_string(params).unwrap()
        )
    }

    fn stable_decision_key(
        concept_key: &str,
        params: &serde_json::Value,
        reason: &str,
    ) -> String {
        format!(
            "{}|{}|{}",
            concept_key,
            serde_json::to_string(params).unwrap(),
            reason
        )
    }

    async fn insert_preapproved_decisions_when_ready(
        pool: sqlx::PgPool,
        scope_signature: String,
        deadline: Instant,
    ) {
        while Instant::now() < deadline {
            let rows = sqlx::query(
                r#"
                SELECT d.page_draft_key, d.draft_revision, n.page_node_key
                FROM site.page_drafts d
                JOIN site.page_nodes n ON n.page_node_key = d.page_node_key
                WHERE n.scope_signature = $1
                "#,
            )
            .bind(&scope_signature)
            .fetch_all(&pool)
            .await
            .unwrap();
            if !rows.is_empty() {
                for row in rows {
                    let page_node_key: String = row.get("page_node_key");
                    let page_draft_key: String = row.get("page_draft_key");
                    let draft_revision: i32 = row.get("draft_revision");
                    let revision_id = artifact_key(
                        "cms_revision",
                        &[
                            &page_node_key,
                            &page_draft_key,
                            &draft_revision.to_string(),
                            "seo_cms_publish@1",
                        ],
                    );
                    let decision_key =
                        artifact_key("cms_approval", &[&page_node_key, &revision_id, "approved"]);
                    sqlx::query(
                        r#"
                        INSERT INTO site.cms_approval_decisions
                            (decision_key, page_node_key, revision_id, actor_role, decision, reason,
                             decided_at, decision_payload)
                        VALUES ($1, $2, $3, 'seo_reviewer', 'approved', 'truth_certification_preapproval',
                                now(), '{"actor_role":"seo_reviewer","reason":"truth_certification_preapproval"}'::jsonb)
                        ON CONFLICT (decision_key) DO NOTHING
                        "#,
                    )
                    .bind(decision_key)
                    .bind(page_node_key)
                    .bind(revision_id)
                    .execute(&pool)
                    .await
                    .unwrap();
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn collect_truth_certification_fixture_report(
        pool: &sqlx::PgPool,
        fixture: &TruthCertificationFixture,
        run_id: &str,
        context_key: &str,
        scope_signature: &str,
    ) -> TruthCertificationFixtureReport {
        let verified_rows = sqlx::query(
            r#"
            SELECT rule_type_key, concept_key, params
            FROM verified.rule_instances
            WHERE context_key = $1
              AND status = 'verified'
            ORDER BY rule_type_key, concept_key
            "#,
        )
        .bind(context_key)
        .fetch_all(pool)
        .await
        .unwrap();
        let verified_rule_set = verified_rows
            .into_iter()
            .map(|row| {
                stable_rule_key(
                    &row.get::<String, _>("rule_type_key"),
                    &row.get::<String, _>("concept_key"),
                    &row.get::<serde_json::Value, _>("params"),
                )
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let truth_adjudication = latest_output_payload_json(pool, run_id, "truth_adjudication").await;
        let mut needs_hitl_set = BTreeSet::new();
        let mut rejected_set = BTreeSet::new();
        for payload in &truth_adjudication {
            for decision in payload
                .get("decisions")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
            {
                let concept = decision
                    .get("concept_canonical_key")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let params = decision.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));
                let reason = decision
                    .get("adjudication_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                match decision.get("decision").and_then(|value| value.as_str()) {
                    Some("needs_hitl") => {
                        needs_hitl_set.insert(stable_decision_key(concept, &params, reason));
                    }
                    Some("rejected") => {
                        rejected_set.insert(stable_decision_key(concept, &params, reason));
                    }
                    _ => {}
                }
            }
        }

        let contradiction = latest_output_payload_json(pool, run_id, "contradiction_gate").await;
        let contradiction_groups = contradiction
            .iter()
            .flat_map(|payload| {
                payload
                    .get("sections")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .flat_map(|section| {
                        section
                            .get("output")
                            .and_then(|value| value.get("conflicts"))
                            .and_then(|value| value.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|conflict| {
                                Some(format!(
                                    "{}|{}|{}|{}|{}",
                                    conflict.get("subject_key")?.as_str()?,
                                    conflict.get("predicate_key")?.as_str()?,
                                    conflict.get("value_left")?.as_str()?,
                                    conflict.get("value_right")?.as_str()?,
                                    conflict.get("severity")?.as_str()?
                                ))
                            })
                    })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let completeness = latest_output_payload_json(pool, run_id, "completeness_judge").await;
        let completeness_failures = completeness
            .iter()
            .flat_map(|payload| {
                payload
                    .get("sections")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .flat_map(|section| {
                        let blocked = section
                            .get("blocked_by_gate")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false);
                        if blocked {
                            Vec::new()
                        } else {
                            section
                                .get("output")
                                .and_then(|value| value.get("missing_elements"))
                                .and_then(|value| value.as_array())
                                .into_iter()
                                .flatten()
                                .filter_map(|missing| {
                                    Some(format!(
                                        "{}|{}|{}",
                                        missing.get("loss_type")?.as_str()?,
                                        missing.get("raw_fragment")?.as_str()?,
                                        missing.get("action")?.as_str()?
                                    ))
                                })
                                .collect::<Vec<_>>()
                        }
                    })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let draft_qas = latest_proto_outputs::<DraftQaOutputPayload>(pool, run_id, "draft_qa").await;
        let draft_blocking_reasons = draft_qas
            .iter()
            .flat_map(|output| output.blocking_reasons.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let draft_verdict = if draft_qas.is_empty() {
            "blocked".to_string()
        } else if draft_qas.iter().any(|output| output.verdict == "publish_ready") {
            "allow".to_string()
        } else {
            "blocked".to_string()
        };

        let cms_approved =
            latest_proto_outputs::<CmsPublishOutputPayload>(pool, run_id, "cms_publish_approved")
                .await;
        let cms_review =
            latest_proto_outputs::<CmsPublishOutputPayload>(pool, run_id, "cms_request_review")
                .await;
        let publish_verdict = if cms_approved.iter().any(|output| output.verdict == "approved") {
            "allow".to_string()
        } else if !cms_review.is_empty() || fixture.run_mode.contains("publish") {
            "blocked".to_string()
        } else {
            "not_attempted".to_string()
        };

        let projection_barriers =
            latest_proto_outputs::<ProjectionBarrierAuditOutputPayload>(
                pool,
                run_id,
                "projection_barrier(semantic_projection)",
            )
            .await;
        let projection_verdict = if projection_barriers
            .last()
            .map(|output| output.status.as_str() == "clear")
            .unwrap_or(false)
        {
            "allow".to_string()
        } else {
            "blocked".to_string()
        };

        let verified_truth_write =
            latest_output_payload_json(pool, run_id, "verified_truth_write").await;
        let changed_truth_keys = verified_truth_write
            .iter()
            .flat_map(|payload| {
                payload
                    .get("changed_truth_keys")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_string))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let rebuild_outputs =
            latest_proto_outputs::<RebuildDetectOutputPayload>(pool, run_id, "rebuild_detect")
                .await;
        let rebuild_impacted_page_count = rebuild_outputs
            .iter()
            .map(|output| output.impacts.len())
            .sum::<usize>();
        let rebuild_impact_emitted = rebuild_outputs
            .iter()
            .any(|output| !output.impacted_page_node_keys.is_empty());
        let draft_normalize_outputs =
            latest_proto_outputs::<DraftNormalizeOutputPayload>(pool, run_id, "draft_normalize")
                .await;
        let required_factual_blocks_without_support = draft_normalize_outputs
            .iter()
            .flat_map(|output| {
                output
                    .draft
                    .as_ref()
                    .into_iter()
                    .flat_map(|draft| {
                        draft.content_blocks.iter().filter_map(|block| {
                            let factual = !matches!(
                                block.section_role.as_str(),
                                "related_pages" | "cta_disclaimer"
                            );
                            if block.required && factual && block.support_refs.is_empty() {
                                Some(block.section_role.clone())
                            } else {
                                None
                            }
                        })
                    })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let temporal_step_multiset = temporal_step_multiset(pool, run_id).await;
        let semantic_snapshot = capture_semantic_snapshot(pool, scope_signature).await;
        let semantic_page_draft_qa_verdicts = semantic_snapshot
            .page_drafts
            .iter()
            .map(|draft| draft.qa_verdict.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let mut failure_reasons = Vec::new();
        if verified_rule_set != fixture.expected_outcomes.verified_rule_set {
            failure_reasons.push("verified_rule_set_mismatch".to_string());
        }
        if needs_hitl_set.iter().cloned().collect::<Vec<_>>() != fixture.expected_outcomes.needs_hitl_set {
            failure_reasons.push("needs_hitl_set_mismatch".to_string());
        }
        if rejected_set.iter().cloned().collect::<Vec<_>>() != fixture.expected_outcomes.rejected_set {
            failure_reasons.push("rejected_set_mismatch".to_string());
        }
        if contradiction_groups != fixture.expected_outcomes.contradiction_groups {
            failure_reasons.push("contradiction_groups_mismatch".to_string());
        }
        if completeness_failures != fixture.expected_outcomes.completeness_failures {
            failure_reasons.push("completeness_failures_mismatch".to_string());
        }
        if draft_verdict != fixture.expected_outcomes.draft_verdict {
            failure_reasons.push("draft_verdict_mismatch".to_string());
        }
        if publish_verdict != fixture.expected_outcomes.publish_verdict {
            failure_reasons.push("publish_verdict_mismatch".to_string());
        }
        if projection_verdict != fixture.expected_outcomes.projection_verdict {
            failure_reasons.push("projection_verdict_mismatch".to_string());
        }
        if rebuild_impact_emitted != fixture.expected_outcomes.rebuild_impact_emitted {
            failure_reasons.push("rebuild_impact_mismatch".to_string());
        }

        TruthCertificationFixtureReport {
            fixture_id: fixture.fixture_id.clone(),
            run_id: run_id.to_string(),
            context_key: context_key.to_string(),
            scope_signature: scope_signature.to_string(),
            verified_rule_set,
            needs_hitl_set: needs_hitl_set.into_iter().collect(),
            rejected_set: rejected_set.into_iter().collect(),
            contradiction_groups,
            completeness_failures,
            draft_verdict,
            draft_blocking_reasons,
            publish_verdict,
            projection_verdict,
            changed_truth_keys,
            rebuild_impact_emitted,
            rebuild_impacted_page_count,
            temporal_step_multiset,
            semantic_page_node_count: semantic_snapshot.page_nodes.len(),
            semantic_page_draft_count: semantic_snapshot.page_drafts.len(),
            semantic_page_draft_qa_verdicts,
            required_factual_blocks_without_support,
            pass: failure_reasons.is_empty(),
            failure_reasons,
            workflow_failure_diagnostics: None,
        }
    }

    async fn certification_failure_diagnostics(pool: &sqlx::PgPool, run_id: &str) -> String {
        let rows = sqlx::query(
            r#"
            SELECT step_name, status, COALESCE(error_class, '') AS error_class, COALESCE(error_message, '') AS error_message
            FROM pipeline.step_executions
            WHERE run_id = $1
            ORDER BY updated_at DESC, step_execution_id DESC
            LIMIT 12
            "#,
        )
        .bind(Uuid::parse_str(run_id).unwrap())
        .fetch_all(pool)
        .await
        .unwrap();
        let summary = rows
            .into_iter()
            .map(|row| {
                format!(
                    "{}:{}:{}:{}",
                    row.get::<String, _>("step_name"),
                    row.get::<String, _>("status"),
                    row.get::<String, _>("error_class"),
                    row.get::<String, _>("error_message")
                )
            })
            .collect::<Vec<_>>();
        summary.join("\n")
    }

    fn expected_workflow_failure_allowed(
        fixture: &TruthCertificationFixture,
        report: &TruthCertificationFixtureReport,
        diagnostics: &str,
    ) -> bool {
        fixture.expected_outcomes.draft_verdict == "blocked"
            && report.draft_verdict == "blocked"
            && report.publish_verdict == fixture.expected_outcomes.publish_verdict
            && diagnostics.contains("truth_admissibility_gate:failed:")
    }

    async fn register_synthetic_site_input(
        repo: &SqlxSeoRuntimeRepository<'_>,
        run_id: String,
        query_batch_key: String,
    ) -> contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload {
        register_site_build_input(
            repo,
            &SeoSiteBuildRegistrationRequest {
                run_id,
                context_key: None,
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec!["spain tourist visa belarus".to_string()],
                query_batch_key: Some(query_batch_key),
                run_mode: Some(
                    run_mode_for_scenario(SeoScenarioKind::Full, false, false, true).to_string(),
                ),
            },
        )
        .await
        .unwrap()
    }

    async fn seed_synthetic_truth(pool: &sqlx::PgPool, context_key: &str, source_base_url: &str) {
        sqlx::query(
            r#"
            INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
            VALUES ('document.passport', 'document', 'Passport', 'active')
            ON CONFLICT (concept_key) DO UPDATE
            SET label_ru = EXCLUDED.label_ru,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO kb.sources (source_key, source_type, source_label, base_url, trust_level, status)
            VALUES ('source.official.synthetic', 'government', 'Synthetic official source', $1, 5, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(format!("{source_base_url}/official"))
        .execute(pool)
        .await
        .unwrap();
        seed_admissible_verified_rule(
            pool,
            context_key,
            "rule.synthetic.passport",
            "document_required",
            "document.passport",
            "document_required",
            serde_json::json!({
                "subtype": "travel_document",
            }),
            "source.official.synthetic",
            &format!("{source_base_url}/official/passport"),
            "Passport is required for the visa application.",
        )
        .await;
    }

    async fn seed_admissible_verified_rule(
        pool: &sqlx::PgPool,
        context_key: &str,
        rule_instance_id: &str,
        rule_type_key: &str,
        concept_key: &str,
        role_type: &str,
        params: serde_json::Value,
        source_key: &str,
        source_url: &str,
        fragment_text: &str,
    ) {
        let source_base_url = source_url
            .split('/')
            .take(3)
            .collect::<Vec<_>>()
            .join("/");
        sqlx::query(
            r#"
            INSERT INTO kb.sources (source_key, source_type, source_label, base_url, trust_level, status)
            VALUES ($1, 'government', 'Synthetic support source', $2, 5, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(source_key)
        .bind(source_base_url)
        .execute(pool)
        .await
        .unwrap();
        let raw_html = format!("<html><body><main><p>{fragment_text}</p></main></body></html>");
        let snapshot_hash = format!("seed:{}:{}", rule_instance_id, raw_html.len());
        let page_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.pages
                (url, domain, dtype, status_code, final_url, title, raw_html, raw_html_bytes, content_hash, content, source_observation, processed)
            VALUES
                ($1, $2, 'government', 200, $1, 'Synthetic support source', $3, $4, $5, $6, $7, true)
            RETURNING id
            "#,
        )
        .bind(source_url)
        .bind(
            source_url
                .split('/')
                .nth(2)
                .unwrap_or("example.com")
                .to_string(),
        )
        .bind(&raw_html)
        .bind(raw_html.len() as i32)
        .bind(&snapshot_hash)
        .bind(Json(serde_json::json!({
            "markdown": fragment_text,
            "headings": [],
            "links": [],
        })))
        .bind(Json(serde_json::json!({
            "seeded_by": "integration_harness",
            "source_key": source_key,
        })))
        .fetch_one(pool)
        .await
        .unwrap();
        let section_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO raw.sections
                (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
            VALUES
                ($1, 'Requirements', 1, 0, 'paragraph', $2, $3)
            RETURNING id
            "#,
        )
        .bind(page_id)
        .bind(fragment_text)
        .bind(format!("seed:{}:section:{}", rule_instance_id, fragment_text.len()))
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO verified.rule_instances
                (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status,
                 source_key, confidence, evidence_section_id, evidence_quote, span_start, span_end,
                 source_snapshot_hash, verification_method, adjudication_reason, publish_admissibility,
                 freshness_class, completeness_class, registry_version, prompt_version, model_version,
                 pipeline_version, effective_from)
            VALUES
                ($1, $2, $3, $4, $5, $6, 'verified',
                 $7, 0.99, $8, $9, 0, $10,
                 $11, 'synthetic_seed', 'integration_harness_seed', 'admissible',
                 'fresh', 'complete', 'registry@1', 'seed@1', 'none', 'integration_harness@1', current_date)
            ON CONFLICT (rule_instance_id) DO UPDATE
            SET params = EXCLUDED.params,
                status = EXCLUDED.status,
                source_key = EXCLUDED.source_key,
                confidence = EXCLUDED.confidence,
                evidence_section_id = EXCLUDED.evidence_section_id,
                evidence_quote = EXCLUDED.evidence_quote,
                span_start = EXCLUDED.span_start,
                span_end = EXCLUDED.span_end,
                source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                verification_method = EXCLUDED.verification_method,
                adjudication_reason = EXCLUDED.adjudication_reason,
                publish_admissibility = EXCLUDED.publish_admissibility,
                freshness_class = EXCLUDED.freshness_class,
                completeness_class = EXCLUDED.completeness_class,
                registry_version = EXCLUDED.registry_version,
                prompt_version = EXCLUDED.prompt_version,
                model_version = EXCLUDED.model_version,
                pipeline_version = EXCLUDED.pipeline_version,
                updated_at = now()
            "#,
        )
        .bind(rule_instance_id)
        .bind(context_key)
        .bind(rule_type_key)
        .bind(concept_key)
        .bind(role_type)
        .bind(Json(params))
        .bind(source_key)
        .bind(section_id)
        .bind(fragment_text)
        .bind(fragment_text.len() as i32)
        .bind(snapshot_hash)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_truth_context_and_concept(
        pool: &sqlx::PgPool,
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

    async fn seed_raw_page_with_section(
        pool: &sqlx::PgPool,
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

    fn local_truth_response(payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-truth-test",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": payload.to_string()
                }
            }]
        })
    }

    fn local_truth_candidate_payload(
        raw_text: &str,
        role: &str,
        concept_canonical_key: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "candidates": [{
                "role": role,
                "concept_canonical_key": concept_canonical_key,
                "raw_mention": raw_text,
                "params": params,
                "scope": {},
                "severity": "mandatory",
                "applies_to_profiles": [],
                "exceptions_raw": "",
                "conditions_raw": "",
                "alternatives": [],
                "modality_raw": "",
                "derivation_type": "direct",
                "is_numeric": true,
                "is_range": false,
                "is_incomplete": false,
                "confidence": 0.94,
                "evidence_section_id": 0,
                "evidence_quote": raw_text,
                "span_start": 0,
                "span_end": raw_text.len(),
                "uncertainty_flags": []
            }]
        })
    }

    fn local_truth_env(endpoint: &str) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "SEO_TRUTH_LLM_LOCAL_ENDPOINT".to_string(),
                endpoint.to_string(),
            ),
            (
                "SEO_TRUTH_LLM_LOCAL_MODEL".to_string(),
                "stub-truth-extraction-model".to_string(),
            ),
        ])
    }

    async fn capture_semantic_snapshot(
        pool: &sqlx::PgPool,
        scope_signature: &str,
    ) -> SemanticSnapshot {
        let page_nodes = sqlx::query(
            r#"
            SELECT canonical_url_path, page_type_key, dominant_intent, lifecycle_state
            FROM site.page_nodes
            WHERE scope_signature = $1
            ORDER BY canonical_url_path
            "#,
        )
        .bind(scope_signature)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| SemanticPageNode {
            canonical_url_path: row.get("canonical_url_path"),
            page_type_key: row.get("page_type_key"),
            dominant_intent: row.get("dominant_intent"),
            lifecycle_state: row.get("lifecycle_state"),
        })
        .collect::<Vec<_>>();

        let page_drafts = sqlx::query(
            r#"
            SELECT
                n.canonical_url_path,
                d.qa_verdict,
                d.body_markdown,
                d.truth_snapshot_ref,
                COALESCE(count(b.page_support_binding_key), 0)::bigint AS support_ref_count
            FROM site.page_drafts d
            JOIN site.page_nodes n ON n.page_node_key = d.page_node_key
            LEFT JOIN site.page_support_bindings b ON b.page_draft_key = d.page_draft_key
            WHERE n.scope_signature = $1
            GROUP BY n.canonical_url_path, d.qa_verdict, d.body_markdown, d.truth_snapshot_ref, d.updated_at
            ORDER BY n.canonical_url_path, d.updated_at
            "#,
        )
        .bind(scope_signature)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
            .map(|row| SemanticPageDraft {
                canonical_url_path: row.get("canonical_url_path"),
                qa_verdict: row.get("qa_verdict"),
                body_markdown: row.get("body_markdown"),
                has_truth_snapshot_ref: !row
                    .get::<String, _>("truth_snapshot_ref")
                    .trim()
                    .is_empty(),
                support_ref_count: row.get("support_ref_count"),
            })
        .collect::<Vec<_>>();

        SemanticSnapshot {
            page_nodes,
            page_drafts,
        }
    }

    fn normalized_cli_phase_multiset(
        result: &seo_application::scenario::SeoScenarioResult,
    ) -> BTreeMap<String, usize> {
        let allowed = [
            "serp_ingest",
            "crawl_sources",
            "raw_knowledge_ingestion",
            "serp_normalize",
            "opportunity_build",
            "ia_build",
            "link_recommend",
            "global_site_reconcile",
            "draft_assemble",
            "editorial_draft_generate",
            "draft_normalize",
            "content_contract_validate",
            "draft_qa",
            "rebuild_detect",
        ];
        let mut counts = BTreeMap::new();
        for report in &result.phase_reports {
            if allowed.contains(&report.phase.as_str()) {
                *counts.entry(report.phase.clone()).or_insert(0) += 1;
            }
        }
        for phase in ["raw_knowledge_ingestion", "global_site_reconcile"] {
            if let Some(count) = counts.get_mut(phase) {
                *count = 1;
            }
        }
        counts
    }

    async fn temporal_step_multiset(pool: &sqlx::PgPool, run_id: &str) -> BTreeMap<String, usize> {
        let allowed = [
            "serp_ingest",
            "crawl_sources",
            "raw_knowledge_ingestion",
            "serp_normalize",
            "opportunity_build",
            "ia_build",
            "link_recommend",
            "global_site_reconcile",
            "draft_assemble",
            "editorial_draft_generate",
            "draft_normalize",
            "content_contract_validate",
            "draft_qa",
            "rebuild_detect",
        ];
        let rows = sqlx::query(
            r#"
            SELECT step_name
            FROM pipeline.step_executions
            WHERE run_id = $1
              AND status = 'done'
            ORDER BY step_name
            "#,
        )
        .bind(Uuid::parse_str(run_id).unwrap())
        .fetch_all(pool)
        .await
        .unwrap();
        let mut counts = BTreeMap::new();
        for row in rows {
            let step_name: String = row.get("step_name");
            if allowed.contains(&step_name.as_str()) {
                *counts.entry(step_name).or_insert(0) += 1;
            }
        }
        counts
    }

    #[tokio::test]
    async fn integration_ctx_exposes_stub_servers() {
        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": "https://example.gov/visa",
                            "description": "Visa rules"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":[]}"
                    }
                }]
            }),
        )
        .await
        .unwrap();

        let serp = reqwest::Client::new()
            .post(format!(
                "{}/v3/serp/google/organic/live/advanced",
                ctx.dataforseo.base_url
            ))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            serp["tasks"][0]["result"][0]["items"][0]["rank_absolute"],
            1
        );

        let completion = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", ctx.openai.base_url))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(completion["choices"][0]["message"]["role"], "assistant");
    }

    fn sample_editorial_input() -> EditorialDraftGenerateInputPayload {
        EditorialDraftGenerateInputPayload {
            run_id: "run-1".to_string(),
            request: Some(LlmDraftRequest {
                request_key: "req-1".to_string(),
                run_id: "run-1".to_string(),
                editorial_brief: Some(EditorialBrief {
                    editorial_brief_key: "brief-1".to_string(),
                    page_node_key: "page-1".to_string(),
                    page_brief_key: "page-brief-1".to_string(),
                    target_audience: "tourist".to_string(),
                    expertise_goal: "help applicant understand the process".to_string(),
                    section_templates: vec![SectionTemplateBinding {
                        template_key: "overview".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "informational".to_string(),
                        section_role: "overview".to_string(),
                        heading: "Overview".to_string(),
                        template_body: "overview".to_string(),
                        template_version: 1,
                        required: true,
                    }],
                    verified_support: vec![SeoVerifiedFactSupportState {
                        support_ref: "rule-1".to_string(),
                        role_type: "document_required".to_string(),
                        fragment_text: "Passport is required.".to_string(),
                        source_label: "Official source".to_string(),
                        source_tier: "official".to_string(),
                        freshness_class: "fresh".to_string(),
                        observed_at: "2026-05-08T00:00:00Z".to_string(),
                        valid_until: "2027-05-08T00:00:00Z".to_string(),
                    }],
                    required_links: vec![LinkRecommendationState::default()],
                    truth_snapshot_ref: "truth-1".to_string(),
                    source_context_chunks: Vec::new(),
                }),
                provider_policy: "multi_provider:first_available".to_string(),
                prompt_version: "editorial_prompt@1".to_string(),
                output_contract: "headless".to_string(),
            }),
            provider_policy: String::new(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn provider_env_drives_real_adapter_contracts_against_stubs() {
        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "type": "organic",
                            "rank_group": 1,
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": "https://example.gov/visa",
                            "description": "Visa rules"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":{\"overview\":\"Passport is required.\"}}"
                    }
                }]
            }),
        )
        .await
        .unwrap();
        let _env = ctx.apply_provider_env();

        let config = DataForSeoConfig::from_env_with_locale(Some("en-US"))
            .expect("dataforseo config from integration env");
        let serp = DataForSeoSerpClient::from_config(config)
            .unwrap()
            .google_organic_live_advanced("visa rules")
            .await
            .unwrap();
        assert_eq!(serp.organic_results.len(), 1);
        assert_eq!(serp.organic_results[0].title, "Official source");

        let editorial = editorial_llm_adapter::generate_editorial_draft(&sample_editorial_input())
            .await
            .expect("editorial draft via local-compatible stub");
        assert_eq!(editorial.provider_key, "local_compatible");
        let candidate = editorial.candidate.expect("editorial candidate");
        assert!(candidate
            .sections
            .iter()
            .any(|section| section.section_role == "overview"
                && section.body_markdown.contains("Passport is required.")));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn postgres_harness_bootstraps_schema() {
        let harness = PostgresHarness::start().await.unwrap();
        let schemas: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM information_schema.schemata WHERE schema_name IN ('pipeline', 'system', 'site')")
                .fetch_one(&harness.pool)
                .await
                .unwrap();
        assert_eq!(schemas, 3);
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn qdrant_harness_accepts_client_probe() {
        let harness = QdrantHarness::start().await.unwrap();
        let exists = harness
            .client
            .collection_exists("content_chunks")
            .await
            .unwrap();
        assert!(!exists);
        assert!(harness.qdrant_url.starts_with("http://127.0.0.1:"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn neo4j_harness_exposes_bolt_connection() {
        let harness = Neo4jHarness::start().await.unwrap();
        let graph = neo4rs::Graph::new(&harness.uri, &harness.user, &harness.password).unwrap();
        graph.run(neo4rs::query("RETURN 1 AS ok")).await.unwrap();
        assert!(harness.uri.starts_with("127.0.0.1:"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn temporal_harness_exposes_grpc_endpoint() {
        let harness = TemporalHarness::start().await.unwrap();
        assert!(harness.temporal_url.starts_with("http://127.0.0.1:"));
        assert_eq!(harness.namespace, "default");
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    async fn infra_harness_exposes_runtime_env_surface() {
        let harness = InfraHarness::start().await.unwrap();
        let env = harness.runtime_env();
        assert!(env.contains_key("DATABASE_URL"));
        assert!(env.contains_key("QDRANT_URL"));
        assert!(env.contains_key("NEO4J_URI"));
        assert!(env.contains_key("NEO4J_USER"));
        assert!(env.contains_key("NEO4J_PASSWORD"));
        assert!(env.contains_key("TEMPORAL_URL"));
        assert!(env.contains_key("TEMPORAL_NAMESPACE"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_promotes_corroborated_candidates_into_admissible_verified_truth() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-corroborated:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text = "Consular fee is 35 EUR";
        let (page_a, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-a.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;
        let (page_b, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-b.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![
                local_truth_response(local_truth_candidate_payload(
                    raw_text,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
                local_truth_response(local_truth_candidate_payload(
                    raw_text,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
            ],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_a, page_b],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 2);
        assert_eq!(report.verified_rule_count, 1);
        assert_eq!(report.needs_hitl_candidate_count, 0);

        let verified_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'verified'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(verified_candidates, 2);

        let verified_row = sqlx::query(
            "SELECT status, publish_admissibility, verification_method, adjudication_reason
             FROM verified.rule_instances
             WHERE context_key = $1 AND concept_key = 'consular_fee'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(verified_row.get::<String, _>("status"), "verified");
        assert_eq!(
            verified_row.get::<String, _>("publish_admissibility"),
            "admissible"
        );
        assert_eq!(
            verified_row.get::<String, _>("verification_method"),
            "cross_source_consensus@1"
        );
        assert!(verified_row
            .get::<String, _>("adjudication_reason")
            .contains("corroborated_structured_candidates"));
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_marks_incomplete_candidate_as_needs_hitl() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-needs-hitl:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text = "Consular fee is payable on submission.";
        let (page_id, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-c.example.gov/fees",
            "government",
            "Fees",
            raw_text,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![local_truth_response(local_truth_candidate_payload(
                raw_text,
                "FEE_ITEM",
                "consular_fee",
                serde_json::json!({"currency": "EUR"}),
            ))],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_id],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 1);
        assert_eq!(report.verified_rule_count, 0);
        assert_eq!(report.needs_hitl_candidate_count, 1);

        let needs_hitl_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'needs_hitl'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(needs_hitl_candidates, 1);

        let admissible_verified_rows: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM verified.rule_instances
             WHERE context_key = $1 AND publish_admissibility = 'admissible'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(admissible_verified_rows, 0);
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_runtime_keeps_conflicting_structured_candidates_out_of_verified_storage() {
        let harness = PostgresHarness::start().await.unwrap();
        let context_key = "es:truth-conflict:by";
        seed_truth_context_and_concept(
            &harness.pool,
            context_key,
            "consular_fee",
            "fee",
            "Консульский сбор",
        )
        .await;

        let raw_text_a = "Consular fee is 35 EUR";
        let raw_text_b = "Consular fee is 80 EUR";
        let (page_a, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-d.example.gov/fees",
            "government",
            "Fees",
            raw_text_a,
        )
        .await;
        let (page_b, _) = seed_raw_page_with_section(
            &harness.pool,
            "https://source-e.example.gov/fees",
            "government",
            "Fees",
            raw_text_b,
        )
        .await;

        let stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            vec![
                local_truth_response(local_truth_candidate_payload(
                    raw_text_a,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 35, "currency": "EUR"}),
                )),
                local_truth_response(local_truth_candidate_payload(
                    raw_text_b,
                    "FEE_ITEM",
                    "consular_fee",
                    serde_json::json!({"amount": 80, "currency": "EUR"}),
                )),
            ],
        )
        .await
        .unwrap();
        let _truth_env = env_overrides::EnvOverrideGuard::apply(local_truth_env(&format!(
            "{}/v1/chat/completions",
            stub.base_url
        )));

        let report = raw_crawl_adapter::ingest_raw_pages_into_verified(
            &harness.pool,
            &Uuid::new_v4().to_string(),
            context_key,
            &[page_a, page_b],
        )
        .await
        .unwrap();

        assert!(!report.extraction_provider_unavailable);
        assert_eq!(report.extracted_rule_count, 2);
        assert_eq!(report.verified_rule_count, 0);
        assert_eq!(report.needs_hitl_candidate_count, 2);

        let needs_hitl_candidates: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM extracted.rule_candidates
             WHERE context_key = $1 AND epistemic_status = 'needs_hitl'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(needs_hitl_candidates, 2);

        let admissible_verified_rows: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
             FROM verified.rule_instances
             WHERE context_key = $1 AND publish_admissibility = 'admissible'",
        )
        .bind(context_key)
        .fetch_one(&harness.pool)
        .await
        .unwrap();
        assert_eq!(admissible_verified_rows, 0);
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn truth_certification_suite_matches_fixture_expectations() {
        let fixtures = load_truth_certification_fixtures();
        let fixture_ids = env::var("TRUTH_CERT_FIXTURE_IDS").ok().map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        });
        let fixtures = if let Some(fixture_ids) = fixture_ids {
            fixtures
                .into_iter()
                .filter(|fixture| fixture_ids.contains(&fixture.fixture_id))
                .collect::<Vec<_>>()
        } else {
            fixtures
        };
        let fixture_limit = env::var("TRUTH_CERT_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok());
        let fixtures = if let Some(limit) = fixture_limit {
            fixtures.into_iter().take(limit).collect::<Vec<_>>()
        } else {
            fixtures
        };
        assert!(
            !fixtures.is_empty(),
            "expected at least one certification fixture"
        );

        let (source_servers, serp_sequence) = render_fixture_source_pages(&fixtures).await;
        assert_eq!(source_servers.len(), fixtures.iter().map(|fixture| fixture.source_inputs.pages.len()).sum::<usize>());
        let serp_stub = stub_servers::spawn_json_sequence_stub(
            "/v3/serp/google/organic/live/advanced",
            serp_sequence.into_iter().map(|(_, payload)| payload).collect(),
        )
        .await
        .unwrap();
        let truth_stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            truth_certification_truth_sequence(&fixtures),
        )
        .await
        .unwrap();
        let editorial_stub = stub_servers::spawn_json_sequence_stub(
            "/v1/chat/completions",
            truth_certification_editorial_sequence(&fixtures),
        )
        .await
        .unwrap();
        let infra = InfraHarness::start().await.unwrap();
        let temporal = TemporalHarness::start().await.unwrap();

        let mut worker_env = infra.runtime_env();
        worker_env.insert("TEMPORAL_URL".to_string(), temporal.temporal_url.clone());
        worker_env.insert("TEMPORAL_NAMESPACE".to_string(), temporal.namespace.clone());
        worker_env.extend(truth_certification_env(
            &format!(
                "{}/v3/serp/google/organic/live/advanced",
                serp_stub.base_url
            ),
            &format!("{}/v1/chat/completions", truth_stub.base_url),
            &format!("{}/v1/chat/completions", editorial_stub.base_url),
        ));
        let mut worker = spawn_temporal_worker(&worker_env);
        let mut outbox_worker = spawn_outbox_worker(&worker_env);
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            worker.child.try_wait().unwrap().is_none(),
            "temporal_worker exited before certification workflow start"
        );
        assert!(
            outbox_worker.child.try_wait().unwrap().is_none(),
            "outbox_worker exited before certification workflow start"
        );

        let client = connect_client(
            &temporal.temporal_url,
            format!("truth-cert-client-{}", Uuid::new_v4()),
            &temporal.namespace,
        )
        .await
        .unwrap();
        let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
        let mut reports = Vec::new();

        for fixture in &fixtures {
            assert_eq!(
                fixture.business_scope.truth_identity_tuple,
                "ES|tourist||BY",
                "fixture {} drifted from certification truth tuple",
                fixture.fixture_id
            );
            assert_eq!(
                fixture.business_scope.publishing_scope_tuple,
                "alegria-site|ru-RU|ES|tourist|BY",
                "fixture {} drifted from certification publish tuple",
                fixture.fixture_id
            );
            assert!(fixture
                .invariants
                .no_truth_promotion_from_retrieval_or_graph);
            assert!(fixture.invariants.no_source_tier_shortcut_to_verified);
            assert!(fixture
                .invariants
                .no_unsupported_claim_reaches_draft_as_truth);

            let run_id = Uuid::new_v4().to_string();
            let site_input = register_truth_certification_input(
                &repo,
                run_id.clone(),
                format!("truth-cert-batch-{}", fixture.fixture_id),
                fixture.source_inputs.query.clone(),
                fixture.run_mode.clone(),
            )
            .await;
            let scope_signature = site_input
                .scope
                .as_ref()
                .expect("scope payload")
                .scope_signature
                .clone();
            reset_truth_certification_state(&infra.postgres.pool, &site_input.context_key).await;
            seed_truth_certification_support_bundle(
                &infra.postgres.pool,
                &site_input.context_key,
                fixture,
            )
            .await;

            let approval_task = if fixture.run_mode == "full_auto_after_approval" {
                Some(tokio::spawn(insert_preapproved_decisions_when_ready(
                    infra.postgres.pool.clone(),
                    scope_signature.clone(),
                    Instant::now() + Duration::from_secs(30),
                )))
            } else {
                None
            };

            let handle = client
                .start_workflow(
                    UntypedWorkflow::new("SeoSiteBuildCanonicalCutoverWorkflow"),
                    RawValue::default(),
                    WorkflowStartOptions::new("alegria-pipeline", run_id.clone()).build(),
                )
                .await
                .unwrap();
            let workflow_result = timeout(
                Duration::from_secs(180),
                handle.get_result(WorkflowGetResultOptions::default()),
            )
            .await;
            let workflow_failed = match workflow_result {
                Ok(Ok(_)) => false,
                Ok(Err(_err)) => {
                    true
                }
                Err(err) => {
                    let diagnostics =
                        certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                    panic!(
                        "workflow timed out for fixture {} run_id={} error={err:?}\n{}",
                        fixture.fixture_id, run_id, diagnostics
                    );
                }
            };

            if let Some(task) = approval_task {
                task.await.unwrap();
            }

            let mut report = collect_truth_certification_fixture_report(
                &infra.postgres.pool,
                fixture,
                &run_id,
                &site_input.context_key,
                &scope_signature,
            )
            .await;
            if workflow_failed {
                let diagnostics =
                    certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                if !expected_workflow_failure_allowed(fixture, &report, &diagnostics) {
                    report.pass = false;
                    report
                        .failure_reasons
                        .push("workflow_failed".to_string());
                }
                report.workflow_failure_diagnostics = Some(diagnostics);
            }
            reports.push(report);
            if let Ok(report_path) = env::var("TRUTH_CERT_REPORT_PATH") {
                let partial = TruthCertificationSuiteReport {
                    workflow_type: "SeoSiteBuildCanonicalCutoverWorkflow".to_string(),
                    fixture_count: reports.len(),
                    fixtures: reports.clone(),
                };
                fs::write(&report_path, serde_json::to_vec_pretty(&partial).unwrap()).unwrap();
            }
        }

        let suite_report = TruthCertificationSuiteReport {
            workflow_type: "SeoSiteBuildCanonicalCutoverWorkflow".to_string(),
            fixture_count: reports.len(),
            fixtures: reports,
        };

        if let Ok(report_path) = env::var("TRUTH_CERT_REPORT_PATH") {
            fs::write(
                &report_path,
                serde_json::to_vec_pretty(&suite_report).unwrap(),
            )
            .unwrap();
        }

        assert!(
            suite_report.fixtures.iter().all(|fixture| fixture.pass),
            "truth certification mismatches: {}",
            serde_json::to_string_pretty(&suite_report).unwrap()
        );
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn synthetic_full_scenario_persists_page_draft_on_full_harness() {
        let source = stub_servers::spawn_text_stub(
            "/official/spain-tourist-visa",
            r#"
            <html>
              <head>
                <title>Spain Tourist Visa</title>
                <meta name="description" content="Official visa requirements" />
              </head>
              <body>
                <main>
                  <h1>Spain tourist visa for Belarusians</h1>
                  <p>Passport is required for the visa application.</p>
                  <p>The visa fee is 80 EUR.</p>
                  <p>Applications are submitted at the visa centre.</p>
                </main>
              </body>
            </html>
            "#,
            "text/html; charset=utf-8",
        )
        .await
        .unwrap();

        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "type": "organic",
                            "rank_group": 1,
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": format!("{}/official/spain-tourist-visa", source.base_url),
                            "description": "Official visa requirements"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":{\"overview\":\"Passport is required for the visa application.\"}}"
                    }
                }]
            }),
        )
        .await
        .unwrap();
        let infra = InfraHarness::start().await.unwrap();
        let _runtime_env = infra.apply_runtime_env();
        let _provider_env = ctx.apply_provider_env();

        let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
        let run_id = Uuid::new_v4().to_string();
        let query_batch_key = format!("synthetic-batch-{run_id}");
        let site_input = register_site_build_input(
            &repo,
            &SeoSiteBuildRegistrationRequest {
                run_id: run_id.clone(),
                context_key: None,
                market: "alegria-site".to_string(),
                locale: "ru-RU".to_string(),
                country_code: "ES".to_string(),
                visa_type: "tourist".to_string(),
                visa_subtype: None,
                applicant_profile: "standard".to_string(),
                citizenship_code: "BY".to_string(),
                bootstrap_context: true,
                queries: vec!["spain tourist visa belarus".to_string()],
                query_batch_key: Some(query_batch_key),
                run_mode: Some(
                    run_mode_for_scenario(SeoScenarioKind::Full, false, false, true).to_string(),
                ),
            },
        )
        .await
        .unwrap();

        seed_synthetic_truth(
            &infra.postgres.pool,
            &site_input.context_key,
            &source.base_url,
        )
        .await;

        let output_dir = std::env::temp_dir().join(format!("integration-harness-step3-{run_id}"));
        fs::create_dir_all(&output_dir).unwrap();

        let result = execute_site_build_scenario(
            &repo,
            &SeoScenarioRequest {
                scenario: SeoScenarioKind::Full,
                mode: SeoExecutionMode::SemiAutoOperator,
                policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                output_dir: output_dir.display().to_string(),
                base_url: "https://example.com".to_string(),
                site_input,
            },
        )
        .await
        .unwrap();

        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "serp_ingest"),
            "missing serp_ingest phase: {:?}",
            result.phase_reports
        );
        assert!(result.phase_reports.iter().any(|r| r.phase == "serp_ingest"));
        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "crawl_sources"),
            "missing crawl_sources phase: {:?}",
            result.phase_reports
        );
        assert!(result.phase_reports.iter().any(|r| r.phase == "crawl_sources"));
        assert!(
            result
                .phase_reports
                .iter()
                .any(|r| r.phase == "draft_assemble"),
            "missing draft_assemble phase: {:?}",
            result.phase_reports
        );
        assert!(result.phase_reports.iter().any(|r| r.phase == "draft_assemble"));

        let draft_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_drafts")
            .fetch_one(&infra.postgres.pool)
            .await
            .unwrap();
        assert!(
            draft_count >= 1,
            "expected at least one persisted page draft, status={} page_total={} phases={:?}",
            result.status,
            result.page_total,
            result.phase_reports
        );
    }

    #[cfg(feature = "e2e")]
    #[tokio::test]
    #[serial]
    async fn cli_vs_temporal_semantic_parity_on_full_harness() {
        let source = stub_servers::spawn_text_stub(
            "/official/spain-tourist-visa",
            r#"
            <html>
              <head>
                <title>Spain Tourist Visa</title>
                <meta name="description" content="Official visa requirements" />
              </head>
              <body>
                <main>
                  <h1>Spain tourist visa for Belarusians</h1>
                  <p>Passport is required for the visa application.</p>
                  <p>The visa fee is 80 EUR.</p>
                  <p>Applications are submitted at the visa centre.</p>
                </main>
              </body>
            </html>
            "#,
            "text/html; charset=utf-8",
        )
        .await
        .unwrap();

        let ctx = IntegrationCtx::start(
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": [{
                            "type": "organic",
                            "rank_group": 1,
                            "rank_absolute": 1,
                            "title": "Official source",
                            "url": format!("{}/official/spain-tourist-visa", source.base_url),
                            "description": "Official visa requirements"
                        }]
                    }]
                }]
            }),
            serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"sections\":{\"overview\":\"Passport is required for the visa application.\"}}"
                    }
                }]
            }),
        )
        .await
        .unwrap();

        let (cli_result, cli_snapshot, cli_phase_multiset) = {
            let cli_postgres = PostgresHarness::start().await.unwrap();
            let _cli_provider_env = ctx.apply_provider_env();
            let cli_repo = SqlxSeoRuntimeRepository::new(&cli_postgres.pool);
            let cli_run_id = Uuid::new_v4().to_string();
            let cli_input = register_synthetic_site_input(
                &cli_repo,
                cli_run_id.clone(),
                format!("synthetic-cli-batch-{cli_run_id}"),
            )
            .await;
            seed_synthetic_truth(&cli_postgres.pool, &cli_input.context_key, &source.base_url)
                .await;
            let cli_output_dir =
                std::env::temp_dir().join(format!("integration-harness-step4-cli-{cli_run_id}"));
            fs::create_dir_all(&cli_output_dir).unwrap();
            let cli_result = execute_site_build_scenario(
                &cli_repo,
                &SeoScenarioRequest {
                    scenario: SeoScenarioKind::Full,
                    mode: SeoExecutionMode::SemiAutoOperator,
                    policy: SeoRunPolicy::for_semi_auto_operator(false, false, true),
                    output_dir: cli_output_dir.display().to_string(),
                    base_url: "https://example.com".to_string(),
                    site_input: cli_input.clone(),
                },
            )
            .await
            .unwrap();
            let cli_scope_signature = cli_input.scope.as_ref().unwrap().scope_signature.clone();
            let cli_snapshot =
                capture_semantic_snapshot(&cli_postgres.pool, &cli_scope_signature).await;
            let cli_phase_multiset = normalized_cli_phase_multiset(&cli_result);
            (cli_result, cli_snapshot, cli_phase_multiset)
        };

        let temporal_postgres = PostgresHarness::start().await.unwrap();
        let temporal = TemporalHarness::start().await.unwrap();
        let temporal_repo = SqlxSeoRuntimeRepository::new(&temporal_postgres.pool);
        let temporal_run_id = Uuid::new_v4().to_string();
        let temporal_input = register_synthetic_site_input(
            &temporal_repo,
            temporal_run_id.clone(),
            format!("synthetic-temporal-batch-{temporal_run_id}"),
        )
        .await;
        seed_synthetic_truth(
            &temporal_postgres.pool,
            &temporal_input.context_key,
            &source.base_url,
        )
        .await;

        let mut worker_env = BTreeMap::new();
        worker_env.insert(
            "DATABASE_URL".to_string(),
            temporal_postgres.database_url.clone(),
        );
        worker_env.insert("TEMPORAL_URL".to_string(), temporal.temporal_url.clone());
        worker_env.insert("TEMPORAL_NAMESPACE".to_string(), temporal.namespace.clone());
        worker_env.extend(ctx.provider_env());
        let mut worker = spawn_temporal_worker(&worker_env);
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            worker.child.try_wait().unwrap().is_none(),
            "temporal_worker exited before workflow start"
        );

        let client = connect_client(
            &temporal.temporal_url,
            format!("integration-parity-client-{}", Uuid::new_v4()),
            &temporal.namespace,
        )
        .await
        .unwrap();
        let handle = client
            .start_workflow(
                UntypedWorkflow::new("SeoSiteBuildWorkflow"),
                RawValue::default(),
                WorkflowStartOptions::new("alegria-pipeline", temporal_run_id.clone()).build(),
            )
            .await
            .unwrap();
        timeout(
            Duration::from_secs(120),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("temporal parity workflow timed out")
        .expect("temporal parity workflow failed");

        let temporal_scope_signature = temporal_input
            .scope
            .as_ref()
            .unwrap()
            .scope_signature
            .clone();
        let temporal_snapshot =
            capture_semantic_snapshot(&temporal_postgres.pool, &temporal_scope_signature).await;
        let temporal_step_multiset =
            temporal_step_multiset(&temporal_postgres.pool, &temporal_run_id).await;

        assert_eq!(cli_result.status, "done");
        assert_eq!(
            cli_result.page_total as usize,
            cli_snapshot.page_nodes.len()
        );
        assert!(
            !cli_snapshot.page_drafts.is_empty(),
            "cli path produced no drafts"
        );
        assert_eq!(cli_snapshot, temporal_snapshot);
        assert_eq!(cli_phase_multiset, temporal_step_multiset);
    }
}
