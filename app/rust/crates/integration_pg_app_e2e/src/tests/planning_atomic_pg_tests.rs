#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_link_recommend_outbox_failure_rolls_back_then_retries() {
    use contracts::generated::alegria::temporal::v1::{
        LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState,
    };

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let blueprint = format!("planning-atomic-blueprint-{marker}");
    let scope = format!("planning-atomic-scope-{marker}");
    let source = format!("planning-atomic-source-{marker}");
    let target = format!("planning-atomic-target-{marker}");
    let recommendation = format!("planning-atomic-link-{marker}");

    sqlx::query(
        "INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) \
         VALUES ($1,'country_hub_page','informational')",
    )
    .bind(&blueprint)
    .execute(pool)
    .await
    .unwrap();
    for (page_key, slug) in [(&source, "source"), (&target, "target")] {
        sqlx::query(
            "INSERT INTO site.page_nodes \
             (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) \
             VALUES ($1,$2,$3,'country_hub_page','informational',$4,$5,'planned')",
        )
        .bind(page_key)
        .bind(&scope)
        .bind(&blueprint)
        .bind(slug)
        .bind(format!("/test/{marker}/{slug}/"))
        .execute(pool)
        .await
        .unwrap();
    }

    let input = LinkRecommendInputPayload {
        run_id: Uuid::new_v4().to_string(),
        ..Default::default()
    };
    let output = LinkRecommendOutputPayload {
        link_recommendations: vec![LinkRecommendationState {
            link_recommendation_key: recommendation.clone(),
            scope_signature: scope,
            source_page_key: source.clone(),
            target_page_key: target.clone(),
            link_role: "contextual".to_string(),
            anchor_strategy: "descriptive".to_string(),
            required_flag: true,
            score: 0.9,
            status: "candidate".to_string(),
            ..Default::default()
        }],
    };

    // Inject failure at the final Outbox insert after the link row and its
    // required-link rebuild edge have been written inside the transaction.
    sqlx::query(
        "ALTER TABLE system.sync_outbox ADD CONSTRAINT planning_atomic_outbox_fault \
         CHECK (aggregate_type <> 'link_recommendation')",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(
        infrastructure::adapters::sqlx_seo_adapter::persist_link_recommend_output(
            pool, &input, &output,
        )
        .await
        .is_err()
    );
    let (links, dependencies, events): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM site.link_recommendations WHERE link_recommendation_key=$1), \
         (SELECT count(*) FROM monitoring.seo_rebuild_dependencies WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3), \
         (SELECT count(*) FROM system.sync_outbox WHERE aggregate_type='link_recommendation' AND aggregate_key=$1)",
    )
    .bind(&recommendation)
    .bind(&source)
    .bind(&target)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((links, dependencies, events), (0, 0, 0));
    sqlx::query("ALTER TABLE system.sync_outbox DROP CONSTRAINT planning_atomic_outbox_fault")
        .execute(pool)
        .await
        .unwrap();

    infrastructure::adapters::sqlx_seo_adapter::persist_link_recommend_output(
        pool, &input, &output,
    )
    .await
    .unwrap();
    // Retrying the same result must not duplicate its projection event.
    infrastructure::adapters::sqlx_seo_adapter::persist_link_recommend_output(
        pool, &input, &output,
    )
    .await
    .unwrap();
    let (links, dependencies, events): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM site.link_recommendations WHERE link_recommendation_key=$1), \
         (SELECT count(*) FROM monitoring.seo_rebuild_dependencies WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3), \
         (SELECT count(*) FROM system.sync_outbox WHERE aggregate_type='link_recommendation' AND aggregate_key=$1)",
    )
    .bind(&recommendation)
    .bind(&source)
    .bind(&target)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((links, dependencies, events), (1, 1, 1));
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_navigation_late_item_failure_preserves_previous_tree() {
    use infrastructure::adapters::sqlx_seo_adapter::persist_global_navigation_from_active_pages;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let blueprint = format!("navigation-atomic-blueprint-{marker}");
    let scope = format!("navigation-atomic-scope-{marker}");
    let market = format!("navigation-atomic-market-{marker}");
    let locale = "ru-RU";
    let first = format!("navigation-atomic-first-{marker}");
    let second = format!("navigation-atomic-second-{marker}");
    sqlx::query(
        "INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) \
         VALUES ($1,'country_hub_page','informational')",
    )
    .bind(&blueprint)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO site.site_scopes (scope_signature,market,locale) VALUES ($1,$2,$3)",
    )
    .bind(&scope)
    .bind(&market)
    .bind(locale)
    .execute(pool)
    .await
    .unwrap();
    let insert_page =
        "INSERT INTO site.page_nodes \
         (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) \
         VALUES ($1,$2,$3,'country_hub_page','informational',$4,$5,'planned')";
    sqlx::query(insert_page)
        .bind(&first)
        .bind(&scope)
        .bind(&blueprint)
        .bind("first")
        .bind(format!("/test/{marker}/first/"))
        .execute(pool)
        .await
        .unwrap();

    let baseline = persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .unwrap();
    assert!(!baseline.navigation_tree_key.is_empty());
    let old_item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1 AND page_node_key=$2",
    )
    .bind(&baseline.navigation_tree_key)
    .bind(&first)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(old_item_count, 1);

    // A second page makes the next attempt replace the previous tree items
    // and update scope page_count before inserting the second menu row.
    sqlx::query(insert_page)
        .bind(&second)
        .bind(&scope)
        .bind(&blueprint)
        .bind("second")
        .bind(format!("/test/{marker}/second/"))
        .execute(pool)
        .await
        .unwrap();
    // NOT VALID permits the existing first menu row, but rejects the new
    // second item after the DELETE + first INSERT. The transaction must undo
    // the partial menu replacement and scope-count mutation.
    sqlx::query(
        "ALTER TABLE site.navigation_items ADD CONSTRAINT navigation_atomic_late_fault \
         CHECK (page_node_key NOT LIKE 'navigation-atomic-second-%') NOT VALID",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .is_err());
    let old_scope_count: i32 = sqlx::query_scalar(
        "SELECT page_count FROM site.site_scopes WHERE scope_signature=$1",
    )
    .bind(&scope)
    .fetch_one(pool)
    .await
    .unwrap();
    let tree_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1",
    )
    .bind(&baseline.navigation_tree_key)
    .fetch_one(pool)
    .await
    .unwrap();
    let second_item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1 AND page_node_key=$2",
    )
    .bind(&baseline.navigation_tree_key)
    .bind(&second)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((old_scope_count, tree_count, second_item_count), (1, 1, 0));

    sqlx::query(
        "ALTER TABLE site.navigation_items DROP CONSTRAINT navigation_atomic_late_fault",
    )
    .execute(pool)
    .await
    .unwrap();
    let updated = persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .unwrap();
    assert_eq!(updated.navigation_tree_key, baseline.navigation_tree_key);
    let scope_count: i32 = sqlx::query_scalar(
        "SELECT page_count FROM site.site_scopes WHERE scope_signature=$1",
    )
    .bind(&scope)
    .fetch_one(pool)
    .await
    .unwrap();
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1",
    )
    .bind(&updated.navigation_tree_key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((scope_count, item_count), (2, 2));

    // Replaying the same snapshot is stable; no phantom rebuild plan is
    // queued because actual dispatch uses monitoring.seo_rebuild_backlog.
    persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .unwrap();
    let (stable_items, phantom_plans): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1),
            (SELECT count(*) FROM site.global_rebuild_plan
             WHERE trigger_type='navigation_reconciled')",
    )
    .bind(&updated.navigation_tree_key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((stable_items, phantom_plans), (2, 0));

    // Removing the last active page clears the menu, scopes and groups; it
    // must not enqueue an unconsumed navigation-only rebuild ceremony.
    sqlx::query(
        "UPDATE site.page_nodes SET lifecycle_state='deprecated' WHERE page_node_key=ANY($1)",
    )
    .bind(&vec![first, second])
    .execute(pool)
    .await
    .unwrap();
    let emptied = persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .unwrap();
    assert_eq!(emptied.page_item_count, 0);
    let items_after_retirement: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items WHERE navigation_tree_key=$1",
    )
    .bind(&updated.navigation_tree_key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(items_after_retirement, 0);
    let (empty_scope_pages, empty_scope_status): (i32, String) = sqlx::query_as(
        "SELECT page_count,status FROM site.site_scopes WHERE scope_signature=$1",
    )
    .bind(&scope)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((empty_scope_pages, empty_scope_status.as_str()), (0, "deprecated"));
    persist_global_navigation_from_active_pages(pool, &market, locale)
        .await
        .unwrap();
    let phantom_empty: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.global_rebuild_plan
         WHERE trigger_type='navigation_reconciled'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(phantom_empty, 0);
}

