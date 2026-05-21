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
        EditorialBrief, EditorialDraftGenerateInputPayload, LinkRecommendationState,
        LlmDraftRequest, SectionTemplateBinding, SeoVerifiedFactSupportState,
    };
    use infrastructure::adapters::{
        dataforseo_serp_adapter::{DataForSeoConfig, DataForSeoSerpClient},
        editorial_llm_adapter, raw_crawl_adapter,
        seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
        temporalio_sdk_adapter::{
            connect_client, RawValue, UntypedWorkflow, WorkflowGetResultOptions,
            WorkflowStartOptions,
        },
    };
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
        collections::BTreeMap,
        env, fs,
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::Duration,
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

    fn target_dir() -> PathBuf {
        env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root().join("target"))
    }

    fn build_temporal_worker_binary() -> PathBuf {
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
    }

    fn spawn_temporal_worker(envs: &BTreeMap<String, String>) -> ChildGuard {
        let binary = build_temporal_worker_binary();
        let mut command = Command::new(binary);
        command
            .current_dir(workspace_root())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
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
