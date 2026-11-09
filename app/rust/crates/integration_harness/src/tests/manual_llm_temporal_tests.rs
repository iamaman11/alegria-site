// The real canonical workflow must not bypass a pending ChatGPT task.
// Restart the actual Temporal worker while the task is waiting and finish
// through the same Rust PostgreSQL inbox + dedicated Temporal signal.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn canonical_temporal_manual_editorial_resumes_after_worker_restart() {
    use contracts::generated::alegria::temporal::v1::EditorialDraftGenerateInputPayload;
    use infrastructure::adapters::{
        manual_llm_queue::{self, ManualLlmJob, ManualLlmSubmission},
        seo_workflow_control_adapter::TemporalSeoWorkflowControlAdapter,
    };

    let fixture = load_truth_certification_fixtures()
        .into_iter()
        .find(|fixture| fixture.fixture_id == "happy_path_publish_allowed")
        .expect("existing canonical happy-path certification fixture");
    assert_eq!(fixture.run_mode, "draft_only");
    let (source_servers, serp_responses, rendered_pages) =
        render_fixture_source_pages(std::slice::from_ref(&fixture)).await;
    assert!(!source_servers.is_empty());
    let infra = InfraHarness::start().await.unwrap();
    let voyage_stub = stub_servers::spawn_voyage_contract_stub().await.unwrap();
    let serp_stub = stub_servers::spawn_json_sequence_stub(
        "/v3/serp/google/organic/live/advanced",
        vec![serp_responses[0].1.clone()],
    )
    .await
    .unwrap();
    let truth_stub = stub_servers::spawn_json_sequence_stub(
        "/v1/chat/completions",
        truth_certification_truth_sequence(std::slice::from_ref(&fixture)),
    )
    .await
    .unwrap();
    let editorial_stub = stub_servers::spawn_json_sequence_stub(
        "/v1/chat/completions",
        truth_certification_editorial_sequence(std::slice::from_ref(&fixture)),
    )
    .await
    .unwrap();
    let mut worker_env = infra.runtime_env();
    worker_env.extend(truth_certification_env(
        &format!(
            "{}/v3/serp/google/organic/live/advanced",
            serp_stub.base_url
        ),
        &format!("{}/v1/chat/completions", truth_stub.base_url),
        &format!("{}/v1/chat/completions", editorial_stub.base_url),
    ));
    worker_env.insert("SEO_LLM_PROVIDER".to_string(), "chat_manual".to_string());
    worker_env.insert(
        "SEO_TRUTH_LLM_PROVIDER".to_string(),
        "chat_manual".to_string(),
    );
    // The restarted process must retain the same Temporal Worker build identity.
    worker_env.insert(
        "WORKER_BUILD_ID".to_string(),
        format!("manual-acceptance-{}", Uuid::new_v4()),
    );
    worker_env.insert("VOYAGE_API_KEY".to_string(), "truth-cert-stub".to_string());
    worker_env.insert(
        "VOYAGE_API_BASE_URL".to_string(),
        format!("{}/v1", voyage_stub.base_url),
    );

    let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
    let run_id = Uuid::new_v4().to_string();
    let input = register_truth_certification_input(
        &repo,
        run_id.clone(),
        format!("manual-temporal-batch-{run_id}"),
        fixture.source_inputs.query.clone(),
        fixture.run_mode.clone(),
    )
    .await;
    reset_truth_certification_state(&infra.postgres.pool, &input.context_key).await;
    seed_truth_certification_source_governance(&infra.postgres.pool, &fixture, &rendered_pages)
        .await;
    seed_truth_certification_support_bundle(&infra.postgres.pool, &input.context_key, &fixture)
        .await;
    let _outbox = spawn_outbox_worker(&worker_env);
    let worker = spawn_temporal_worker(&worker_env);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let client = connect_client(
        &infra.temporal.temporal_url,
        format!("manual-temporal-client-{run_id}"),
        &infra.temporal.namespace,
    )
    .await
    .unwrap();
    let handle = client
        .start_workflow(
            UntypedWorkflow::new("SeoSiteBuildCanonicalCutoverWorkflow"),
            RawValue::default(),
            WorkflowStartOptions::new("alegria-pipeline", run_id.clone()).build(),
        )
        .await
        .unwrap();

    // Before editorial generation, prove that the same Temporal run waits
    // for every raw-section truth proposal in the same durable inbox.
    // An explicit empty candidate array is valid but cannot create verified truth.
    let truth_pool = infra.postgres.pool.clone();
    let truth_run = run_id.clone();
    let truth_temporal = infra.temporal.temporal_url.clone();
    let truth_namespace = infra.temporal.namespace.clone();
    let truth_driver = tokio::spawn(async move {
        let control = TemporalSeoWorkflowControlAdapter::new(
            truth_temporal,
            format!("manual-truth-fixture-{truth_run}"),
            truth_namespace,
        );
        let mut count = 0usize;
        loop {
            if let Some(job) = manual_llm_queue::next_job(&truth_pool).await.unwrap() {
                if job.operation == "editorial_draft" {
                    return count;
                }
                assert_eq!(job.operation, "truth_candidates");
                assert_eq!(
                    job.payload["context_key"].as_str().unwrap().is_empty(),
                    false
                );
                // One real, byte-anchored source claim must pass the
                // *canonical* validator, not just the JSON inbox parser.
                let raw_text = job.payload["raw_text"].as_str().unwrap();
                let section_id = job.payload["raw_section_id"].as_i64().unwrap();
                let quote = "Passport is required for the visa application.";
                let candidates = raw_text
                    .find(quote)
                    .map(|start| {
                        vec![serde_json::json!({
                            "role": "DOCUMENT_REQUIRED",
                            "concept_canonical_key": "passport",
                            "raw_mention": quote,
                            "params": {"severity": "mandatory", "subtype": "passport"},
                            "scope": {},
                            "severity": "mandatory",
                            "alternatives": [],
                            "derivation_type": "direct",
                            "confidence": 0.95,
                            "evidence_section_id": section_id,
                            "evidence_quote": quote,
                            "span_start": start,
                            "span_end": start + quote.len(),
                        })]
                    })
                    .unwrap_or_default();
                let submission: manual_llm_queue::ManualTruthSubmission =
                    serde_json::from_value(serde_json::json!({
                        "task_id": job.task_id,
                        "schema_version": job.schema_version,
                        "input_sha256": job.input_sha256,
                        "response": {"candidates": candidates},
                    }))
                    .unwrap();
                manual_llm_queue::submit_truth(&truth_pool, &submission, "gpt-6")
                    .await
                    .unwrap();
                control.signal_llm_result_ready(&truth_run).await.unwrap();
                count += 1;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    let first: ManualLlmJob<EditorialDraftGenerateInputPayload> =
        timeout(Duration::from_secs(180), async {
            loop {
                if let Some(job) = manual_llm_queue::next_editorial(&infra.postgres.pool)
                    .await
                    .unwrap()
                {
                    break job;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("canonical Temporal workflow did not enqueue a manual editorial job");
    assert_eq!(first.payload.run_id, run_id);
    assert_eq!(first.operation, "editorial_draft");
    assert!(
        truth_driver.await.unwrap() > 0,
        "canonical workflow skipped manual source-proposal stage"
    );
    assert_eq!(
        manual_llm_queue::truth_run_status(&infra.postgres.pool, &run_id)
            .await
            .unwrap(),
        "done"
    );
    let truth_results = manual_llm_queue::read_truth_results(&infra.postgres.pool, &run_id)
        .await
        .unwrap();
    assert!(
        truth_results
            .iter()
            .any(|(_, result)| !result.candidates.is_empty()),
        "canonical test must exercise at least one positive manual candidate"
    );
    let pending = manual_llm_queue::read_editorial_result(&infra.postgres.pool, &first.payload)
        .await
        .unwrap();
    assert_eq!(pending.status, "awaiting_manual");

    // No draft can be promoted by merely enqueueing an LLM request.
    let existing_drafts: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_drafts")
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
    assert_eq!(existing_drafts, 0);

    // Temporal history + PostgreSQL queue survive losing the worker process.
    drop(worker);
    let mut worker = spawn_temporal_worker(&worker_env);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        worker.child.try_wait().unwrap().is_none(),
        "restarted canonical worker exited"
    );

    let control = TemporalSeoWorkflowControlAdapter::new(
        infra.temporal.temporal_url.clone(),
        format!("manual-temporal-signaler-{run_id}"),
        infra.temporal.namespace.clone(),
    );
    let make_submission = |job: &ManualLlmJob<EditorialDraftGenerateInputPayload>| {
        let brief = job
            .payload
            .request
            .as_ref()
            .and_then(|request| request.editorial_brief.as_ref())
            .expect("canonical workflow created editorial brief");
        let role = brief
            .section_templates
            .iter()
            .find(|section| section.required)
            .expect("at least one required editorial section")
            .section_role
            .clone();
        let supported_text = brief
            .verified_support
            .first()
            .expect("manual editorial requires verified evidence")
            .fragment_text
            .clone();
        let mut sections = serde_json::Map::new();
        sections.insert(role, serde_json::json!(supported_text));
        let response = serde_json::json!({
            "task_id": job.task_id,
            "schema_version": job.schema_version,
            "input_sha256": job.input_sha256,
            "response": {"sections": sections}
        });
        serde_json::from_value::<ManualLlmSubmission>(response).unwrap()
    };
    let first_submission = make_submission(&first);
    assert_eq!(
        manual_llm_queue::submit_editorial(&infra.postgres.pool, &first_submission, "gpt-6")
            .await
            .unwrap(),
        run_id
    );
    control.signal_llm_result_ready(&run_id).await.unwrap();
    // A duplicate replay signal is harmless and must not grant CMS approval.
    control.signal_llm_result_ready(&run_id).await.unwrap();
    assert_eq!(
        manual_llm_queue::submit_editorial(&infra.postgres.pool, &first_submission, "gpt-6")
            .await
            .unwrap(),
        run_id
    );

    // Some sites contain multiple pages; each page has its own durable
    // manual task, and a response never substitutes for a different page.
    let driver_pool = infra.postgres.pool.clone();
    let driver_run_id = run_id.clone();
    let driver = tokio::spawn(async move {
        let mut submitted = std::collections::BTreeSet::new();
        submitted.insert(first.task_id);
        let control = control;
        loop {
            if let Some(job) = manual_llm_queue::next_editorial(&driver_pool)
                .await
                .unwrap()
            {
                assert_eq!(job.payload.run_id, driver_run_id);
                assert!(submitted.insert(job.task_id.clone()));
                let submission = make_submission(&job);
                manual_llm_queue::submit_editorial(&driver_pool, &submission, "gpt-6")
                    .await
                    .unwrap();
                control
                    .signal_llm_result_ready(&driver_run_id)
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    let result = timeout(
        Duration::from_secs(240),
        handle.get_result(WorkflowGetResultOptions::default()),
    )
    .await;
    driver.abort();
    assert!(
        result.is_ok(),
        "canonical manual editorial workflow timed out after restarting worker"
    );
    result.unwrap().expect("canonical manual workflow failed");
    let drafted: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_drafts")
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
    assert!(
        drafted > 0,
        "manual editorial did not produce a real page draft"
    );
    drop(worker);
}