/// Two real site markets cannot write each other's planned pages into their
/// persisted navigation snapshots, nor retire the other market's scope.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_navigation_two_markets_are_isolated_and_retire_independently() {
    use infrastructure::adapters::sqlx_seo_adapter::persist_global_navigation_from_active_pages;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let blueprint = format!("nav-markets-blueprint-{marker}");
    sqlx::query(
        "INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) \
         VALUES ($1,'country_hub_page','informational')",
    )
    .bind(&blueprint)
    .execute(pool)
    .await
    .unwrap();
    let scopes = [
        (
            format!("nav-market-a-scope-{marker}"),
            format!("nav-market-a-{marker}"),
            "ru-RU",
            format!("nav-market-a-page-{marker}"),
        ),
        (
            format!("nav-market-b-scope-{marker}"),
            format!("nav-market-b-{marker}"),
            "en-US",
            format!("nav-market-b-page-{marker}"),
        ),
    ];
    for (index, (scope, market, locale, page)) in scopes.iter().enumerate() {
        sqlx::query(
            "INSERT INTO site.site_scopes (scope_signature,market,locale) VALUES ($1,$2,$3)",
        )
        .bind(scope)
        .bind(market)
        .bind(*locale)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO site.page_nodes \
             (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state,menu_group) \
             VALUES ($1,$2,$3,'country_hub_page','informational',$4,$5,'planned',$6)",
        )
        .bind(page)
        .bind(scope)
        .bind(&blueprint)
        .bind(format!("country-{}", index + 1))
        .bind(format!("/{locale}/{marker}/{}", index + 1))
        .bind("visa:shared-name")
        .execute(pool)
        .await
        .unwrap();
    }
    let a = persist_global_navigation_from_active_pages(
        pool,
        &scopes[0].1,
        scopes[0].2,
    )
    .await
    .unwrap();
    let b = persist_global_navigation_from_active_pages(
        pool,
        &scopes[1].1,
        scopes[1].2,
    )
    .await
    .unwrap();
    assert_ne!(a.navigation_tree_key, b.navigation_tree_key);
    assert_eq!((a.page_item_count, b.page_item_count), (1, 1));
    for (tree, expected, forbidden) in [
        (&a.navigation_tree_key, &scopes[0].3, &scopes[1].3),
        (&b.navigation_tree_key, &scopes[1].3, &scopes[0].3),
    ] {
        let (correct, bleed): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE page_node_key=$2), \
                    count(*) FILTER (WHERE page_node_key=$3) \
             FROM site.navigation_items WHERE navigation_tree_key=$1 AND status='active'",
        )
        .bind(tree.as_str())
        .bind(expected.as_str())
        .bind(forbidden.as_str())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!((correct, bleed), (1, 0));
    }
    // A market-B-only retirement must not change the market-A menu, scope
    // status or rebuild revision.
    sqlx::query("UPDATE site.page_nodes SET lifecycle_state='deprecated' WHERE page_node_key=$1")
        .bind(&scopes[1].3)
        .execute(pool)
        .await
        .unwrap();
    let b_empty = persist_global_navigation_from_active_pages(
        pool,
        &scopes[1].1,
        scopes[1].2,
    )
    .await
    .unwrap();
    assert_eq!(b_empty.page_item_count, 0);
    let unchanged_a: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items \
         WHERE navigation_tree_key=$1 AND page_node_key=$2",
    )
    .bind(&a.navigation_tree_key)
    .bind(&scopes[0].3)
    .fetch_one(pool)
    .await
    .unwrap();
    let statuses: Vec<(String, i32, String)> = sqlx::query_as(
        "SELECT scope_signature,page_count,status FROM site.site_scopes \
         WHERE scope_signature = ANY($1) ORDER BY scope_signature",
    )
    .bind(&vec![scopes[0].0.clone(), scopes[1].0.clone()])
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(unchanged_a, 1);
    assert!(statuses.iter().any(|row| row.0 == scopes[0].0 && row.1 == 1 && row.2 == "active"));
    assert!(statuses.iter().any(|row| row.0 == scopes[1].0 && row.1 == 0 && row.2 == "deprecated"));
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_registered_input_late_blob_failure_rolls_back_run() {
    use infrastructure::adapters::sqlx_seo_adapter;
    use pg_runtime_ports::SqlxSeoPgRepository;
    use seo_ports::{SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest};

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let repo = SqlxSeoPgRepository::new(pool);
    let registered = repo
        .register_site_build_input(&SeoSiteBuildRegistrationRequest {
            run_id: Uuid::new_v4().to_string(),
            context_key: None,
            market: "alegria-site".into(),
            locale: "ru-RU".into(),
            country_code: "ES".into(),
            visa_type: "tourist".into(),
            visa_subtype: None,
            applicant_profile: "standard".into(),
            citizenship_code: "BY".into(),
            bootstrap_context: true,
            queries: vec!["spain visa".into()],
            query_batch_key: None,
            run_mode: Some("publish_with_hitl".into()),
            retained_source_run_id: None,
        })
        .await
        .unwrap();
    let mut input = registered.clone();
    input.run_id = Uuid::new_v4().to_string();

    // The existing registration has already populated another run. Reject
    // only subsequent input blob writes, after execution_runs INSERT, without
    // requiring deletion or mutation of that valid fixture.
    sqlx::query(
        "ALTER TABLE pipeline.execution_run_blobs ADD CONSTRAINT planning_atomic_blob_fault \
         CHECK (field_name <> 'input_payload') NOT VALID",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(sqlx_seo_adapter::upsert_seo_site_build_input(pool, &input)
        .await
        .is_err());
    let absent_run: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1::uuid",
    )
    .bind(&input.run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let absent_blob: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.execution_run_blobs WHERE run_id=$1::uuid AND field_name='input_payload'",
    )
    .bind(&input.run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((absent_run, absent_blob), (0, 0));

    sqlx::query(
        "ALTER TABLE pipeline.execution_run_blobs DROP CONSTRAINT planning_atomic_blob_fault",
    )
    .execute(pool)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx_seo_adapter::upsert_seo_site_build_input(pool, &input)
            .await
            .unwrap();
    }
    let (runs, blobs): (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1::uuid), \
         (SELECT count(*) FROM pipeline.execution_run_blobs WHERE run_id=$1::uuid AND field_name='input_payload')",
    )
    .bind(&input.run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((runs, blobs), (1, 1));
}

