// The real canonical workflow must not bypass a pending ChatGPT task.
// Restart the actual Temporal worker while the task is waiting and finish
// through the same Rust PostgreSQL inbox + dedicated Temporal signal.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn canonical_temporal_manual_editorial_resumes_after_worker_restart() {
    run_canonical_manual_editorial("draft_only", false).await;
}

// Unlike the restart smoke above, this runs the *real* publish_with_hitl
// workflow through CMS request_review, the persisted human decision and the
// Temporal resume signal. A blocked exact revision must terminate for rework
// without executing approved publication. The separate PG revision test also
// covers preservation of an existing published live pointer.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn canonical_temporal_blocked_review_ends_for_rework_without_publication() {
    run_canonical_manual_editorial("publish_with_hitl", true).await;
}

#[cfg(feature = "e2e")]
async fn run_canonical_manual_editorial(run_mode: &str, block_review: bool) {
    let started = std::time::Instant::now();
    let checkpoint = |stage: &str| {
        eprintln!(
            "CI_TEMPORAL_MANUAL_STAGE stage={} elapsed_ms={}",
            stage,
            started.elapsed().as_millis()
        );
    };
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
    checkpoint("infra_ready");
    let voyage_stub = stub_servers::spawn_voyage_contract_stub().await.unwrap();
    let serp_stub = stub_servers::spawn_standard_serp_stub(vec![serp_responses[0].1.clone()])
    .await
    .unwrap();
    let truth_stub = stub_servers::spawn_json_sequence_stub(
        "/v1/chat/completions",
        truth_certification_truth_sequence(std::slice::from_ref(&fixture)),
    )
    .await
    .unwrap();
    let editorial_stub = stub_servers::spawn_editorial_fixture_stub(None)
    .await
    .unwrap();
    let mut worker_env = infra.runtime_env();
    worker_env.extend(truth_certification_env(
        &format!(
            "{}/v3/serp/google/organic/task_post",
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
        run_mode.to_string(),
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
    checkpoint("workflow_started");

    // Before editorial generation, prove that the same Temporal run waits
    // for every raw-section truth proposal in the same durable inbox.
    // An explicit empty candidate array is valid but cannot create verified truth.
    // Drive the manual path with the same evidence-bearing proposals as the
    // certified happy path, not a passport-only subset that can leave fee/time
    // unresolved and correctly block the subsequent editorial phase.
    let truth_fixture_candidates = fixture.model_stub_inputs.extraction_responses[0]
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .expect("certified happy path provides direct source proposals")
        .clone();
    assert_eq!(truth_fixture_candidates.len(), 3);
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
                // Re-anchor each certified proposal to this *actual* raw
                // section. Missing source quotes cannot become candidates.
                // The normal manual inbox and canonical validator still
                // verify each byte span and all candidate fields.
                let raw_text = job.payload["raw_text"].as_str().unwrap();
                let section_id = job.payload["raw_section_id"].as_i64().unwrap();
                let candidates = truth_fixture_candidates
                    .iter()
                    .filter_map(|fixture_candidate| {
                        let quote = fixture_candidate.get("evidence_quote")?.as_str()?;
                        let start = raw_text.find(quote)?;
                        let mut candidate = fixture_candidate.clone();
                        candidate["evidence_section_id"] = serde_json::json!(section_id);
                        candidate["span_start"] = serde_json::json!(start);
                        candidate["span_end"] = serde_json::json!(start + quote.len());
                        Some(candidate)
                    })
                    .collect::<Vec<_>>();
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
        match timeout(Duration::from_secs(180), async {
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
        {
            Ok(job) => job,
            Err(_) => {
                // Report the *actual* failure boundary rather than treating every
                // missing editorial task as a slow LLM or raising the timeout.
                let steps = certification_failure_diagnostics(&infra.postgres.pool, &run_id).await;
                let verified_facts: Result<i64, sqlx::Error> = sqlx::query_scalar(
                    "SELECT count(*)::bigint FROM verified.rule_instances
                     WHERE context_key = $1 AND status = 'verified'",
                )
                .bind(&input.context_key)
                .fetch_one(&infra.postgres.pool)
                .await;
                let verified_vectors: Result<i64, sqlx::Error> = sqlx::query_scalar(
                    "SELECT count(*)::bigint FROM kb.qdrant_points
                     WHERE collection_name = 'verified_rules_4'",
                )
                .fetch_one(&infra.postgres.pool)
                .await;
                let jobs = sqlx::query(
                    "SELECT operation, job_status, count(*)::bigint AS total
                     FROM pipeline.manual_llm_jobs
                     WHERE run_id = $1
                     GROUP BY operation, job_status
                     ORDER BY operation, job_status",
                )
                .bind(&run_id)
                .fetch_all(&infra.postgres.pool)
                .await
                .map(|rows| {
                    rows.iter()
                        .map(|row| format!(
                            "{}:{}:{}",
                            row.get::<String, _>("operation"),
                            row.get::<String, _>("job_status"),
                            row.get::<i64, _>("total")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                let run_status: Result<Option<String>, sqlx::Error> =
                    sqlx::query_scalar("SELECT status FROM pipeline.execution_runs WHERE run_id = $1")
                        .bind(Uuid::parse_str(&run_id).expect("registered run is a UUID"))
                        .fetch_optional(&infra.postgres.pool)
                        .await;
                let terminal = timeout(
                    Duration::from_secs(2),
                    handle.get_result(WorkflowGetResultOptions::default()),
                )
                .await;
                let temporal = match terminal {
                    Ok(Ok(_)) => "completed_without_editorial".to_string(),
                    Ok(Err(error)) => format!("failed: {error}"),
                    Err(_) => "not_terminal".to_string(),
                };
                panic!(
                    "canonical Temporal workflow did not enqueue a manual editorial job: \
                     run_id={run_id}; execution_run={run_status:?}; \
                     workflow={temporal}; truth_driver_finished={}; \
                     verified_facts={verified_facts:?}; verified_vectors={verified_vectors:?}; \
                     manual_jobs={jobs:?}; last_steps=\n{steps}",
                    truth_driver.is_finished(),
                );
            }
        };
    assert_eq!(first.payload.run_id, run_id);
    checkpoint("editorial_task_ready");
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
    checkpoint("worker_restarted");

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
        assert_eq!(job.payload.request.as_ref().unwrap().output_contract, "llm_draft_candidate@2");
        let allowed = infrastructure::adapters::editorial_llm_adapter::editorial_allowed_support(brief);
        let editorial = stub_servers::editorial_fixture_claims(&serde_json::to_value(allowed).unwrap()).unwrap();
        let response = serde_json::json!({
            "task_id": job.task_id,
            "schema_version": job.schema_version,
            "input_sha256": job.input_sha256,
            "response": editorial
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
    if block_review {
        use infrastructure::adapters::{
            seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
            temporalio_sdk_adapter::{
                temporalio_common::data_converters::PayloadConverter, UntypedQuery,
                WorkflowQueryOptions,
            },
        };
        use seo_application::cms_review::{
            apply_human_review_decision, ApplyHumanReviewDecisionInput,
        };
        use seo_ports::HumanReviewDecision;

        let (page_node_key, revision_id): (String, String) =
            timeout(Duration::from_secs(180), async {
                loop {
                    if let Some(row) = sqlx::query_as::<_, (String, String)>(
                        "SELECT page_node_key, revision_id
                         FROM site.cms_publish_events
                         WHERE event_type='seo_page_review_requested'
                           AND event_payload->>'run_id'=$1
                         ORDER BY occurred_at LIMIT 1",
                    )
                    .bind(&run_id)
                    .fetch_optional(&infra.postgres.pool)
                    .await
                    .unwrap()
                    {
                        break row;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!("publish_with_hitl did not request an exact CMS review: {run_id}")
            });
        checkpoint("cms_review_requested");

        // The request event may be committed before the workflow has entered
        // its durable signal wait. Query the actual Temporal workflow to avoid
        // an early resume signal being ignored by waiting_hitl=false.
        let converter = PayloadConverter::serde_json();
        timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(reply) = handle
                    .query(
                        UntypedQuery::new("status"),
                        RawValue::default(),
                        WorkflowQueryOptions::default(),
                    )
                    .await
                {
                    let state: serde_json::Value = reply.to_value(&converter);
                    if state["paused"] == true
                        && state["phase"]
                            .as_str()
                            .is_some_and(|phase| phase.starts_with("human_approval_wait"))
                    {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("canonical workflow never entered the durable human-review wait");
        checkpoint("temporal_hitl_waiting");

        let repo = SqlxSeoRuntimeRepository::new(&infra.postgres.pool);
        let control = TemporalSeoWorkflowControlAdapter::new(
            infra.temporal.temporal_url.clone(),
            format!("blocked-review-signal-{run_id}"),
            infra.temporal.namespace.clone(),
        );
        let outcome = apply_human_review_decision(
            &repo,
            &control,
            &ApplyHumanReviewDecisionInput {
                page_node_key: page_node_key.clone(),
                revision_id: Some(revision_id.clone()),
                actor_role: "editorial_e2e_reviewer".to_string(),
                decision: HumanReviewDecision::Blocked,
                reason: "Exact revision requires rewriting".to_string(),
                source: "canonical_temporal_publish_mode_e2e".to_string(),
            },
        )
        .await
        .expect("real CMS human-review transaction and Temporal resume must succeed");
        assert_eq!(outcome.workflow_id, run_id);
        assert_eq!(outcome.revision_id, revision_id);

        let completed = timeout(
            Duration::from_secs(120),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("blocked review did not terminate for editorial rework")
            .expect("canonical Temporal workflow failed after human block");
        let result: String = completed.to_value(&converter);
        assert!(
            result.contains("blocked:editorial_rework_required"),
            "{result}"
        );
        assert!(result.contains("published_pages=0"), "{result}");
        checkpoint("blocked_rework_completed");

        let saved_decision: String = sqlx::query_scalar(
            "SELECT decision FROM site.cms_approval_decisions WHERE page_node_key=$1 AND revision_id=$2",
        )
        .bind(&page_node_key)
        .bind(&revision_id)
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
        assert_eq!(saved_decision, "blocked");
        let (current_revision, current_status): (String, String) = sqlx::query_as(
            "SELECT current_revision_id, current_status FROM site.cms_pages WHERE page_node_key=$1",
        )
        .bind(&page_node_key)
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
        assert_eq!(current_revision, revision_id);
        assert_eq!(
            current_status, "review_required",
            "human block promoted the CMS revision"
        );
        let published_events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM site.cms_publish_events WHERE event_type='seo_page_published' AND event_payload->>'run_id'=$1",
        )
        .bind(&run_id)
        .fetch_one(&infra.postgres.pool)
        .await
        .unwrap();
        assert_eq!(published_events, 0, "a blocked review was published");
        driver.abort();
        drop(worker);
        return;
    }

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
    checkpoint("workflow_completed");
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
