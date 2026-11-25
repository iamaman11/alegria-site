#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn cms_revision_publish_keeps_verified_links_structured_without_duplicate_body_sections() {
    use contracts::generated::alegria::temporal::v1::{
        CmsPublishInputPayload, CmsPublishOutputPayload,
    };
    use infrastructure::adapters::{
        sqlx_seo_adapter, sqlx_seo_cms_adapter, static_site_builder_adapter,
    };
    use seo_ports::{CmsReviewDecisionRequest, HumanReviewDecision};

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let scope = format!("cms-link-scope-{marker}");
    let blueprint = format!("cms-link-blueprint-{marker}");
    let parent_page = format!("cms-link-parent-{marker}");
    let parent_brief = format!("cms-link-parent-brief-{marker}");
    let parent_draft = format!("cms-link-parent-draft-{marker}");
    let parent_revision = format!("cms-link-parent-revision-{marker}");
    let tourist_page = format!("cms-link-tourist-{marker}");
    let tourist_brief = format!("cms-link-tourist-brief-{marker}");
    let tourist_draft = format!("cms-link-tourist-draft-{marker}");
    let parent_path = format!("/ru/visa/{marker}/");
    let tourist_path = format!("/ru/visa/{marker}/tourist/");

    sqlx::query("INSERT INTO site.site_scopes (scope_signature,locale) VALUES ($1,'ru')")
        .bind(&scope).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) VALUES ($1,'country_hub_page','informational')")
        .bind(&blueprint).execute(pool).await.unwrap();
    for (page, path, slug) in [
        (&parent_page, &parent_path, "parent"),
        (&tourist_page, &tourist_path, "tourist"),
    ] {
        sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path) VALUES ($1,$2,$3,'country_hub_page','informational',$4,$5)")
            .bind(page).bind(&scope).bind(&blueprint).bind(slug).bind(path)
            .execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'Parent page')")
        .bind(&parent_brief).bind(&parent_page).bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,body_markdown) VALUES ($1,$2,$3,'Parent text')")
        .bind(&parent_draft).bind(&parent_page).bind(&parent_brief).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Виза в Испанию','Виза в Испанию',$5,'published')")
        .bind(&parent_revision).bind(&parent_page).bind(&parent_draft).bind(&parent_brief)
        .bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status,published_at) VALUES ($1,$2,$3,'ru','country_hub_page','informational',$4,$5,'published',now())")
        .bind(&parent_page).bind(&scope).bind(&parent_path)
        .bind(format!("cms-doc-{marker}")).bind(&parent_revision)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'Tourist page')")
        .bind(&tourist_brief).bind(&tourist_page).bind(&blueprint).execute(pool).await.unwrap();
    let manifest = serde_json::json!({
        "content_blocks": [
            {"block_type":"intro","section_role":"intro","markdown":"Текст туристической страницы"},
            {"block_type":"related_pages","section_role":"related_pages", "heading":"Связанные страницы",
             "markdown":format!("- Recommended internal link: hub -> {parent_page}")}
        ],
        "claim_ledger": []
    });
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,body_markdown,qa_verdict,truth_snapshot_ref,traceability_manifest) VALUES ($1,$2,$3,'Текст туристической страницы','publish_ready','verified-truth',$4)")
        .bind(&tourist_draft).bind(&tourist_page).bind(&tourist_brief).bind(manifest)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.link_recommendations (link_recommendation_key,scope_signature,source_page_key,target_page_key,required_flag,link_role,score) VALUES ($1,$2,$3,$4,true,'contextual',0.9)")
        .bind(format!("cms-link-{marker}")).bind(&scope).bind(&tourist_page)
        .bind(&parent_page).execute(pool).await.unwrap();
    // Planning creates required candidate edges to child pages which do not
    // have CMS revisions yet. They are not accepted publication obligations.
    let future_child = format!("cms-link-unpublished-child-{marker}");
    let future_path = format!("{tourist_path}documents/");
    sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path) VALUES ($1,$2,$3,'country_hub_page','informational','documents',$4)")
        .bind(&future_child).bind(&scope).bind(&blueprint).bind(&future_path)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.link_recommendations (link_recommendation_key,scope_signature,source_page_key,target_page_key,required_flag,link_role,score) VALUES ($1,$2,$3,$4,true,'hub_to_child',0.8)")
        .bind(format!("cms-link-future-{marker}")).bind(&scope).bind(&tourist_page)
        .bind(&future_child).execute(pool).await.unwrap();

    let input = CmsPublishInputPayload {
        run_id: Uuid::new_v4().to_string(),
        publish_mode: "request_review".into(),
        ..Default::default()
    };
    sqlx::query("INSERT INTO pipeline.execution_runs (run_id,workflow_run_id,workflow_type,context_key) VALUES ($1,'cms-revision-test-workflow','seo_site_build','ES|tourist||BY')")
        .bind(Uuid::parse_str(&input.run_id).unwrap()).execute(pool).await.unwrap();
    let planned = CmsPublishOutputPayload {
        page_draft_key: tourist_draft,
        ..Default::default()
    };
    // Fail at the *last* write, after the revision, page, lifecycle and CMS
    // event have already been attempted. No partial publish may survive.
    let original_lifecycle: String = sqlx::query_scalar(
        "SELECT lifecycle_state FROM site.page_nodes WHERE page_node_key=$1",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "ALTER TABLE system.sync_outbox ADD CONSTRAINT cms_atomic_review_fault \
         CHECK (event_type <> 'seo_page_review_requested')",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(sqlx_seo_cms_adapter::persist_cms_publish_output(pool, &input, &planned)
        .await
        .is_err());
    let surviving_revision: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.cms_page_revisions WHERE page_draft_key=$1",
    )
    .bind(&planned.page_draft_key)
    .fetch_one(pool)
    .await
    .unwrap();
    let surviving_page: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.cms_pages WHERE page_node_key=$1",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    let surviving_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.cms_publish_events WHERE page_node_key=$1",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    let lifecycle_after_failure: String = sqlx::query_scalar(
        "SELECT lifecycle_state FROM site.page_nodes WHERE page_node_key=$1",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((surviving_revision, surviving_page, surviving_event), (0, 0, 0));
    assert_eq!(lifecycle_after_failure, original_lifecycle);
    sqlx::query("ALTER TABLE system.sync_outbox DROP CONSTRAINT cms_atomic_review_fault")
        .execute(pool)
        .await
        .unwrap();

    let result = sqlx_seo_cms_adapter::persist_cms_publish_output(pool, &input, &planned)
        .await.unwrap();
    assert_eq!(result.verdict, "review_requested");
    assert!(result.blocking_reasons.is_empty());
    let (review_status, page_status, page_lifecycle): (String, String, String) = sqlx::query_as(
        "SELECT r.revision_status,p.current_status,n.lifecycle_state FROM site.cms_page_revisions r JOIN site.cms_pages p ON p.page_node_key=r.page_node_key JOIN site.page_nodes n ON n.page_node_key=r.page_node_key WHERE r.revision_id=$1",
    ).bind(&result.revision_id).fetch_one(pool).await.unwrap();
    assert_eq!((review_status.as_str(), page_status.as_str(), page_lifecycle.as_str()),
        ("review_required", "review_required", "review_required"));

    let reviewer = CmsReviewDecisionRequest {
        page_node_key: tourist_page.clone(),
        revision_id: result.revision_id.clone(),
        actor_role: "editor_fixture".to_string(),
        decision: HumanReviewDecision::Approved,
        reason: "Exact revision approved".to_string(),
        source: "cms_revision_transition_e2e".to_string(),
    };
    sqlx_seo_cms_adapter::apply_human_review_decision(pool, &reviewer)
        .await
        .unwrap();
    let persisted = sqlx_seo_cms_adapter::load_latest_approval_decision(
        pool,
        &tourist_page,
        &result.revision_id,
    )
    .await
    .unwrap()
    .expect("exact approved revision");
    let approved_input = CmsPublishInputPayload {
        publish_mode: "approved_publish".to_string(),
        approval_decision: Some(seo_steps::cms_review_wire::to_wire(persisted)),
        ..input.clone()
    };
    // This time fail after the Outbox insert but before committing the
    // materialization artifact. The entire approval must roll back.
    sqlx::query(
        "ALTER TABLE site.publish_artifacts ADD CONSTRAINT cms_atomic_artifact_fault \
         CHECK (status <> 'pending_materialization')",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &approved_input,
        &CmsPublishOutputPayload {
            page_draft_key: result.page_draft_key.clone(),
            revision_id: result.revision_id.clone(),
            ..Default::default()
        },
    )
    .await
    .is_err());
    let still_review_required: String = sqlx::query_scalar(
        "SELECT revision_status FROM site.cms_page_revisions WHERE revision_id=$1",
    )
    .bind(&result.revision_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let approved_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.cms_publish_events \
         WHERE page_node_key=$1 AND event_type='seo_page_approved'",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    let approved_outbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND event_type='seo_page_approved'",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    let artifacts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.publish_artifacts WHERE page_node_key=$1",
    )
    .bind(&tourist_page)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(still_review_required, "review_required");
    assert_eq!((approved_events, approved_outbox, artifacts), (0, 0, 0));
    sqlx::query("ALTER TABLE site.publish_artifacts DROP CONSTRAINT cms_atomic_artifact_fault")
        .execute(pool)
        .await
        .unwrap();

    let approved = sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &approved_input,
        &CmsPublishOutputPayload {
            page_draft_key: result.page_draft_key.clone(),
            revision_id: result.revision_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(approved.verdict, "approved");
    let replay = sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &approved_input,
        &CmsPublishOutputPayload {
            page_draft_key: result.page_draft_key.clone(),
            revision_id: result.revision_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(replay.outbox_emitted, 0, "same approval must not duplicate Outbox");
    let changed_run = CmsPublishInputPayload {
        run_id: Uuid::new_v4().to_string(),
        ..approved_input.clone()
    };
    assert!(sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &changed_run,
        &CmsPublishOutputPayload {
            page_draft_key: result.page_draft_key.clone(),
            revision_id: result.revision_id.clone(),
            ..Default::default()
        },
    )
    .await
    .is_err(), "one immutable event key cannot acquire another run_id");
    let durable_run_id: String = sqlx::query_scalar(
        "SELECT event_payload->>'run_id' FROM site.cms_publish_events \
         WHERE page_node_key=$1 AND revision_id=$2 AND event_type='seo_page_approved'",
    )
    .bind(&tourist_page)
    .bind(&result.revision_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(durable_run_id, input.run_id);
    let (revision_status, page_status): (String, String) = sqlx::query_as(
        "SELECT r.revision_status,p.current_status FROM site.cms_page_revisions r JOIN site.cms_pages p ON p.page_node_key=r.page_node_key WHERE r.revision_id=$1",
    ).bind(&approved.revision_id).fetch_one(pool).await.unwrap();
    assert_eq!((revision_status.as_str(), page_status.as_str()), ("approved", "approved"));
    let (body, links): (serde_json::Value, serde_json::Value) = sqlx::query_as(
        "SELECT body_payload,required_link_payload FROM site.cms_page_revisions WHERE revision_id=$1",
    ).bind(&result.revision_id).fetch_one(pool).await.unwrap();
    // The CMS stores authored content only. The static renderer owns the
    // related-pages section, so links must not be injected twice.
    assert_eq!(body["markdown"], "Текст туристической страницы");
    let blocks = body["content_blocks"].as_array().unwrap();
    assert!(blocks.iter().all(|block| block["section_role"] != "internal_links"));
    assert_eq!(links.as_array().unwrap().len(), 1);
    assert_eq!(links[0]["target_page_key"], parent_page);
    assert_eq!(links[0]["canonical_url_path"], parent_path);

    // Public HTML is built from the EXACT approved CMS revision and scoped
    // persisted navigation. The planner's mutable link graph must not append
    // a second related-pages section or expose internal target page keys.
    sqlx::query("UPDATE site.site_scopes SET market='global' WHERE scope_signature=$1")
        .bind(&scope)
        .execute(pool)
        .await
        .unwrap();
    sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, "global", "ru")
        .await
        .unwrap();
    let static_dir = std::env::temp_dir().join(format!("cms-approved-static-{marker}"));
    let preview = static_site_builder_adapter::build_static_site_candidate_incremental(
        pool,
        &static_dir,
        "https://example.test",
        &tourist_page,
        &approved.revision_id,
    )
    .await
    .unwrap();
    let public_html = &preview.previews[0].rendered_html;
    // Verify the generated release *on disk*. A preview-only assertion could
    // pass even if the candidate directory omitted its public HTML files.
    let published_child_html = std::fs::read_to_string(
        static_dir.join(tourist_path.trim_matches('/')).join("index.html"),
    )
    .unwrap();
    assert_eq!(&published_child_html, public_html);
    assert!(
        static_dir.join(parent_path.trim_matches('/')).join("index.html").is_file(),
        "candidate release omitted its already published dependency"
    );
    let release_manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(static_dir.join("alegria-static-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(release_manifest["page_count"], 2);
    assert!(release_manifest["pages"].as_array().unwrap().iter().any(|page| {
        page["page_node_key"] == tourist_page && page["revision_id"] == approved.revision_id
    }));
    assert_eq!(public_html.matches("<section class=\"related\">").count(), 1);
    assert!(public_html.contains(&format!(
        "href=\"{}\"",
        parent_path.trim_end_matches('/')
    )));
    assert!(!public_html.contains(&parent_page), "internal page ID leaked into public HTML");
    assert!(!public_html.contains(&future_child), "unpublished target leaked into public HTML");
    assert!(
        !public_html.contains(&future_path),
        "a planned but unpublished page became a public menu entry"
    );
    assert_eq!((preview.previews[0].required_link_count, preview.previews[0].rendered_link_count), (1, 1));

    // Reconciliation can change candidate links after approval; it cannot
    // rewrite a revision's exact authored/published destination manifest.
    sqlx::query(
        "UPDATE site.link_recommendations SET status='expired' \
         WHERE source_page_key=$1 AND target_page_key=$2",
    )
    .bind(&tourist_page)
    .bind(&parent_page)
    .execute(pool)
    .await
    .unwrap();
    let after_graph_change =
        static_site_builder_adapter::build_static_site_candidate_incremental(
            pool,
            &static_dir,
            "https://example.test",
            &tourist_page,
            &approved.revision_id,
        )
        .await
        .unwrap();
    assert!(after_graph_change.previews[0]
        .rendered_html
        .contains(&format!("href=\"{}\"", parent_path.trim_end_matches('/'))));
    assert_eq!(
        std::fs::read_to_string(
            static_dir.join(tourist_path.trim_matches('/')).join("index.html")
        )
        .unwrap(),
        published_child_html,
        "mutable link recommendations must not rewrite approved public HTML"
    );
    // Conversely, once an approved target is absent from the exact release,
    // silently filtering it out would let the site drop a mandatory link.
    sqlx::query(
        "UPDATE site.cms_pages SET current_revision_id=NULL WHERE page_node_key=$1",
    )
    .bind(&parent_page)
    .execute(pool)
    .await
    .unwrap();
    let missing_target = static_site_builder_adapter::build_static_site_candidate_incremental(
        pool,
        &static_dir,
        "https://example.test",
        &tourist_page,
        &approved.revision_id,
    )
    .await;
    assert!(missing_target.is_err(), "approved link to missing release target must block");
    assert_eq!(
        std::fs::read_to_string(
            static_dir.join(tourist_path.trim_matches('/')).join("index.html")
        )
        .unwrap(),
        published_child_html,
        "failed candidate build must leave the last materialized HTML untouched"
    );
    sqlx::query("UPDATE site.cms_pages SET current_revision_id=$2 WHERE page_node_key=$1")
        .bind(&parent_page)
        .bind(&parent_revision)
        .execute(pool)
        .await
        .unwrap();

    // A published root/hub can receive a new review candidate when its only
    // outgoing required recommendation points to a future, unpublished child.
    // The existing live revision stays live and no future URL is fabricated.
    // A new CMS review must refer to a new immutable page draft; the
    // published parent revision already owns the original parent_draft.
    let parent_replacement_draft = format!("cms-link-parent-draft-2-{marker}");
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown,qa_verdict,truth_snapshot_ref) VALUES ($1,$2,$3,2,'New parent text','publish_ready','verified-truth')")
        .bind(&parent_replacement_draft)
        .bind(&parent_page)
        .bind(&parent_brief)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.link_recommendations (link_recommendation_key,scope_signature,source_page_key,target_page_key,required_flag,link_role,score) VALUES ($1,$2,$3,$4,true,'hub_to_child',0.8)")
        .bind(format!("cms-link-parent-future-{marker}"))
        .bind(&scope)
        .bind(&parent_page)
        .bind(&future_child)
        .execute(pool)
        .await
        .unwrap();
    let hub = sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &input,
        &CmsPublishOutputPayload {
            page_draft_key: parent_replacement_draft,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(hub.verdict, "review_requested");
    assert!(hub.blocking_reasons.is_empty());
    let hub_links: serde_json::Value = sqlx::query_scalar(
        "SELECT required_link_payload FROM site.cms_page_revisions WHERE revision_id=$1",
    )
    .bind(&hub.revision_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(hub_links, serde_json::json!([]));
    let live_hub: String = sqlx::query_scalar(
        "SELECT current_revision_id FROM site.cms_pages WHERE page_node_key=$1",
    )
    .bind(&parent_page)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(live_hub, parent_revision);

    // After explicit acceptance, an unpublished required target must block;
    // a required URL can never be invented from an unpublished page node.
    sqlx::query("UPDATE site.cms_pages SET current_status='review_required' WHERE page_node_key=$1")
        .bind(&parent_page).execute(pool).await.unwrap();
    sqlx::query("UPDATE site.link_recommendations SET status='accepted' WHERE source_page_key=$1 AND target_page_key=$2")
        .bind(&tourist_page).bind(&parent_page).execute(pool).await.unwrap();
    let next_draft = format!("cms-link-tourist-draft-2-{marker}");
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown,qa_verdict,truth_snapshot_ref) VALUES ($1,$2,$3,2,'Fresh content','publish_ready','verified-truth')")
        .bind(&next_draft).bind(&tourist_page).bind(&tourist_brief)
        .execute(pool).await.unwrap();
    let blocked = sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool, &input,
        &CmsPublishOutputPayload { page_draft_key: next_draft, ..Default::default() },
    ).await.unwrap();
    assert_eq!(blocked.verdict, "blocked");
    assert!(blocked.blocking_reasons.contains(&"required_internal_target_not_published".to_string()));
    let (revision_status, page_status): (String, String) = sqlx::query_as(
        "SELECT r.revision_status,p.current_status FROM site.cms_page_revisions r JOIN site.cms_pages p ON p.page_node_key=r.page_node_key WHERE r.revision_id=$1",
    ).bind(&blocked.revision_id).fetch_one(pool).await.unwrap();
    assert_eq!((revision_status.as_str(), page_status.as_str()), ("blocked", "blocked"));

    let invalid_mode = CmsPublishInputPayload {
        publish_mode: "unexpected_mode".into(),
        ..input
    };
    assert!(sqlx_seo_cms_adapter::persist_cms_publish_output(
        pool,
        &invalid_mode,
        &CmsPublishOutputPayload {
            page_draft_key: blocked.page_draft_key,
            ..Default::default()
        },
    ).await.is_err());

    // The provisional wire plan is deliberately empty: persisted draft facts
    // must still block both an LLM fallback and an empty public body.
    for (revision, name, body, provider, reason) in [
        (4, "fallback", "Generated fallback body", "deterministic_fallback", "deterministic_fallback_publish_blocked"),
        (5, "empty", "", "source_backed_template@1", "empty_body"),
    ] {
        let draft_key = format!("cms-persisted-{name}-draft-{marker}");
        sqlx::query(
            "INSERT INTO site.page_drafts \
             (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown,qa_verdict,truth_snapshot_ref,traceability_manifest) \
             VALUES ($1,$2,$3,$4,$5,'publish_ready','verified-truth',$6)",
        )
        .bind(&draft_key)
        .bind(&tourist_page)
        .bind(&tourist_brief)
        .bind(revision)
        .bind(body)
        .bind(serde_json::json!({ "llm_provider_key": provider }))
        .execute(pool)
        .await
        .unwrap();
        let response = sqlx_seo_cms_adapter::persist_cms_publish_output(
            pool,
            &CmsPublishInputPayload {
                run_id: Uuid::new_v4().to_string(),
                publish_mode: "request_review".to_string(),
                ..Default::default()
            },
            &CmsPublishOutputPayload {
                page_draft_key: draft_key,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(response.verdict, "blocked");
        assert!(response.blocking_reasons.iter().any(|value| value == reason));
        assert!(response.publish_artifact.is_none());
    }
}
