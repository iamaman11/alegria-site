/// Navigation-only changes must refresh the physically published HTML, without
/// minting or changing CMS revisions. The delivery port makes this a real
/// PostgreSQL → static files → atomic live symlink → readback/rollback test.
#[cfg(all(feature = "e2e", unix))]
#[tokio::test]
#[serial]
async fn cms_revision_navigation_only_refreshes_real_public_html_and_recovers_from_readback_failure(
) {
    use infrastructure::adapters::{
        sqlx_seo_adapter, sqlx_seo_cms_adapter, static_site_builder_adapter,
    };
    use seo_ports::{
        PublicReleaseDelivery, PublicReleaseFile, ReleaseDeliveryError, ReleaseDeliveryStage,
    };
    use std::{
        collections::HashMap,
        path::Path,
        sync::{Arc, Mutex},
    };

    struct LocalPublicDelivery {
        routes: Vec<(String, String)>,
        remote: Mutex<HashMap<String, Vec<u8>>>,
        deploys: Mutex<usize>,
        fail_next_readback: Mutex<bool>,
    }

    #[async_trait::async_trait]
    impl PublicReleaseDelivery for LocalPublicDelivery {
        fn preflight(&self) -> Result<(), ReleaseDeliveryError> {
            Ok(())
        }

        fn deploy(
            &self,
            release: &Path,
            _previous: Option<&Path>,
        ) -> Result<(), ReleaseDeliveryError> {
            let mut served = HashMap::new();
            for (file, route) in &self.routes {
                let bytes =
                    std::fs::read(release.join(file)).map_err(|err| ReleaseDeliveryError {
                        stage: ReleaseDeliveryStage::Upload,
                        detail: err.to_string(),
                    })?;
                served.insert(route.clone(), bytes);
            }
            *self.remote.lock().unwrap() = served;
            *self.deploys.lock().unwrap() += 1;
            Ok(())
        }

        async fn verify(
            &self,
            origin: &str,
            files: &[PublicReleaseFile],
        ) -> Result<(), ReleaseDeliveryError> {
            if origin != "https://staging.alegria.by/"
                || std::mem::take(&mut *self.fail_next_readback.lock().unwrap())
            {
                return Err(ReleaseDeliveryError {
                    stage: ReleaseDeliveryStage::Readback,
                    detail: "injected public readback fault".to_string(),
                });
            }
            let served = self.remote.lock().unwrap();
            for artifact in files
                .iter()
                .filter(|artifact| !artifact.public_route.is_empty())
            {
                if served.get(&artifact.public_route) != Some(&artifact.bytes) {
                    return Err(ReleaseDeliveryError {
                        stage: ReleaseDeliveryStage::Readback,
                        detail: format!("public route {} does not match", artifact.public_route),
                    });
                }
            }
            Ok(())
        }
    }

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let market = format!("navigation-release-market-{marker}");
    let scope = format!("navigation-release-scope-{marker}");
    let blueprint = format!("navigation-release-blueprint-{marker}");
    let parent = format!("navigation-release-parent-{marker}");
    let child = format!("navigation-release-child-{marker}");
    let parent_path = format!("/ru/visa/{marker}/");
    let child_path = format!("{parent_path}tourist/");
    sqlx::query("INSERT INTO site.site_scopes (scope_signature,market,locale) VALUES ($1,$2,'ru')")
        .bind(&scope)
        .bind(&market)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) VALUES ($1,'country_hub_page','informational')")
        .bind(&blueprint).execute(pool).await.unwrap();

    let mut original_revisions = Vec::new();
    for (ordinal, page, path, slug) in [
        (1, &parent, &parent_path, "hub"),
        (2, &child, &child_path, "tourist"),
    ] {
        let brief = format!("nav-brief-{marker}-{ordinal}");
        let draft = format!("nav-draft-{marker}-{ordinal}");
        let revision = format!("nav-revision-{marker}-{ordinal}");
        sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) VALUES ($1,$2,$3,'country_hub_page','informational',$4,$5,'published')")
            .bind(page).bind(&scope).bind(&blueprint).bind(slug).bind(path)
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,$4)")
            .bind(&brief).bind(page).bind(&blueprint).bind(format!("Nav page {ordinal}"))
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,body_markdown) VALUES ($1,$2,$3,'Stable published body')")
            .bind(&draft).bind(page).bind(&brief).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,$5,$5,$6,'published')")
            .bind(&revision).bind(page).bind(&draft).bind(&brief)
            .bind(format!("Nav page {ordinal}")).bind(&blueprint)
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status,published_at) VALUES ($1,$2,$3,'ru','country_hub_page','informational',$4,$5,'published',now())")
            .bind(page).bind(&scope).bind(path).bind(format!("nav-cms-{ordinal}-{marker}"))
            .bind(&revision).execute(pool).await.unwrap();
        original_revisions.push(revision);
    }
    sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, &market, "ru")
        .await
        .unwrap();

    let root = std::env::temp_dir().join(format!("alegria-navigation-release-{marker}"));
    std::fs::create_dir(&root).unwrap();
    let baseline = root.join("initial-published");
    static_site_builder_adapter::build_static_site(pool, &baseline, "https://staging.alegria.by")
        .await
        .unwrap();
    let manifest_path = baseline.join("alegria-static-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["publication_run_id"] = serde_json::json!(Uuid::new_v4().to_string());
    manifest["publication_page_node_key"] = serde_json::json!(parent);
    manifest["publication_revision_id"] = serde_json::json!(original_revisions[0]);
    let initial_fingerprint = manifest["navigation_fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        baseline.join(".alegria_static_site"),
        b"alegria_immutable_release@1\n",
    )
    .unwrap();
    let live = root.join("public");
    std::os::unix::fs::symlink(&baseline, &live).unwrap();
    let _env = env_overrides::EnvOverrideGuard::apply([
        (
            "SEO_STATIC_OUTPUT_DIR".to_owned(),
            live.to_str().unwrap().to_owned(),
        ),
        (
            "SEO_STATIC_RELEASE_ROOT".to_owned(),
            root.to_str().unwrap().to_owned(),
        ),
        (
            "SEO_STATIC_BASE_URL".to_owned(),
            "https://staging.alegria.by".to_owned(),
        ),
    ]);
    let delivery = Arc::new(LocalPublicDelivery {
        routes: vec![
            (
                format!("{}/index.html", parent_path.trim_matches('/')),
                parent_path.trim_end_matches('/').to_string(),
            ),
            (
                format!("{}/index.html", child_path.trim_matches('/')),
                child_path.trim_end_matches('/').to_string(),
            ),
            ("sitemap.xml".into(), "/sitemap.xml".into()),
            ("robots.txt".into(), "/robots.txt".into()),
        ],
        remote: Mutex::new(HashMap::new()),
        deploys: Mutex::new(0),
        fail_next_readback: Mutex::new(false),
    });
    delivery.deploy(&baseline, None).unwrap();

    // A separately published market is in the same PostgreSQL instance, but
    // not in this site's release. A navigation refresh must never turn the
    // cross-scope CMS catalog into a global filesystem deployment.
    let foreign_scope = format!("navigation-foreign-scope-{marker}");
    let foreign_page = format!("navigation-foreign-page-{marker}");
    let foreign_path = format!("/en/visas/{marker}/");
    let foreign_brief = format!("navigation-foreign-brief-{marker}");
    let foreign_draft = format!("navigation-foreign-draft-{marker}");
    let foreign_revision = format!("navigation-foreign-revision-{marker}");
    sqlx::query("INSERT INTO site.site_scopes (scope_signature,market,locale) VALUES ($1,'foreign-market','en')")
        .bind(&foreign_scope).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) VALUES ($1,$2,$3,'country_hub_page','informational','foreign',$4,'published')")
        .bind(&foreign_page).bind(&foreign_scope).bind(&blueprint).bind(&foreign_path)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'Foreign page')")
        .bind(&foreign_brief).bind(&foreign_page).bind(&blueprint)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,body_markdown) VALUES ($1,$2,$3,'Foreign body')")
        .bind(&foreign_draft).bind(&foreign_page).bind(&foreign_brief)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Foreign page','Foreign page',$5,'published')")
        .bind(&foreign_revision).bind(&foreign_page).bind(&foreign_draft)
        .bind(&foreign_brief).bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status,published_at) VALUES ($1,$2,$3,'en','country_hub_page','informational',$4,$5,'published',now())")
        .bind(&foreign_page).bind(&foreign_scope).bind(&foreign_path)
        .bind(format!("foreign-cms-{marker}")).bind(&foreign_revision)
        .execute(pool).await.unwrap();
    let foreign_html = live.join(foreign_path.trim_matches('/')).join("index.html");
    assert!(!foreign_html.exists());

    // Production's PostgreSQL NOTIFY is the actual trigger consumed by the
    // Temporal Worker listener. Identical reconciliation should be silent.
    let mut listener = sqlx::postgres::PgListener::connect(&pg.database_url)
        .await
        .unwrap();
    listener
        .listen(sqlx_seo_adapter::NAVIGATION_NOTIFY_CHANNEL)
        .await
        .unwrap();
    sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, &market, "ru")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.recv())
            .await
            .is_err(),
        "identical navigation triggered a redundant rebuild"
    );

    // Use the same long-running NOTIFY/catch-up loop as the Temporal Worker,
    // with the existing delivery port bound to our local public HTTP fixture.
    // PostgreSQL exposes the last LISTEN statement, so this is a real
    // readiness barrier rather than a guessed startup sleep.
    let listener_name = format!("alegria_navigation_e2e_{marker}");
    let listener_url = format!("{}&application_name={listener_name}", pg.database_url);
    let refresh_delivery: Arc<dyn PublicReleaseDelivery> = delivery.clone();
    let refresh_loop = tokio::spawn(sqlx_seo_cms_adapter::run_published_navigation_refresh_loop(
        Arc::new(pool.clone()),
        listener_url.clone(),
        refresh_delivery,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let listening: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                 WHERE application_name=$1 AND state='idle' AND query LIKE 'LISTEN%')",
            )
            .bind(&listener_name)
            .fetch_one(pool)
            .await
            .unwrap();
            if listening {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("published navigation refresh listener did not become ready");

    let child_html_path = live.join(child_path.trim_matches('/')).join("index.html");
    let original_html = std::fs::read_to_string(&child_html_path).unwrap();
    assert!(original_html.contains("Tourist Visa"));

    // Change navigation metadata alone; approved CMS content and revision
    // pointers remain immutable.
    sqlx::query("UPDATE site.page_nodes SET canonical_slug='visitor' WHERE page_node_key=$1")
        .bind(&child)
        .execute(pool)
        .await
        .unwrap();
    let changed =
        sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, &market, "ru")
            .await
            .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
        .await
        .expect("navigation change must notify the Worker")
        .unwrap();
    assert_eq!(event.payload(), changed.navigation_tree_key);
    // No direct refresh call: the background listener must publish and
    // finish its atomic commit before this test observes the real HTML.
    let updated_html = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let html = std::fs::read_to_string(&child_html_path).unwrap();
            if html.contains("Visitor Visa")
                && !html.contains("Tourist Visa")
                && !root.join(".alegria-pending-render-refresh.json").exists()
            {
                break html;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("automatic Worker navigation refresh did not publish updated HTML");
    assert!(updated_html.contains("Visitor Visa"));
    assert!(!updated_html.contains("Tourist Visa"));
    let updated_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(live.join("alegria-static-manifest.json")).unwrap())
            .unwrap();
    assert_ne!(
        updated_manifest["navigation_fingerprint"],
        initial_fingerprint
    );
    assert!(
        !foreign_html.exists(),
        "foreign scope HTML leaked into this release"
    );
    assert!(
        updated_manifest["pages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|page| { page["page_node_key"] != foreign_page }),
        "foreign published CMS revision leaked into release manifest"
    );
    assert_eq!(
        updated_manifest["publication_revision_id"],
        original_revisions[0]
    );
    for (page, revision) in [
        (&parent, &original_revisions[0]),
        (&child, &original_revisions[1]),
    ] {
        let current: String = sqlx::query_scalar(
            "SELECT current_revision_id FROM site.cms_pages WHERE page_node_key=$1",
        )
        .bind(page)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            &current, revision,
            "navigation-only refresh changed approved CMS revision"
        );
    }
    let count_after_change = *delivery.deploys.lock().unwrap();
    assert!(
        !sqlx_seo_cms_adapter::refresh_published_renderer_if_stale_with_delivery(
            pool,
            delivery.as_ref(),
        )
        .await
        .unwrap()
    );
    assert_eq!(*delivery.deploys.lock().unwrap(), count_after_change);
    let release_before_failure = std::fs::read_link(&live).unwrap();

    sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, &market, "ru")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.recv())
            .await
            .is_err(),
        "unchanged menu must not trigger another release"
    );

    // Failure after upload but before HTTPS confirmation must be rolled back
    // by the live Worker listener itself, without a test-driven refresh call.
    // A restarted listener must catch up from PostgreSQL even if the original
    // notification was already consumed by the failed attempt.
    let deployments_before_failure = *delivery.deploys.lock().unwrap();
    *delivery.fail_next_readback.lock().unwrap() = true;
    sqlx::query("UPDATE site.page_nodes SET canonical_slug='updated' WHERE page_node_key=$1")
        .bind(&child)
        .execute(pool)
        .await
        .unwrap();
    sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, &market, "ru")
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let deployments = *delivery.deploys.lock().unwrap();
            let local = std::fs::read_to_string(&child_html_path).unwrap();
            let rolled_back = {
                let remote = delivery.remote.lock().unwrap();
                remote
                    .get(child_path.trim_end_matches('/'))
                    .is_some_and(|bytes| bytes == updated_html.as_bytes())
            };
            if deployments >= deployments_before_failure + 2
                && local == updated_html
                && rolled_back
                && !root.join(".alegria-pending-render-refresh.json").exists()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("navigation listener did not roll back failed public readback");
    assert_eq!(std::fs::read_link(&live).unwrap(), release_before_failure);
    assert_eq!(
        std::fs::read_to_string(&child_html_path).unwrap(),
        updated_html
    );
    assert_eq!(
        delivery
            .remote
            .lock()
            .unwrap()
            .get(child_path.trim_end_matches('/'))
            .unwrap()
            .as_slice(),
        updated_html.as_bytes()
    );
    refresh_loop.abort();
    let _ = refresh_loop.await;

    let resumed_delivery: Arc<dyn PublicReleaseDelivery> = delivery.clone();
    let resumed_loop = tokio::spawn(sqlx_seo_cms_adapter::run_published_navigation_refresh_loop(
        Arc::new(pool.clone()),
        listener_url,
        resumed_delivery,
    ));
    let recovered_html = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let html = std::fs::read_to_string(&child_html_path).unwrap();
            if html.contains("Updated Visa")
                && !root.join(".alegria-pending-render-refresh.json").exists()
            {
                break html;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("restarted navigation listener did not catch up the missed release");
    resumed_loop.abort();
    let _ = resumed_loop.await;
    assert!(!recovered_html.contains("Visitor Visa"));
    assert_ne!(std::fs::read_link(&live).unwrap(), release_before_failure);
    assert_eq!(
        delivery
            .remote
            .lock()
            .unwrap()
            .get(child_path.trim_end_matches('/'))
            .unwrap()
            .as_slice(),
        recovered_html.as_bytes()
    );
    for (page, revision) in [
        (&parent, &original_revisions[0]),
        (&child, &original_revisions[1]),
    ] {
        let current: String = sqlx::query_scalar(
            "SELECT current_revision_id FROM site.cms_pages WHERE page_node_key=$1",
        )
        .bind(page)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            &current, revision,
            "navigation recovery invented a CMS revision"
        );
    }
    assert!(!root.join(".alegria-pending-render-refresh.json").exists());
    drop(_env);
    std::fs::remove_dir_all(&root).unwrap();
}