/// The first canonical planning run may omit the precomputed scope signature.
/// SQL must register the SAME derived scope identity before navigation reads
/// its pages, including a fresh database with no prior site_scopes row.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_first_reconcile_registers_derived_scope_for_navigation() {
    use contracts::generated::alegria::temporal::v1::{
        GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, PageNodeState,
        SeoScopePayload,
    };
    use pg_runtime_ports::SqlxSeoPgRepository;
    use seo_ports::PlanningRepository;

    let pg = PostgresHarness::start().await.unwrap();
    let marker = Uuid::new_v4().simple().to_string();
    let page_key = format!("first-derived-scope-page-{marker}");
    let scope = SeoScopePayload {
        market: format!("derived-market-{marker}"),
        locale: "ru-RU".into(),
        country_code: "ES".into(),
        visa_type: "tourist".into(),
        applicant_profile: "standard".into(),
        scope_signature: String::new(),
        ..Default::default()
    };
    let identity = seo_steps::seo_step_support::scope_signature(Some(&scope)).unwrap();
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM site.site_scopes WHERE scope_signature=$1")
            .bind(&identity)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(before, 0);
    let input = GlobalSiteReconcileInputPayload {
        run_id: Uuid::new_v4().to_string(),
        scope: Some(scope.clone()),
        reconcile_reason: format!("first-derived-scope-{marker}"),
        ..Default::default()
    };
    let output = GlobalSiteReconcileOutputPayload {
        page_nodes: vec![PageNodeState {
            page_node_key: page_key.clone(),
            scope_signature: identity.clone(),
            page_type_key: "country_hub_page".into(),
            dominant_intent: "informational".into(),
            canonical_slug: "spain".into(),
            canonical_url_path: format!("/ru/{marker}/spain/"),
            lifecycle_state: "planned".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let repo = SqlxSeoPgRepository::new(&pg.pool);
    repo.persist_global_site_reconcile_output(&input, &output)
        .await
        .unwrap();
    let (market, locale, page_count): (String, String, i32) = sqlx::query_as(
        "SELECT market,locale,page_count FROM site.site_scopes WHERE scope_signature=$1",
    )
    .bind(&identity)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(
        (&market, &locale, page_count),
        (&scope.market, &scope.locale, 1)
    );
    let nav_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items i \
         JOIN site.navigation_trees t ON t.navigation_tree_key=i.navigation_tree_key \
         WHERE t.market=$1 AND t.locale=$2 AND i.page_node_key=$3",
    )
    .bind(market)
    .bind(locale)
    .bind(page_key)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(nav_count, 1, "fresh derived scope was not included in navigation");
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn planning_atomic_global_reconcile_late_navigation_failure_rolls_back_every_owner() {
    use contracts::generated::alegria::temporal::v1::{
        ContentGapState, GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload,
        PageNodeState, SeoScopePayload,
    };
    use pg_runtime_ports::SqlxSeoPgRepository;
    use seo_domain::artifact_identity::ContentGapDetector;
    use seo_ports::PlanningRepository;

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let run_id = Uuid::new_v4().to_string();
    let market = format!("global-atomic-market-{marker}");
    let scope = SeoScopePayload {
        market,
        locale: "ru-RU".into(),
        country_code: "ES".into(),
        visa_type: "tourist".into(),
        applicant_profile: "standard".into(),
        scope_signature: String::new(),
        ..Default::default()
    };
    let scope_signature = seo_steps::seo_step_support::scope_signature(Some(&scope)).unwrap();
    let first = format!("global-atomic-first-{marker}");
    let second = format!("global-atomic-second-{marker}");
    let link = format!("global-atomic-link-{marker}");
    let obsolete = format!("global-atomic-obsolete-{marker}");
    let obsolete_target = format!("global-atomic-obsolete-target-{marker}");
    let expired_link = format!("global-atomic-expired-link-{marker}");
    let expired_overlap = format!("global-atomic-overlapping-expired-{marker}");
    let optional_link = format!("global-atomic-optional-link-{marker}");
    // A pre-existing opaque, unpublished plan is retired only after the new
    // IA/link/opportunity writes. Its original state must survive a late fault.
    for (key, path) in [
        (&obsolete, format!("/legacy/{marker}/")),
        (&obsolete_target, format!("/legacy/{marker}/target/")),
    ] {
        sqlx::query(
            "INSERT INTO site.page_nodes \
             (page_node_key,scope_signature,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) \
             VALUES ($1,$2,'country_hub_page','informational',$3,$4,'planned')",
        )
        .bind(key)
        .bind(&scope_signature)
        .bind(&marker)
        .bind(path)
        .execute(pool)
        .await
        .unwrap();
    }
    for (recommendation, source, target, role) in [
        (&expired_link, &obsolete, &obsolete_target, "hub_to_child"),
        (&optional_link, &obsolete_target, &obsolete, "child_to_hub"),
    ] {
        sqlx::query(
            "INSERT INTO site.link_recommendations \
             (link_recommendation_key,scope_signature,source_page_key,target_page_key, \
              link_role,anchor_strategy,required_flag,score,status) \
             VALUES ($1,$2,$3,$4,$5,'navigation_contextual',true,0.95,'accepted')",
        )
        .bind(recommendation)
        .bind(&scope_signature)
        .bind(source)
        .bind(target)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO monitoring.seo_rebuild_dependencies \
             (rebuild_dependency_key,page_node_key,dependency_type,dependency_ref,status) \
             VALUES ($1,$2,'required_link',$3,'active')",
        )
        .bind(seo_domain::artifact_identity::seo_artifact_key(
            "rebuild_dependency",
            &[source, "required_link", target],
        ))
        .bind(source)
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    }
    let gap_detector = ContentGapDetector::SerpReliability;
    let gap = gap_detector.key(&scope_signature, "", "who_fits");
    let pages = [(&first, "first"), (&second, "second")]
        .into_iter()
        .map(|(key, slug)| PageNodeState {
            page_node_key: (*key).clone(),
            scope_signature: scope_signature.clone(),
            page_type_key: "country_hub_page".into(),
            dominant_intent: "informational".into(),
            canonical_slug: slug.into(),
            canonical_url_path: format!("/test/{marker}/{slug}/"),
            lifecycle_state: "planned".into(),
            menu_group: "main".into(),
            breadcrumb_policy: "standard".into(),
            canonical_url_family: "country_hub".into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let input = GlobalSiteReconcileInputPayload {
        run_id: run_id.clone(),
        scope: Some(scope),
        reconcile_reason: format!("atomic-reconcile-{marker}"),
        ..Default::default()
    };
    let output = GlobalSiteReconcileOutputPayload {
        page_nodes: pages,
        link_recommendations: vec![
            LinkRecommendationState {
                link_recommendation_key: link.clone(),
                scope_signature: scope_signature.clone(),
                source_page_key: first.clone(),
                target_page_key: second.clone(),
                link_role: "contextual".into(),
                anchor_strategy: "descriptive".into(),
                required_flag: true,
                score: 0.9,
                status: "candidate".into(),
                ..Default::default()
            },
            LinkRecommendationState {
                link_recommendation_key: expired_overlap.clone(),
                scope_signature: scope_signature.clone(),
                source_page_key: first.clone(),
                target_page_key: second.clone(),
                link_role: "legacy_contextual".into(),
                anchor_strategy: "legacy_descriptive".into(),
                required_flag: true,
                score: 0.7,
                status: "expired".into(),
                ..Default::default()
            },
            LinkRecommendationState {
                link_recommendation_key: expired_link.clone(),
                scope_signature: scope_signature.clone(),
                source_page_key: obsolete.clone(),
                target_page_key: obsolete_target.clone(),
                link_role: "hub_to_child".into(),
                anchor_strategy: "navigation_contextual".into(),
                required_flag: true,
                score: 0.95,
                status: "expired".into(),
                ..Default::default()
            },
            LinkRecommendationState {
                link_recommendation_key: optional_link.clone(),
                scope_signature: scope_signature.clone(),
                source_page_key: obsolete_target.clone(),
                target_page_key: obsolete.clone(),
                link_role: "child_to_hub".into(),
                anchor_strategy: "navigation_contextual".into(),
                required_flag: false,
                score: 0.95,
                status: "accepted".into(),
                ..Default::default()
            },
        ],
        content_gaps: vec![ContentGapState {
            content_gap_key: gap.clone(),
            scope_signature: scope_signature.clone(),
            missing_topic: "who_fits".into(),
            severity: "medium".into(),
            status: "open".into(),
            reason_code: gap_detector.reason().into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let repo = SqlxSeoPgRepository::new(pool);

    // Navigation's persisted menu item is written after the IA pages,
    // required links, gaps and their Outbox events. Rejecting a real menu
    // write must roll back every owner in the global reconcile transaction.
    sqlx::query(
        "ALTER TABLE site.navigation_items ADD CONSTRAINT global_atomic_navigation_fault \
         CHECK (page_node_key NOT LIKE 'global-atomic-first-%') NOT VALID",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(repo
        .persist_global_site_reconcile_output(&input, &output)
        .await
        .is_err());
    let (pages, links, gaps, dependencies, events, plans, trees): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM site.page_nodes WHERE page_node_key=ANY($1)), \
             (SELECT count(*) FROM site.link_recommendations WHERE link_recommendation_key=$2), \
             (SELECT count(*) FROM site.content_gaps WHERE content_gap_key=$3), \
             (SELECT count(*) FROM monitoring.seo_rebuild_dependencies WHERE page_node_key=$4), \
             (SELECT count(*) FROM system.sync_outbox WHERE run_id=$5), \
             (SELECT count(*) FROM site.global_rebuild_plan WHERE trigger_type='navigation_reconciled'), \
             (SELECT count(*) FROM site.navigation_trees WHERE market=$6)",
    )
    .bind(vec![first.clone(), second.clone()])
    .bind(&link)
    .bind(&gap)
    .bind(&first)
    .bind(&run_id)
    .bind(input.scope.as_ref().unwrap().market.as_str())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        (pages, links, gaps, dependencies, events, plans, trees),
        (0, 0, 0, 0, 0, 0, 0)
    );
    let obsolete_after_failure: String = sqlx::query_scalar(
        "SELECT lifecycle_state FROM site.page_nodes WHERE page_node_key=$1",
    )
    .bind(&obsolete)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(obsolete_after_failure, "planned");
    let (prior_link_after_failure, prior_edge_after_failure): (String, String) =
        sqlx::query_as(
            "SELECT \
             (SELECT status FROM site.link_recommendations WHERE link_recommendation_key=$1), \
             (SELECT status FROM monitoring.seo_rebuild_dependencies \
              WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3)",
        )
        .bind(&expired_link)
        .bind(&obsolete)
        .bind(&obsolete_target)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        (prior_link_after_failure.as_str(), prior_edge_after_failure.as_str()),
        ("accepted", "active")
    );
    let optional_still_required: bool = sqlx::query_scalar(
        "SELECT required_flag FROM site.link_recommendations WHERE link_recommendation_key=$1",
    )
    .bind(&optional_link)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(optional_still_required);
    sqlx::query(
        "ALTER TABLE site.navigation_items DROP CONSTRAINT global_atomic_navigation_fault",
    )
    .execute(pool)
    .await
    .unwrap();

    repo.persist_global_site_reconcile_output(&input, &output)
        .await
        .unwrap();
    let reconciled_menu_items: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.navigation_items i \
         JOIN site.navigation_trees t ON t.navigation_tree_key=i.navigation_tree_key \
         WHERE t.market=$1 AND i.page_node_key=ANY($2)",
    )
    .bind(input.scope.as_ref().unwrap().market.as_str())
    .bind(vec![first.clone(), second.clone()])
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(reconciled_menu_items, 2);
    let obsolete_after_success: String = sqlx::query_scalar(
        "SELECT lifecycle_state FROM site.page_nodes WHERE page_node_key=$1",
    )
    .bind(&obsolete)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(obsolete_after_success, "deprecated");
    let (expired_status, deprecated_edge): (String, String) = sqlx::query_as(
        "SELECT \
         (SELECT status FROM site.link_recommendations WHERE link_recommendation_key=$1), \
         (SELECT status FROM monitoring.seo_rebuild_dependencies \
          WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3)",
    )
    .bind(&expired_link)
    .bind(&obsolete)
    .bind(&obsolete_target)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((expired_status.as_str(), deprecated_edge.as_str()), ("expired", "deprecated"));
    let (optional_is_required, optional_edge): (bool, String) = sqlx::query_as(
        "SELECT \
         (SELECT required_flag FROM site.link_recommendations WHERE link_recommendation_key=$1), \
         (SELECT status FROM monitoring.seo_rebuild_dependencies \
          WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3)",
    )
    .bind(&optional_link)
    .bind(&obsolete_target)
    .bind(&obsolete)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((optional_is_required, optional_edge.as_str()), (false, "deprecated"));
    let (expired_overlap_status, live_edge_status): (String, String) = sqlx::query_as(
        "SELECT \
         (SELECT status FROM site.link_recommendations WHERE link_recommendation_key=$1), \
         (SELECT status FROM monitoring.seo_rebuild_dependencies \
          WHERE page_node_key=$2 AND dependency_type='required_link' AND dependency_ref=$3)",
    )
    .bind(&expired_overlap)
    .bind(&first)
    .bind(&second)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((expired_overlap_status.as_str(), live_edge_status.as_str()), ("expired", "active"));
    let projections_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox WHERE run_id=$1",
    )
    .bind(&run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(projections_before >= 7);
    repo.persist_global_site_reconcile_output(&input, &output)
        .await
        .unwrap();
    let projections_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox WHERE run_id=$1",
    )
    .bind(&run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(projections_after, projections_before);
    let gap_qdrant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM system.sync_outbox \
         WHERE aggregate_key=$1 AND event_type='QdrantUpsertCommand'",
    )
    .bind(&gap)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(gap_qdrant, 1, "the same Qdrant content must not multiply on replay");
}
