#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn draft_qa_persistence_rolls_back_late_failure_and_rejects_missing_draft() {
    use contracts::generated::alegria::temporal::v1::{
        ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
        DraftAssembleOutputPayload, DraftQaInputPayload, DraftQaOutputPayload, DraftState,
        PageBriefState,
    };
    use infrastructure::adapters::sqlx_seo_adapter::{
        persist_content_contract_validate_output, persist_draft_assemble_output,
        persist_draft_qa_output, persist_rebuild_detect_output,
    };
    use seo_domain::rebuild::{RebuildDetectRequest, RebuildDetectResult, RebuildVerdict};

    let pg = PostgresHarness::start().await.unwrap();
    let key = Uuid::new_v4().simple().to_string();
    let scope = format!("qa-atomic-scope-{key}");
    let page = format!("qa-atomic-page-{key}");
    let blueprint = format!("qa-atomic-blueprint-{key}");
    let brief = format!("qa-atomic-brief-{key}");
    let draft_key = format!("qa-atomic-draft-{key}");
    let run_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO site.site_scopes (scope_signature,locale) VALUES ($1,'ru')")
        .bind(&scope)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) VALUES ($1,'country_hub_page','informational')")
        .bind(&blueprint)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) VALUES ($1,$2,$3,'country_hub_page','informational','draft','/qa-atomic/','draft_ready')")
        .bind(&page)
        .bind(&scope)
        .bind(&blueprint)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'QA test')")
        .bind(&brief)
        .bind(&page)
        .bind(&blueprint)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,body_markdown) VALUES ($1,$2,$3,'Verified body')")
        .bind(&draft_key)
        .bind(&page)
        .bind(&brief)
        .execute(&pg.pool)
        .await
        .unwrap();

    let draft = DraftState {
        page_draft_key: draft_key.clone(),
        page_node_key: page.clone(),
        page_brief_key: brief,
        body_markdown: "Verified body".into(),
        ..DraftState::default()
    };
    let input = DraftQaInputPayload {
        run_id: run_id.clone(),
        draft: Some(draft.clone()),
        ..DraftQaInputPayload::default()
    };
    let publish_ready = DraftQaOutputPayload {
        verdict: "publish_ready".into(),
        required_next_action: "request_human_review".into(),
        ..DraftQaOutputPayload::default()
    };

    let mut missing = input.clone();
    missing.draft.as_mut().unwrap().page_draft_key = format!("missing-{draft_key}");
    let error = persist_draft_qa_output(&pg.pool, &missing, &publish_ready)
        .await
        .unwrap_err();
    assert_eq!(error.class_str(), "validation_failure");

    // Induce failure after the page_drafts UPDATE, at the lifecycle audit.
    // PostgreSQL must roll back the updated verdict and all other QA effects.
    sqlx::query("CREATE FUNCTION public.qa_atomicity_fail_lifecycle() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected QA lifecycle failure'; END $$")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER qa_atomicity_fail_lifecycle BEFORE INSERT ON site.page_lifecycle_events FOR EACH ROW EXECUTE FUNCTION public.qa_atomicity_fail_lifecycle()")
        .execute(&pg.pool)
        .await
        .unwrap();
    assert!(persist_draft_qa_output(&pg.pool, &input, &publish_ready)
        .await
        .is_err());

    let saved: (String, String) = sqlx::query_as("SELECT d.qa_verdict,p.lifecycle_state FROM site.page_drafts d JOIN site.page_nodes p ON p.page_node_key=d.page_node_key WHERE d.page_draft_key=$1")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(saved, ("not_run".into(), "draft_ready".into()));
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.page_lifecycle_events WHERE page_node_key=$1",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(events, 0);
    let outbox: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(outbox, 0);

    sqlx::query("DROP TRIGGER qa_atomicity_fail_lifecycle ON site.page_lifecycle_events")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.qa_atomicity_fail_lifecycle()")
        .execute(&pg.pool)
        .await
        .unwrap();
    persist_draft_qa_output(&pg.pool, &input, &publish_ready)
        .await
        .unwrap();
    persist_draft_qa_output(&pg.pool, &input, &publish_ready)
        .await
        .unwrap();
    let one_logical_qa: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(one_logical_qa, 1, "same QA verdict must not emit duplicate outbox events");
    let mut failed = publish_ready;
    failed.verdict = "qa_failed".into();
    failed.blocking_reasons = vec!["missing_claim_ledger".into()];
    failed.required_next_action = "resolve_blockers_and_rerun_draft_qa".into();
    persist_draft_qa_output(&pg.pool, &input, &failed)
        .await
        .unwrap();

    let saved: (String, String) = sqlx::query_as("SELECT d.qa_verdict,p.lifecycle_state FROM site.page_drafts d JOIN site.page_nodes p ON p.page_node_key=d.page_node_key WHERE d.page_draft_key=$1")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(saved, ("qa_failed".into(), "review_required".into()));
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.page_lifecycle_events WHERE page_node_key=$1 AND reason='draft_qa'",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1, "repeated review_required state is a lifecycle no-op");
    let open_failures: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM monitoring.seo_quality_failures WHERE page_draft_key=$1 AND status='open'")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(open_failures, 1);
    let two_verdicts: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'")
        .bind(&draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(two_verdicts, 2, "changed QA verdict must produce a distinct projection event");

    // Content Contract runs before Draft QA and still has a replayable runtime
    // result. It must NOT be a second writer of the durable QA decision. A
    // repeated contract validation after QA cannot overwrite QA blockers.
    sqlx::query("INSERT INTO pipeline.execution_runs (run_id,workflow_run_id,workflow_type,context_key) VALUES ($1,$2,'seo_site_build',$3)")
        .bind(Uuid::parse_str(&run_id).unwrap())
        .bind(format!("qa-atomic-workflow-{key}"))
        .bind(&page)
        .execute(&pg.pool)
        .await
        .unwrap();
    let qa_blockers_before: serde_json::Value =
        sqlx::query_scalar("SELECT qa_blockers FROM site.page_drafts WHERE page_draft_key=$1")
            .bind(&draft_key)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    persist_content_contract_validate_output(
        &pg.pool,
        &ContentContractValidateInputPayload {
            run_id: run_id.clone(),
            draft: Some(draft.clone()),
            ..ContentContractValidateInputPayload::default()
        },
        &ContentContractValidateOutputPayload {
            verdict: "blocked".into(),
            blocking_reasons: vec!["missing_rendered_content_blocks".into()],
            ..ContentContractValidateOutputPayload::default()
        },
    )
    .await
    .unwrap();
    let qa_blockers_after: serde_json::Value =
        sqlx::query_scalar("SELECT qa_blockers FROM site.page_drafts WHERE page_draft_key=$1")
            .bind(&draft_key)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(qa_blockers_after, qa_blockers_before);
    let contract_blobs: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM pipeline.execution_run_blobs WHERE run_id=$1 AND field_name='content_contract_validation'")
        .bind(Uuid::parse_str(&run_id).unwrap())
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(contract_blobs, 1, "the intermediate contract result remains durable");

    // A live release stays live while a newer draft is generated or re-QA'd.
    let live_revision = format!("qa-atomic-live-revision-{key}");
    sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Live page','Live page',$5,'published')")
        .bind(&live_revision)
        .bind(&page)
        .bind(&draft_key)
        .bind(&draft.page_brief_key)
        .bind(&blueprint)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status,published_at) VALUES ($1,$2,'/qa-atomic/','ru','country_hub_page','informational',$3,$4,'published',now())")
        .bind(&page)
        .bind(&scope)
        .bind(format!("qa-atomic-document-{key}"))
        .bind(&live_revision)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE site.page_nodes SET lifecycle_state='published' WHERE page_node_key=$1")
        .bind(&page)
        .execute(&pg.pool)
        .await
        .unwrap();

    persist_draft_qa_output(&pg.pool, &input, &failed)
        .await
        .unwrap();
    let next_draft = DraftState {
        page_draft_key: format!("qa-atomic-next-draft-{key}"),
        draft_revision: 2,
        qa_verdict: "not_run".into(),
        ..draft.clone()
    };
    let next_draft_key = next_draft.page_draft_key.clone();
    let next_output = DraftAssembleOutputPayload {
        page_brief: Some(PageBriefState {
            page_brief_key: draft.page_brief_key.clone(),
            page_node_key: page.clone(),
            blueprint_key: blueprint.clone(),
            title: "Updated brief title".into(),
            brief_version: 1,
            status: "ready".into(),
            ..PageBriefState::default()
        }),
        draft: Some(next_draft),
        ..DraftAssembleOutputPayload::default()
    };
    // The Outbox is the LAST write in Draft Assemble. Inject that failure and
    // verify both the new draft and a preceding brief update roll back.
    sqlx::query("CREATE FUNCTION public.atomic_draft_outbox_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type='QdrantUpsertCommand' AND NEW.aggregate_key LIKE 'qa-atomic-next-draft-%' THEN RAISE EXCEPTION 'injected draft outbox failure'; END IF; RETURN NEW; END $$")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER atomic_draft_outbox_fail BEFORE INSERT ON system.sync_outbox FOR EACH ROW EXECUTE FUNCTION public.atomic_draft_outbox_fail()")
        .execute(&pg.pool)
        .await
        .unwrap();
    assert!(persist_draft_assemble_output(&pg.pool, &run_id, &next_output)
        .await
        .is_err());
    let unchanged_brief: String =
        sqlx::query_scalar("SELECT title FROM site.page_briefs WHERE page_brief_key=$1")
            .bind(&draft.page_brief_key)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(unchanged_brief, "QA test");
    let assembled_drafts: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_drafts WHERE page_draft_key=$1")
            .bind(&next_draft_key)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(assembled_drafts, 0);
    let assembled_events: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'")
        .bind(&next_draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(assembled_events, 0, "draft and projection must roll back together");
    sqlx::query("DROP TRIGGER atomic_draft_outbox_fail ON system.sync_outbox")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.atomic_draft_outbox_fail()")
        .execute(&pg.pool)
        .await
        .unwrap();
    persist_draft_assemble_output(
        &pg.pool,
        &run_id,
        &next_output,
    )
    .await
    .unwrap();
    persist_draft_assemble_output(&pg.pool, &run_id, &next_output)
        .await
        .unwrap();
    let once: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'")
        .bind(&next_draft_key)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(once, 1, "replaying the same draft must not duplicate Qdrant projection");
    let live: (String, String, String) = sqlx::query_as("SELECT n.lifecycle_state,p.current_status,p.current_revision_id FROM site.page_nodes n JOIN site.cms_pages p ON p.page_node_key=n.page_node_key WHERE n.page_node_key=$1")
        .bind(&page)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(live, ("published".into(), "published".into(), live_revision));
    let lifecycle_events: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_lifecycle_events WHERE page_node_key=$1")
        .bind(&page)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(lifecycle_events, 1, "published re-QA/assembly must not invent lifecycle transitions");

    let rebuild_input = RebuildDetectRequest {
        run_id: run_id.clone(),
        changed_truth_keys: vec![format!("truth-rebuild-{key}")],
        page_node_keys: vec![page.clone()],
    };
    let rebuild_output = RebuildDetectResult {
        impacted_page_node_keys: vec![page.clone()],
        verdict: RebuildVerdict::RebuildRequired,
        impacts: vec![],
    };
    // Inject a late Outbox failure after Rebuild has written its queue, plan
    // and lifecycle. Nothing must become visible outside the transaction.
    sqlx::query("CREATE FUNCTION public.atomic_rebuild_outbox_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type='SeoGraphProjectionUpserted' AND NEW.aggregate_key LIKE 'qa-atomic-page-%' THEN RAISE EXCEPTION 'injected rebuild outbox failure'; END IF; RETURN NEW; END $$")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER atomic_rebuild_outbox_fail BEFORE INSERT ON system.sync_outbox FOR EACH ROW EXECUTE FUNCTION public.atomic_rebuild_outbox_fail()")
        .execute(&pg.pool)
        .await
        .unwrap();
    assert!(persist_rebuild_detect_output(&pg.pool, &rebuild_input, &rebuild_output)
        .await
        .is_err());
    let rolled_back: (String, i64, i64) = sqlx::query_as(
        "SELECT p.lifecycle_state,
           (SELECT count(*)::bigint FROM monitoring.seo_rebuild_backlog WHERE page_node_key=p.page_node_key),
           (SELECT count(*)::bigint FROM site.global_rebuild_plan WHERE affected_page_node_key=p.page_node_key)
         FROM site.page_nodes p WHERE p.page_node_key=$1",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(rolled_back, ("published".into(), 0, 0));
    sqlx::query("DROP TRIGGER atomic_rebuild_outbox_fail ON system.sync_outbox")
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.atomic_rebuild_outbox_fail()")
        .execute(&pg.pool)
        .await
        .unwrap();
    persist_rebuild_detect_output(&pg.pool, &rebuild_input, &rebuild_output)
        .await
        .unwrap();
    let queued: (String, i64, i64) = sqlx::query_as(
        "SELECT p.lifecycle_state,
           (SELECT count(*)::bigint FROM monitoring.seo_rebuild_backlog WHERE page_node_key=p.page_node_key AND status='queued'),
           (SELECT count(*)::bigint FROM site.global_rebuild_plan WHERE affected_page_node_key=p.page_node_key AND status='queued')
         FROM site.page_nodes p WHERE p.page_node_key=$1",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(queued, ("needs_rebuild".into(), 1, 1));
    sqlx::query("UPDATE monitoring.seo_rebuild_backlog SET status='done' WHERE page_node_key=$1")
        .bind(&page)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE site.global_rebuild_plan SET status='done' WHERE affected_page_node_key=$1")
        .bind(&page)
        .execute(&pg.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE site.page_nodes SET lifecycle_state='published' WHERE page_node_key=$1")
        .bind(&page)
        .execute(&pg.pool)
        .await
        .unwrap();
    // Replay of one logical change is a no-op; another run on the same truth
    // key must create fresh queue work and a fresh projection event.
    persist_rebuild_detect_output(&pg.pool, &rebuild_input, &rebuild_output)
        .await
        .unwrap();
    let replayed: (String, i64, i64) = sqlx::query_as(
        "SELECT p.lifecycle_state,
           (SELECT count(*)::bigint FROM monitoring.seo_rebuild_backlog WHERE page_node_key=p.page_node_key AND status='done'),
           (SELECT count(*)::bigint FROM site.global_rebuild_plan WHERE affected_page_node_key=p.page_node_key AND status='done')
         FROM site.page_nodes p WHERE p.page_node_key=$1",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(replayed, ("published".into(), 1, 1));
    let next_change = RebuildDetectRequest {
        run_id: Uuid::new_v4().to_string(),
        ..rebuild_input
    };
    persist_rebuild_detect_output(&pg.pool, &next_change, &rebuild_output)
        .await
        .unwrap();
    let newer: (String, i64, i64, i64) = sqlx::query_as(
        "SELECT p.lifecycle_state,
           (SELECT count(*)::bigint FROM monitoring.seo_rebuild_backlog WHERE page_node_key=p.page_node_key AND status='queued'),
           (SELECT count(*)::bigint FROM monitoring.seo_rebuild_backlog WHERE page_node_key=p.page_node_key AND status='done'),
           (SELECT count(*)::bigint FROM site.global_rebuild_plan WHERE affected_page_node_key=p.page_node_key AND status='queued')
         FROM site.page_nodes p WHERE p.page_node_key=$1",
    )
    .bind(&page)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(newer, ("needs_rebuild".into(), 1, 1, 1));
    let projections: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='SeoGraphProjectionUpserted'")
        .bind(&page)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(projections, 2, "one projection for each distinct rebuild change");
}
