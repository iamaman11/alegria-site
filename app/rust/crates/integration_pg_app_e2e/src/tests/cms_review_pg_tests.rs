#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn cms_revision_review_requires_exact_pending_revision_and_active_workflow() {
    use infrastructure::adapters::{
        sqlx_seo_adapter, sqlx_seo_cms_adapter, static_site_builder_adapter,
    };
    use seo_ports::{CmsReviewDecisionRequest, HumanReviewDecision};

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let page = format!("cms-hitl-page-{marker}");
    let blueprint = format!("cms-hitl-blueprint-{marker}");
    let brief = format!("cms-hitl-brief-{marker}");
    let original_draft = format!("cms-hitl-draft-old-{marker}");
    let replacement_draft = format!("cms-hitl-draft-new-{marker}");
    let original = format!("cms-hitl-revision-old-{marker}");
    let replacement = format!("cms-hitl-revision-new-{marker}");
    let scope = format!("cms-hitl-scope-{marker}");
    let run_id = Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) VALUES ($1,'country_hub_page','informational')")
        .bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) VALUES ($1,$2,$3,'country_hub_page','informational','spain','/ru/visa/spain/','published')")
        .bind(&page).bind(&scope).bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'Test brief')")
        .bind(&brief).bind(&page).bind(&blueprint).execute(pool).await.unwrap();
    for (draft, revision, number, status) in [
        (&original_draft, &original, 1, "published"),
        (&replacement_draft, &replacement, 2, "review_required"),
    ] {
        sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown) VALUES ($1,$2,$3,$4,'Source-backed test')")
            .bind(draft).bind(&page).bind(&brief).bind(number).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Spain visa','Spain visa',$5,$6)")
            .bind(revision).bind(&page).bind(draft).bind(&brief).bind(&blueprint).bind(status)
            .execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status,published_at) VALUES ($1,$2,'/ru/visa/spain/','ru','country_hub_page','informational',$3,$4,'published',now())")
        .bind(&page).bind(&scope).bind(format!("cms-doc-{marker}")).bind(&original)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline.execution_runs (run_id,workflow_run_id,workflow_type,context_key) VALUES ($1,'temporal-test-run','seo_site_build','ES|tourist||BY')")
        .bind(Uuid::parse_str(&run_id).unwrap()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_publish_events (event_key,event_type,page_node_key,scope_signature,revision_id,event_payload) VALUES ($1,'seo_page_review_requested',$2,$3,$4,$5)")
        .bind(format!("cms-event-{marker}")).bind(&page).bind(&scope).bind(&replacement)
        .bind(serde_json::json!({"run_id": run_id})).execute(pool).await.unwrap();

    let request = |revision_id: &str| CmsReviewDecisionRequest {
        page_node_key: page.clone(),
        revision_id: revision_id.to_string(),
        actor_role: "real_editor_fixture".to_string(),
        decision: HumanReviewDecision::Approved,
        reason: "Review of exact pending revision".to_string(),
        source: "e2e_revision_bound_human_gate".to_string(),
    };
    assert!(sqlx_seo_cms_adapter::apply_human_review_decision(pool, &request("")).await.is_err());
    assert!(sqlx_seo_cms_adapter::apply_human_review_decision(pool, &request(&original)).await.is_err());
    assert!(sqlx_seo_cms_adapter::apply_human_review_decision(pool, &request("wrong-revision")).await.is_err());

    // A reused immutable candidate may never signal an arbitrarily chosen
    // workflow when two still-active runs claim the same review event.
    let second_run = Uuid::new_v4();
    sqlx::query("INSERT INTO pipeline.execution_runs (run_id,workflow_run_id,workflow_type,context_key) VALUES ($1,'second-temporal-run','seo_site_build','ES|tourist||BY')")
        .bind(second_run).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_publish_events (event_key,event_type,page_node_key,scope_signature,revision_id,event_payload) VALUES ($1,'seo_page_review_requested',$2,$3,$4,$5)")
        .bind(format!("cms-second-event-{marker}")).bind(&page).bind(&scope).bind(&replacement)
        .bind(serde_json::json!({"run_id": second_run.to_string()}))
        .execute(pool).await.unwrap();
    assert!(sqlx_seo_cms_adapter::apply_human_review_decision(pool, &request(&replacement)).await.is_err());
    sqlx::query("UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1")
        .bind(second_run).execute(pool).await.unwrap();

    let result = sqlx_seo_cms_adapter::apply_human_review_decision(pool, &request(&replacement))
        .await.unwrap();
    assert_eq!(result.revision_id, replacement);
    assert_eq!(result.workflow_id, run_id);
    let repository =
        pg_runtime_ports::SqlxSeoPgRepository::new(pool);
    let readback = seo_application::review_publish::load_cms_approval_decision(
        &repository,
        &page,
        &replacement,
    )
    .await
    .unwrap();
    assert_eq!(readback.decision_key, result.decision_key);
    assert!(readback.approves_revision(&page, &replacement));
    assert!(!readback.approves_revision(&page, &original));
    assert_eq!(
        seo_steps::cms_review_wire::workflow_disposition(
            &seo_steps::cms_review_wire::to_wire(readback.clone()),
            true,
        ),
        seo_domain::review_decision::EditorialReviewDisposition::Approved,
    );
    let (live_revision, live_status): (String, String) = sqlx::query_as(
        "SELECT current_revision_id,current_status FROM site.cms_pages WHERE page_node_key=$1",
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!((live_revision, live_status), (original.clone(), "published".to_string()));
    let decided: Vec<String> = sqlx::query_scalar(
        "SELECT revision_id FROM site.cms_approval_decisions WHERE page_node_key=$1",
    ).bind(&page).fetch_all(pool).await.unwrap();
    assert_eq!(decided, vec![replacement.clone()]);

    // A second immutable candidate proves that persisted human blocking is
    // terminal only for newly patched workflow histories. It never replaces
    // the published live pointer or approves either review candidate.
    let blocked_draft = format!("cms-hitl-draft-blocked-{marker}");
    let blocked_revision = format!("cms-hitl-revision-blocked-{marker}");
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown) VALUES ($1,$2,$3,3,'Rejected content')")
        .bind(&blocked_draft).bind(&page).bind(&brief).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Rejected page','Rejected page',$5,'review_required')")
        .bind(&blocked_revision).bind(&page).bind(&blocked_draft).bind(&brief).bind(&blueprint)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_publish_events (event_key,event_type,page_node_key,scope_signature,revision_id,event_payload) VALUES ($1,'seo_page_review_requested',$2,$3,$4,$5)")
        .bind(format!("cms-blocked-event-{marker}")).bind(&page).bind(&scope).bind(&blocked_revision)
        .bind(serde_json::json!({"run_id": run_id})).execute(pool).await.unwrap();
    let mut rejected = request(&blocked_revision);
    rejected.decision = HumanReviewDecision::Blocked;
    sqlx_seo_cms_adapter::apply_human_review_decision(pool, &rejected)
        .await.unwrap();
    let blocked = seo_application::review_publish::load_cms_approval_decision(
        &repository, &page, &blocked_revision,
    ).await.unwrap();
    assert!(!blocked.approves_revision(&page, &blocked_revision));
    let blocked_wire = seo_steps::cms_review_wire::to_wire(blocked);
    assert_eq!(
        seo_steps::cms_review_wire::workflow_disposition(&blocked_wire, true),
        seo_domain::review_decision::EditorialReviewDisposition::ReworkRequired,
    );
    assert_eq!(
        seo_steps::cms_review_wire::workflow_disposition(&blocked_wire, false),
        seo_domain::review_decision::EditorialReviewDisposition::Wait,
    );
    let live_after_block: String = sqlx::query_scalar(
        "SELECT current_revision_id FROM site.cms_pages WHERE page_node_key=$1",
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(live_after_block, original);

    sqlx::query("UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1")
        .bind(Uuid::parse_str(&run_id).unwrap()).execute(pool).await.unwrap();
    let mut after_terminal = request(&replacement);
    after_terminal.actor_role = "other_editor_fixture".to_string();
    assert!(sqlx_seo_cms_adapter::apply_human_review_decision(pool, &after_terminal).await.is_err());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.cms_approval_decisions WHERE page_node_key=$1",
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(count, 2);
}
