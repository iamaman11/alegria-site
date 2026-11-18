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

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn persisted_wrapped_official_page_restores_sections_once_without_changing_source() {
    let pg = PostgresHarness::start().await.unwrap();
    let html = r#"<html><body><main>
        <div class="section__header-title"><h1>Условия въезда в Испанию</h1></div>
        <div class="single__text"><p>Официальный текст для проверки правил въезда.</p></div>
        </main></body></html>"#;
    let original_hash = primitives::hash::content_hash_v1(html);
    let page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages (url,domain,dtype,status_code,raw_html,raw_html_bytes,content_hash)
             VALUES ($1,'exteriores.gob.es','official',200,$2,$3,$4) RETURNING id",
    )
    .bind(format!(
        "https://www.exteriores.gob.es/test/{}",
        Uuid::new_v4()
    ))
    .bind(html)
    .bind(html.len() as i32)
    .bind(&original_hash)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    let first = raw_crawl_adapter::recover_empty_raw_sections(&pg.pool, page_id)
        .await
        .unwrap();
    assert_eq!(first, 1);
    let second = raw_crawl_adapter::recover_empty_raw_sections(&pg.pool, page_id)
        .await
        .unwrap();
    assert_eq!(second, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM raw.sections WHERE page_id = $1")
            .bind(page_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let row = sqlx::query(
            "SELECT raw_html, content_hash, content->>'section_count' AS count FROM raw.pages WHERE id=$1",
        )
        .bind(page_id)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("raw_html"), html);
    assert_eq!(row.get::<String, _>("content_hash"), original_hash);
    assert_eq!(row.get::<String, _>("count"), "1");
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn same_day_recrawl_preserves_original_evidence_and_appends_snapshot() {
    let pg = PostgresHarness::start().await.unwrap();
    let url = format!("https://example.gov/visa/{}", Uuid::new_v4());
    let robots = raw_crawl_adapter::RobotsDecisionTrace {
        allowed: true,
        ..Default::default()
    };
    let first_html =
            "<html><body><h1>Passport requirement</h1><p>Valid passport required for applicants.</p></body></html>";
    let second_html =
            "<html><body><h1>Insurance requirement</h1><p>Medical insurance required for applicants.</p></body></html>";
    let first = raw_crawl_adapter::save_crawled_html(
        &pg.pool,
        &url,
        "example.gov",
        &url,
        "official",
        200,
        "text/html",
        first_html,
        std::slice::from_ref(&url),
        &robots,
    )
    .await
    .unwrap();
    let old_sections: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM raw.sections WHERE page_id = $1 ORDER BY id")
            .bind(first.page_id)
            .fetch_all(&pg.pool)
            .await
            .unwrap();
    assert!(!old_sections.is_empty());

    // Previously this tried DELETE FROM raw.sections, breaking VERIFIED
    // evidence foreign keys. The original page/section IDs are durable.
    let second = raw_crawl_adapter::save_crawled_html(
        &pg.pool,
        &url,
        "example.gov",
        &url,
        "official",
        200,
        "text/html",
        second_html,
        std::slice::from_ref(&url),
        &robots,
    )
    .await
    .unwrap();
    assert_ne!(first.page_id, second.page_id);
    let preserved_sections: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM raw.sections WHERE page_id = $1 ORDER BY id")
            .bind(first.page_id)
            .fetch_all(&pg.pool)
            .await
            .unwrap();
    assert_eq!(preserved_sections, old_sections);
    let old_body: String = sqlx::query_scalar("SELECT raw_html FROM raw.pages WHERE id = $1")
        .bind(first.page_id)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(old_body, first_html);
    let fresh_body: String = sqlx::query_scalar("SELECT raw_html FROM raw.pages WHERE id = $1")
        .bind(second.page_id)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(fresh_body, second_html);
    let page_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM raw.pages WHERE url = $1")
            .bind(&url)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(page_count, 2);
}

#[tokio::test]
async fn integration_ctx_exposes_stub_servers() {
    let ctx = IntegrationCtx::start(
        serde_json::json!({
            "status_code": 20000,
            "tasks": [{
                "status_code": 20000,
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
        .get(format!(
            "{}/v3/serp/google/organic/task_get/advanced/stub-serp-0",
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

    let client = reqwest::Client::new();
    let endpoint = format!("{}/v1/chat/completions", ctx.openai.base_url);
    let legacy_request = serde_json::json!({
        "model": "stub-editorial-model",
        "messages": [{"role": "user", "content": "Legacy editorial request"}]
    });
    let completion = client
        .post(&endpoint)
        .json(&legacy_request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(completion["choices"][0]["message"]["role"], "assistant");
    let legacy_body = completion["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(legacy_body).unwrap(),
        serde_json::json!({"sections": []})
    );

    let allowed = serde_json::json!({
        "fees": [{"support_ref": "fee-1", "fragment_text": "Fee is 80 EUR."}]
    });
    let v2_request = serde_json::json!({
        "model": "stub-editorial-model",
        "messages": [{"role": "user", "content": format!("ALLOWED_CLAIMS_JSON: {allowed}")}]
    });
    let v2 = client
        .post(&endpoint)
        .json(&v2_request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let claims: Value =
        serde_json::from_str(v2["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        claims["claims"]["fees"],
        serde_json::json!([{"support_ref": "fee-1", "text": "Fee is 80 EUR."}])
    );
    assert!(claims["metadata"]["title"].is_string());
    assert!(claims.get("sections").is_none());

    for malformed in [
        serde_json::json!({}),
        serde_json::json!({"messages": [{"role": "user", "content": "ALLOWED_CLAIMS_JSON: invalid"}]}),
        serde_json::json!({"messages": [{"role": "user", "content": "ALLOWED_CLAIMS_JSON: []"}]}),
    ] {
        let response = client.post(&endpoint).json(&malformed).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    let no_body = client.post(&endpoint).send().await.unwrap();
    assert_eq!(no_body.status(), reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // The two LLM operations must use distinct stubs with distinct JSON
    // contracts: claims for editorial v2, candidates for truth extraction.
    let truth_completion = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", ctx.truth.base_url))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let truth_body = truth_completion["choices"][0]["message"]["content"]
        .as_str()
        .expect("truth stub text response");
    assert_eq!(
        serde_json::from_str::<Value>(truth_body).unwrap(),
        serde_json::json!({"candidates": []})
    );
    let provider_env = ctx.provider_env();
    assert_ne!(
        provider_env["SEO_TRUTH_LLM_LOCAL_ENDPOINT"],
        provider_env["SEO_LLM_LOCAL_ENDPOINT"]
    );
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
                    source_url: String::new(),
                }],
                required_links: vec![LinkRecommendationState::default()],
                truth_snapshot_ref: "truth-1".to_string(),
                source_context_chunks: Vec::new(),
                locale_code: "en".to_string(),
                canonical_url_path: "/en/visa/spain/".to_string(),
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
            "status_code": 20000,
            "tasks": [{
                "status_code": 20000,
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
    let client = DataForSeoSerpClient::from_config(config).unwrap();
    let posted = client.submit_standard_task("visa rules").await.unwrap();
    assert_eq!(posted["tasks"][0]["status_code"], 20100);
    let serp = client
        .fetch_standard_task("stub-serp-0")
        .await
        .unwrap()
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
        .collection_exists("raw_chunks_4")
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

// One real PostgreSQL inbox, no second workflow or mock approval path.
// The ChatGPT output is typed draft text, never a truth/publication decision.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn manual_llm_queue_persists_validates_replays_and_recovers() {
    use infrastructure::adapters::manual_llm_queue::{self, ManualLlmSubmission};

    let pg = PostgresHarness::start().await.unwrap();
    let mut input = sample_editorial_input();
    input.run_id = Uuid::new_v4().to_string();
    input.request.as_mut().unwrap().run_id = input.run_id.clone();
    input.provider_policy = "chat_manual".to_string();

    let pending = manual_llm_queue::enqueue_editorial(&pg.pool, &input)
        .await
        .unwrap();
    assert_eq!(pending.status, "awaiting_manual");
    assert!(pending.candidate.is_none());
    let job = manual_llm_queue::next_editorial(&pg.pool)
        .await
        .unwrap()
        .expect("pending manual job");
    assert_eq!(job.operation, "editorial_draft");
    assert_eq!(job.payload.run_id, input.run_id);
    assert_eq!(job.input_sha256.len(), 64);

    let payload = serde_json::json!({
        "task_id": job.task_id,
        "schema_version": job.schema_version,
        "input_sha256": job.input_sha256,
        "response": {
            "sections": {
                "overview": "A passport is required according to verified source material."
            }
        }
    });
    let mut bad_hash = payload.clone();
    bad_hash["input_sha256"] = serde_json::json!("wrong-hash");
    let reject: ManualLlmSubmission = serde_json::from_value(bad_hash).unwrap();
    assert!(
        manual_llm_queue::submit_editorial(&pg.pool, &reject, "gpt-6")
            .await
            .is_err()
    );

    let mut bad_role = payload.clone();
    bad_role["response"]["sections"] = serde_json::json!({
        "human_approval": "approved"
    });
    let reject: ManualLlmSubmission = serde_json::from_value(bad_role).unwrap();
    assert!(
        manual_llm_queue::submit_editorial(&pg.pool, &reject, "gpt-6")
            .await
            .is_err()
    );

    let mut bad_envelope = payload.clone();
    bad_envelope["response"]["publish_approved"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<ManualLlmSubmission>(bad_envelope).is_err(),
        "LLM must not be able to approve publication"
    );
    assert_eq!(
        manual_llm_queue::read_editorial_result(&pg.pool, &input)
            .await
            .unwrap()
            .status,
        "awaiting_manual"
    );

    let good: ManualLlmSubmission = serde_json::from_value(payload.clone()).unwrap();
    let run_id = manual_llm_queue::submit_editorial(&pg.pool, &good, "gpt-6")
        .await
        .unwrap();
    assert_eq!(run_id, input.run_id);

    // Simulate a CLI restart: read through a new physical DB connection.
    let reopened = sqlx::PgPool::connect(&pg.database_url).await.unwrap();
    let completed = manual_llm_queue::read_editorial_result(&reopened, &input)
        .await
        .unwrap();
    assert_eq!(completed.status, "done");
    assert_eq!(completed.provider_key, "chat_manual");
    assert_eq!(completed.model_key, "gpt-6");
    assert!(completed.candidate.unwrap().sections.iter().any(|section| {
        section.section_role == "overview"
            && section.body_markdown.contains("A passport is required")
    }));
    assert!(manual_llm_queue::next_editorial(&reopened)
        .await
        .unwrap()
        .is_none());

    // Retrying after a DB commit but before Temporal signaling is safe.
    assert_eq!(
        manual_llm_queue::submit_editorial(&reopened, &good, "gpt-6")
            .await
            .unwrap(),
        input.run_id
    );
    assert!(
        manual_llm_queue::submit_editorial(&reopened, &good, "other-model")
            .await
            .is_err()
    );
    let mut altered = payload;
    altered["response"]["sections"]["overview"] = serde_json::json!("Changed answer");
    let altered: ManualLlmSubmission = serde_json::from_value(altered).unwrap();
    assert!(
        manual_llm_queue::submit_editorial(&reopened, &altered, "gpt-6")
            .await
            .is_err()
    );
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn manual_truth_inbox_refuses_fabricated_evidence_and_replays_atomically() {
    use infrastructure::adapters::{
        manual_llm_queue::{self, ManualTruthSubmission},
        truth_extraction_llm_adapter::{TruthExtractionInput, TruthExtractionRevision},
    };
    let pg = PostgresHarness::start().await.unwrap();
    let run_id = Uuid::new_v4().to_string();
    let source = TruthExtractionInput {
        revision: TruthExtractionRevision::Legacy,
        context_key: "test:visa:by".to_string(),
        raw_section_id: 42,
        source_url: "https://example.gov/requirements".to_string(),
        source_domain: "example.gov".to_string(),
        heading_path: "Documents".to_string(),
        raw_text: "Passport is required.".to_string(),
        source_snapshot_hash: "confirmed-source-snapshot".to_string(),
    };
    manual_llm_queue::enqueue_truth(&pg.pool, &run_id, &source)
        .await
        .unwrap();
    assert_eq!(
        manual_llm_queue::truth_run_status(&pg.pool, &run_id)
            .await
            .unwrap(),
        "awaiting_manual"
    );
    let job = manual_llm_queue::next_job(&pg.pool)
        .await
        .unwrap()
        .expect("pending truth job");
    assert_eq!(job.operation, "truth_candidates");
    assert_eq!(job.payload["raw_section_id"], 42);
    assert!(job.payload.get("revision").is_none());
    assert!(job.instructions.as_deref().unwrap().contains("exact evidence spans"));
    // A retry after upgrading the prompt must retain the historical task.
    let mut upgraded = source.clone();
    upgraded.revision = TruthExtractionRevision::AtomicCoverage;
    manual_llm_queue::enqueue_truth(&pg.pool, &run_id, &upgraded)
        .await
        .unwrap();
    let same = manual_llm_queue::next_job(&pg.pool).await.unwrap().unwrap();
    assert_eq!(same.task_id, job.task_id);
    let mut changed_source = upgraded.clone();
    changed_source.raw_text.push_str(" Another requirement.");
    assert!(manual_llm_queue::enqueue_truth(&pg.pool, &run_id, &changed_source)
        .await
        .is_err());
    let candidate = serde_json::json!({
        "role": "DOCUMENT_REQUIRED",
        "concept_canonical_key": "passport",
        "raw_mention": "Passport",
        "params": {"subtype": "passport", "severity": "mandatory"},
        "scope": {},
        "severity": "mandatory",
        "applies_to_profiles": [],
        "alternatives": [],
        "derivation_type": "direct",
        "is_numeric": false,
        "is_range": false,
        "is_incomplete": false,
        "confidence": 0.95,
        "evidence_section_id": 42,
        "evidence_quote": "Passport is required.",
        "span_start": 0,
        "span_end": source.raw_text.len(),
        "uncertainty_flags": []
    });
    let good = serde_json::json!({
        "task_id": job.task_id,
        "schema_version": 1,
        "input_sha256": job.input_sha256,
        "response": {"candidates": [candidate]}
    });
    let mut invented = good.clone();
    invented["response"]["candidates"][0]["evidence_quote"] =
        serde_json::json!("Fabricated government rule");
    let wrong: ManualTruthSubmission = serde_json::from_value(invented).unwrap();
    assert!(manual_llm_queue::submit_truth(&pg.pool, &wrong, "gpt-6")
        .await
        .is_err());
    let mut wrong_id = good.clone();
    wrong_id["input_sha256"] = serde_json::json!("invalid");
    let wrong: ManualTruthSubmission = serde_json::from_value(wrong_id).unwrap();
    assert!(manual_llm_queue::submit_truth(&pg.pool, &wrong, "gpt-6")
        .await
        .is_err());
    let mut fake_approval = good.clone();
    fake_approval["response"]["verified"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ManualTruthSubmission>(fake_approval).is_err());
    assert_eq!(
        manual_llm_queue::truth_run_status(&pg.pool, &run_id)
            .await
            .unwrap(),
        "awaiting_manual"
    );
    let submitted: ManualTruthSubmission = serde_json::from_value(good.clone()).unwrap();
    assert_eq!(
        manual_llm_queue::submit_truth(&pg.pool, &submitted, "gpt-6")
            .await
            .unwrap(),
        run_id
    );
    let reopened = sqlx::PgPool::connect(&pg.database_url).await.unwrap();
    assert_eq!(
        manual_llm_queue::truth_run_status(&reopened, &run_id)
            .await
            .unwrap(),
        "done"
    );
    let results = manual_llm_queue::read_truth_results(&reopened, &run_id)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1.candidates.len(), 1);
    assert_eq!(results[0].1.provider_key, "chat_manual");
    assert_eq!(results[0].1.prompt_version, "truth_extraction_prompt@1");
    assert_eq!(results[0].0.raw_text, source.raw_text);
    let fresh_run = Uuid::new_v4().to_string();
    manual_llm_queue::enqueue_truth(&reopened, &fresh_run, &upgraded)
        .await
        .unwrap();
    let fresh = manual_llm_queue::next_job(&reopened).await.unwrap().unwrap();
    assert_ne!(fresh.task_id, job.task_id);
    assert_eq!(fresh.payload["revision"], "truth_extraction_prompt@2");
    assert!(fresh.instructions.as_deref().unwrap().contains("Atomic coverage protocol"));
    assert_eq!(
        manual_llm_queue::submit_truth(&reopened, &submitted, "gpt-6")
            .await
            .unwrap(),
        run_id
    );
    assert!(
        manual_llm_queue::submit_truth(&reopened, &submitted, "other-model")
            .await
            .is_err()
    );
    let mut changed = good;
    changed["response"]["candidates"][0]["confidence"] = serde_json::json!(0.84);
    let changed: ManualTruthSubmission = serde_json::from_value(changed).unwrap();
    assert!(manual_llm_queue::submit_truth(&reopened, &changed, "gpt-6")
        .await
        .is_err());
}


#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn changed_editorial_request_supersedes_only_pending_same_page_task() {
    use infrastructure::adapters::manual_llm_queue::{self, ManualLlmSubmission};
    let pg = PostgresHarness::start().await.unwrap();
    let mut original = sample_editorial_input();
    original.run_id = Uuid::new_v4().to_string();
    original.request.as_mut().unwrap().run_id = original.run_id.clone();
    original.provider_policy = "chat_manual".to_string();
    manual_llm_queue::enqueue_editorial(&pg.pool, &original).await.unwrap();
    let before = manual_llm_queue::next_editorial(&pg.pool).await.unwrap().unwrap();

    let mut replacement = original.clone();
    replacement.request.as_mut().unwrap().request_key.push_str("-revised");
    manual_llm_queue::enqueue_editorial(&pg.pool, &replacement).await.unwrap();
    let after = manual_llm_queue::next_editorial(&pg.pool).await.unwrap().unwrap();
    assert_ne!(before.task_id, after.task_id);
    assert!(manual_llm_queue::enqueue_editorial(&pg.pool, &original).await.is_err());
    let obsolete: ManualLlmSubmission = serde_json::from_value(serde_json::json!({
        "task_id": before.task_id, "schema_version": before.schema_version,
        "input_sha256": before.input_sha256,
        "response": {"sections":{"overview":"A passport is required."}}
    })).unwrap();
    assert!(manual_llm_queue::submit_editorial(&pg.pool, &obsolete, "gpt-6").await.is_err());
    assert_eq!(manual_llm_queue::next_editorial(&pg.pool).await.unwrap().unwrap().task_id, after.task_id);
    let obsolete_status: String = sqlx::query_scalar(
        "SELECT job_status FROM pipeline.manual_llm_jobs WHERE task_id=$1",
    ).bind(&obsolete.task_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(obsolete_status, "superseded");
}


#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn retained_corpus_reuses_exact_paid_result_and_raw_pages_without_acquisition() {
    use contracts::generated::alegria::temporal::v1::{
        CrawlSourcesInputPayload, CrawlSourcesOutputPayload, SerpIngestInputPayload,
        SerpIngestOutputPayload,
    };
    use infrastructure::adapters::{
        raw_crawl_adapter::{self, RobotsDecisionTrace},
        seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
        sqlx_seo_adapter, sqlx_step_ledger_adapter as ledger,
    };
    use seo_application::registration::register_site_build_input;
    use seo_ports::SeoSiteBuildRegistrationRequest;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let repo = SqlxSeoRuntimeRepository::new(pool);
    let source_id = Uuid::new_v4().to_string();
    let query = "spain visa documents".to_string();
    let request = SeoSiteBuildRegistrationRequest {
        run_id: source_id.clone(),
        context_key: None, market: "alegria-site".into(), locale: "ru-RU".into(),
        country_code: "ES".into(), visa_type: "tourist".into(), visa_subtype: None,
        applicant_profile: "standard".into(), citizenship_code: "BY".into(),
        bootstrap_context: true, queries: vec![query.clone()],
        query_batch_key: None, run_mode: Some("publish_with_hitl".into()),
        retained_source_run_id: None,
    };
    let source = register_site_build_input(&repo,&request).await.unwrap();
    let mut reuse = request.clone();
    reuse.run_id = Uuid::new_v4().to_string();
    reuse.context_key = Some(source.context_key.clone());
    reuse.bootstrap_context = false;
    reuse.queries.clear();
    reuse.retained_source_run_id = Some(source_id.clone());
    // Nonterminal acquisition cannot be reused.
    assert!(register_site_build_input(&repo,&reuse).await.is_err());

    let url = "https://example.org/visa";
    let raw = raw_crawl_adapter::save_crawled_html(pool,url,"example.org",url,"official",
        200,"text/html","<html><body><main><h1>Required documents</h1><p>A passport is required for the application.</p></main></body></html>",
        &[url.to_string()],&RobotsDecisionTrace { allowed:true,..Default::default() })
        .await.unwrap();
    assert!(raw.section_count>0);
    let empty_url = "https://example.org/unavailable";
    let unavailable = raw_crawl_adapter::save_crawled_html(pool,empty_url,"example.org",empty_url,"official",
        202,"text/html","",&[empty_url.to_string()],&RobotsDecisionTrace { allowed:true,..Default::default() })
        .await.unwrap();

    let provider_result = serde_json::json!({
        "status_code":20000,"tasks":[{"status_code":20000,"result":[{"items":[
            {"type":"organic","rank_group":1,"rank_absolute":1,"url":url,"title":"Official source"}
        ]}]}]
    });
    sqlx::query("INSERT INTO serp.raw_snapshots(run_id,job_id,query,raw_result) VALUES($1,'source-job',$2,$3)")
        .bind(&source_id).bind(&query).bind(sqlx::types::Json(&provider_result)).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO serp.gemini_top10(run_id,job_id,rank,url,title,source_tier) VALUES($1,'source-job',1,$2,'Official source','official')")
        .bind(&source_id).bind(url).execute(pool).await.unwrap();

    let serp_receipt=SerpIngestOutputPayload {
        query_batch_key:source.query_batch_key.clone(),query_count:1,persisted_snapshot_count:1,
        status:"done".into(),
    };
    let crawl_receipt=CrawlSourcesOutputPayload {
        claimed_count:2,crawled_count:2,raw_page_count:2,raw_section_count:(raw.section_count+unavailable.section_count) as u32,
        status:"done".into(),raw_page_ids:vec![raw.page_id,unavailable.page_id],..Default::default()
    };
    let serp_key = format!("{source_id}-serp");
    let crawl_key = format!("{source_id}-crawl");
    for (step,key) in [("serp_ingest",&serp_key),("crawl_sources",&crawl_key)] {
        ledger::begin_step_execution(pool,&source_id,step,1,"fixture",key).await.unwrap();
    }
    let hash=ledger::write_step_payload_blob_typed(pool,&source_id,"serp_ingest",&serp_key,"output",1,&serp_receipt).await.unwrap();
    ledger::complete_step_execution(pool,&source_id,"serp_ingest",&serp_key,&hash).await.unwrap();
    let hash=ledger::write_step_payload_blob_typed(pool,&source_id,"crawl_sources",&crawl_key,"output",1,&crawl_receipt).await.unwrap();
    ledger::complete_step_execution(pool,&source_id,"crawl_sources",&crawl_key,&hash).await.unwrap();
    sqlx::query("UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1::uuid")
        .bind(&source_id).execute(pool).await.unwrap();

    let retained = register_site_build_input(&repo,&reuse).await.unwrap();
    assert_eq!(retained.queries,source.queries);
    assert_ne!(retained.query_batch_key,source.query_batch_key);
    let ingest_input=SerpIngestInputPayload {
        run_id:retained.run_id.clone(),query_batch_key:retained.query_batch_key.clone(),
        scope:retained.scope.clone(),queries:retained.queries.clone(),
    };
    let first=sqlx_seo_adapter::retained_serp_ingest(pool,&ingest_input).await.unwrap().unwrap();
    assert_eq!(first.persisted_snapshot_count,1);
    let repeat=sqlx_seo_adapter::retained_serp_ingest(pool,&ingest_input).await.unwrap().unwrap();
    assert_eq!(first,repeat);
    let crawl_input=CrawlSourcesInputPayload {
        run_id:retained.run_id.clone(),query_batch_key:retained.query_batch_key.clone(),
        limit:25,emit_qdrant: false,sweep_index:0,
    };
    let reused=sqlx_seo_adapter::retained_crawl_sources(pool,&crawl_input).await.unwrap().unwrap();
    assert_eq!(reused.raw_page_ids,vec![raw.page_id]);
    assert_eq!(reused.crawled_count,0);
    assert_eq!(reused.failed_urls,vec![empty_url]);
    assert_eq!(reused.status,"partial");
    let queued: i64=sqlx::query_scalar("SELECT count(*) FROM serp.crawl_queue WHERE first_seen_run_id=$1")
        .bind(&retained.run_id).fetch_one(pool).await.unwrap();
    assert_eq!(queued,0);
    let paid:i64=sqlx::query_scalar("SELECT count(*) FROM serp.raw_snapshots WHERE run_id=$1 AND job_id LIKE 'dataforseo-standard-task-%'")
        .bind(&retained.run_id).fetch_one(pool).await.unwrap();
    assert_eq!(paid,0);
    let ranks:i64=sqlx::query_scalar("SELECT count(*) FROM serp.gemini_top10 WHERE run_id=$1 AND rank=1 AND url=$2")
        .bind(&retained.run_id).bind(url).fetch_one(pool).await.unwrap();
    assert_eq!(ranks,1);
    // A mismatched retained receipt cannot fall back to live acquisition.
    let mut wrong=ingest_input.clone();
    wrong.queries.push("unrelated".into());
    assert!(sqlx_seo_adapter::retained_serp_ingest(pool,&wrong).await.is_err());
}


#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn keyword_cluster_natural_owner_survives_changed_serp_family() {
    use contracts::generated::alegria::temporal::v1::{
        KeywordClusterState, OpportunityBuildInputPayload, OpportunityBuildOutputPayload,
    };
    use infrastructure::adapters::{
        seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository, sqlx_seo_adapter,
    };
    use seo_application::registration::register_site_build_input;
    use seo_ports::SeoSiteBuildRegistrationRequest;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let repo = SqlxSeoRuntimeRepository::new(pool);
    let register = |run_id: String| SeoSiteBuildRegistrationRequest {
        run_id,
        context_key: None,
        market: "alegria-site".into(),
        locale: "ru-RU".into(),
        country_code: "ES".into(),
        visa_type: "tourist".into(),
        visa_subtype: None,
        applicant_profile: "standard".into(),
        citizenship_code: "BY".into(),
        bootstrap_context: true,
        queries: vec!["виза в испанию для белорусов".into()],
        query_batch_key: None,
        run_mode: Some("publish_with_hitl".into()),
        retained_source_run_id: None,
    };
    let first_site = register_site_build_input(&repo, &register(Uuid::new_v4().to_string()))
        .await.unwrap();
    let second_site = register_site_build_input(&repo, &register(Uuid::new_v4().to_string()))
        .await.unwrap();
    let keyword = "виза в испанию для белорусов";
    let old_cluster = KeywordClusterState {
        cluster_key: "original-natural-owner".into(),
        scope_signature: first_site.scope.as_ref().unwrap().scope_signature.clone(),
        seed_keyword: keyword.into(),
        dominant_intent: "informational".into(),
        status: "candidate".into(),
        cluster_version: 1,
        reason_code: "serp_seed_cluster".into(),
        ..Default::default()
    };
    let first = sqlx_seo_adapter::persist_opportunity_build_output(
        pool,
        &OpportunityBuildInputPayload {
            run_id: first_site.run_id,
            scope: first_site.scope.clone(),
            ..Default::default()
        },
        &OpportunityBuildOutputPayload {
            keyword_clusters: vec![old_cluster.clone()],
            ..Default::default()
        },
    ).await.unwrap();
    assert_eq!(first.keyword_clusters[0].cluster_key, "original-natural-owner");

    let mut changed_family = old_cluster;
    changed_family.cluster_key = "new-technical-serp-family".into();
    changed_family.dominant_intent = "commercial".into();
    let second = sqlx_seo_adapter::persist_opportunity_build_output(
        pool,
        &OpportunityBuildInputPayload {
            run_id: second_site.run_id,
            scope: second_site.scope,
            ..Default::default()
        },
        &OpportunityBuildOutputPayload {
            keyword_clusters: vec![changed_family],
            ..Default::default()
        },
    ).await.unwrap();
    assert_eq!(second.keyword_clusters.len(), 1);
    assert_eq!(second.keyword_clusters[0].cluster_key, "original-natural-owner");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.keyword_clusters WHERE seed_keyword=$1 AND cluster_version=1",
    ).bind(keyword).fetch_one(pool).await.unwrap();
    assert_eq!(count, 1, "repeat processing must preserve the natural-key owner");
    let intent: String = sqlx::query_scalar(
        "SELECT dominant_intent FROM site.keyword_clusters WHERE cluster_key='original-natural-owner'",
    ).fetch_one(pool).await.unwrap();
    assert_eq!(intent, "commercial");
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn content_gap_owner_identity_is_idempotent_and_independently_resolved() {
    use contracts::generated::alegria::temporal::v1::{
        ContentGapState, GlobalSiteReconcileInputPayload, IaBuildInputPayload,
        OpportunityBuildInputPayload, OpportunityBuildOutputPayload,
    };
    use infrastructure::adapters::{
        seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository, sqlx_seo_adapter,
    };
    use primitives::{errors::ErrorClass, seo::ContentGapDetector};
    use seo_application::registration::register_site_build_input;
    use seo_ports::SeoSiteBuildRegistrationRequest;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let repo = SqlxSeoRuntimeRepository::new(pool);
    let site = register_site_build_input(
        &repo,
        &SeoSiteBuildRegistrationRequest {
            run_id: Uuid::new_v4().to_string(),
            context_key: None, market: "alegria-site".into(), locale: "ru-RU".into(),
            country_code: "ES".into(), visa_type: "tourist".into(), visa_subtype: None,
            applicant_profile: "standard".into(), citizenship_code: "BY".into(),
            bootstrap_context: true, queries: vec!["spain visa".into()],
            query_batch_key: None, run_mode: Some("publish_with_hitl".into()),
            retained_source_run_id: None,
        },
    ).await.unwrap();
    let ia_input = IaBuildInputPayload {
        run_id: site.run_id.clone(), scope: site.scope.clone(), ..Default::default()
    };
    let ia = seo_steps::ia_build_step::execute(&ia_input);
    assert_eq!(ia.page_nodes.len(), 2);
    sqlx_seo_adapter::persist_ia_build_output(pool, &ia_input, &ia).await.unwrap();

    let reconcile = seo_steps::global_site_reconcile_step::execute(&GlobalSiteReconcileInputPayload {
        run_id: site.run_id.clone(), scope: site.scope.clone(),
        page_nodes: ia.page_nodes.clone(), page_blueprints: ia.page_blueprints.clone(),
        ..Default::default()
    });
    let shared = reconcile.content_gaps.into_iter()
        .filter(|gap| gap.missing_topic == "who_fits")
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 2, "both hubs have the same missing factual role");
    assert_ne!(shared[0].page_node_key, shared[1].page_node_key);
    assert_ne!(shared[0].content_gap_key, shared[1].content_gap_key);
    let scope = shared[0].scope_signature.clone();
    let mut gaps = shared.clone();
    gaps.push(ContentGapState {
        content_gap_key: ContentGapDetector::SerpReliability.key(&scope, "", "who_fits"),
        scope_signature: scope.clone(), missing_topic: "who_fits".into(),
        severity: "medium".into(), status: "open".into(),
        reason_code: ContentGapDetector::SerpReliability.reason().into(),
        ..Default::default()
    });
    gaps.push(ContentGapState {
        content_gap_key: ContentGapDetector::GraphCoverage.key(&scope, &shared[0].page_node_key, "who_fits"),
        reason_code: ContentGapDetector::GraphCoverage.reason().into(),
        ..shared[0].clone()
    });
    let req = OpportunityBuildInputPayload {
        run_id: site.run_id.clone(), scope: site.scope.clone(), ..Default::default()
    };
    let output = OpportunityBuildOutputPayload {
        content_gaps: gaps.clone(), ..Default::default()
    };
    for _ in 0..2 {
        sqlx_seo_adapter::persist_opportunity_build_output(pool, &req, &output).await.unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM site.content_gaps WHERE scope_signature=$1")
        .bind(&scope).fetch_one(pool).await.unwrap();
    assert_eq!(count, 4);
    let mut resolved = shared[0].clone();
    resolved.status = "resolved".into();
    sqlx_seo_adapter::persist_opportunity_build_output(pool, &req, &OpportunityBuildOutputPayload {
        content_gaps: vec![resolved], ..Default::default()
    }).await.unwrap();
    for (index, gap) in gaps.iter().enumerate() {
        let status: String = sqlx::query_scalar("SELECT status FROM site.content_gaps WHERE content_gap_key=$1")
            .bind(&gap.content_gap_key).fetch_one(pool).await.unwrap();
        assert_eq!(status, if index == 0 { "resolved" } else { "open" });
    }
    // The DB, independently of Rust, must reject duplicate scope-wide owners.
    let duplicate = sqlx::query(
        "INSERT INTO site.content_gaps(content_gap_key,scope_signature,missing_topic,detector_version)
         VALUES('duplicate-scope-key',$1,'who_fits','serp_reliability@1')",
    ).bind(&scope).execute(pool).await.unwrap_err();
    assert_eq!(duplicate.as_database_error().unwrap().code().as_deref(), Some("23505"));

    // Closed detector ownership and SQLSTATE classification are fail-closed.
    let mut invalid = shared[0].clone();
    invalid.page_node_key.clear();
    let error = sqlx_seo_adapter::persist_opportunity_build_output(pool, &req, &OpportunityBuildOutputPayload {
        content_gaps: vec![invalid], ..Default::default()
    }).await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::ContractViolation);
    let mut conflicting = ia.clone();
    conflicting.page_nodes[0].page_node_key = "different-identity-same-public-path".into();
    conflicting.page_nodes.truncate(1);
    let error = sqlx_seo_adapter::persist_ia_build_output(pool, &ia_input, &conflicting)
        .await.unwrap_err();
    assert_eq!(error.class(), ErrorClass::ConflictViolation);
    assert!(error.to_string().contains("SQLSTATE=23505"));
}
