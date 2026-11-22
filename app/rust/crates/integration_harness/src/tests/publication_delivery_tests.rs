#[cfg(all(feature = "e2e", unix))]
#[tokio::test]
#[serial]
async fn publishing_a17_postgres_outbox_http_faults_and_recovery() {
    use axum::{
        extract::State,
        http::{StatusCode, Uri},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use contracts::generated::alegria::temporal::v1::{
        FinalizePublishInputPayload, PublishArtifact, RenderPreviewValidateOutputPayload,
    };
    use infrastructure::adapters::sqlx_seo_cms_adapter::{
        persist_finalize_publish_output_with_delivery, recover_pending_publication_with_delivery,
    };
    use seo_ports::{
        PublicReleaseDelivery, PublicReleaseFile, ReleaseDeliveryError, ReleaseDeliveryStage,
    };
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct PublicHttpState {
        files: Mutex<HashMap<String, Vec<u8>>>,
        fail_next_read: std::sync::atomic::AtomicBool,
    }

    async fn serve_file(State(state): State<Arc<PublicHttpState>>, uri: Uri) -> impl IntoResponse {
        if state
            .fail_next_read
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return (StatusCode::SERVICE_UNAVAILABLE, Vec::new());
        }
        match state.files.lock().unwrap().get(uri.path()) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Fault {
        Upload,
        Readback,
    }

    struct ControlledDelivery {
        base_url: String,
        http: Arc<PublicHttpState>,
        fault: Mutex<Option<Fault>>,
    }

    impl ControlledDelivery {
        fn set_fault(&self, next: Fault) {
            *self.fault.lock().unwrap() = Some(next);
        }

        fn error(stage: ReleaseDeliveryStage, detail: &str) -> ReleaseDeliveryError {
            ReleaseDeliveryError {
                stage,
                detail: detail.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl PublicReleaseDelivery for ControlledDelivery {
        fn preflight(&self) -> Result<(), ReleaseDeliveryError> {
            Ok(())
        }

        fn deploy(
            &self,
            release: &Path,
            _previous: Option<&Path>,
        ) -> Result<(), ReleaseDeliveryError> {
            if *self.fault.lock().unwrap() == Some(Fault::Upload) {
                *self.fault.lock().unwrap() = None;
                return Err(Self::error(
                    ReleaseDeliveryStage::Upload,
                    "injected upload failure",
                ));
            }
            let mut files = HashMap::new();
            for (path, route) in [
                ("index.html", "/"),
                ("sitemap.xml", "/sitemap.xml"),
                ("robots.txt", "/robots.txt"),
            ] {
                let bytes = std::fs::read(release.join(path))
                    .map_err(|err| Self::error(ReleaseDeliveryStage::Upload, &err.to_string()))?;
                files.insert(route.to_string(), bytes);
            }
            *self.http.files.lock().unwrap() = files;
            Ok(())
        }

        async fn verify(
            &self,
            origin: &str,
            files: &[PublicReleaseFile],
        ) -> Result<(), ReleaseDeliveryError> {
            assert_eq!(origin, "https://staging.alegria.by");
            if *self.fault.lock().unwrap() == Some(Fault::Readback) {
                *self.fault.lock().unwrap() = None;
                self.http
                    .fail_next_read
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let client = reqwest::Client::new();
            for file in files.iter().filter(|file| !file.public_route.is_empty()) {
                let response = client
                    .get(format!("{}{}", self.base_url, file.public_route))
                    .send()
                    .await
                    .map_err(|err| Self::error(ReleaseDeliveryStage::Readback, &err.to_string()))?;
                if response.status() != StatusCode::OK {
                    return Err(Self::error(
                        ReleaseDeliveryStage::Readback,
                        "HTTP readback failed",
                    ));
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|err| Self::error(ReleaseDeliveryStage::Readback, &err.to_string()))?;
                if bytes.as_ref() != file.bytes.as_slice() {
                    return Err(Self::error(
                        ReleaseDeliveryStage::Readback,
                        "public bytes differ",
                    ));
                }
            }
            Ok(())
        }
    }

    async fn revision(
        pool: &sqlx::PgPool,
        root: &Path,
        page: &str,
        brief: &str,
        blueprint: &str,
        ordinal: i32,
    ) -> FinalizePublishInputPayload {
        let draft = format!("{page}-draft-{ordinal}");
        let rev = format!("{page}-revision-{ordinal}");
        let artifact_key = format!("{page}-artifact-{ordinal}");
        let run_id = Uuid::new_v4().to_string();
        let candidate = root.join(format!("candidate-{ordinal}"));
        std::fs::create_dir(&candidate).unwrap();
        std::fs::write(candidate.join("index.html"), format!("release-{ordinal}")).unwrap();
        std::fs::write(candidate.join("sitemap.xml"), b"<sitemap/>").unwrap();
        std::fs::write(candidate.join("robots.txt"), b"User-agent: *\n").unwrap();
        let manifest = serde_json::json!({
            "base_url": "https://staging.alegria.by",
            "page_count": 1,
            "pages": [{
                "page_node_key": page,
                "revision_id": rev,
                "status": "approved",
                "canonical_url_path": "/",
            }],
        });
        std::fs::write(
            candidate.join("alegria-static-manifest.json"),
            manifest.to_string(),
        )
        .unwrap();
        let meta = serde_json::json!({
            "candidate_output_dir": candidate.to_str().unwrap(),
            "public_output_dir": root.join("public").to_str().unwrap(),
            "base_url": "https://staging.alegria.by",
            "run_id": run_id,
            "page_node_key": page,
            "revision_id": rev,
        });
        sqlx::query("INSERT INTO site.page_drafts (page_draft_key,page_node_key,page_brief_key,draft_revision,body_markdown,truth_snapshot_ref) VALUES ($1,$2,$3,$4,'approved content',$5)")
            .bind(&draft).bind(page).bind(brief).bind(ordinal)
            .bind(format!("truth-{ordinal}"))
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.cms_page_revisions (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status) VALUES ($1,$2,$3,$4,'Published page','Published page',$5,'approved')")
            .bind(&rev).bind(page).bind(&draft).bind(brief).bind(blueprint)
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO site.publish_artifacts (artifact_key,page_node_key,revision_id,artifact_uri,manifest_json,status) VALUES ($1,$2,$3,$4,$5,'built_pending_validation')")
            .bind(&artifact_key).bind(page).bind(&rev)
            .bind(candidate.to_str().unwrap()).bind(sqlx::types::Json(meta.clone()))
            .execute(pool).await.unwrap();
        FinalizePublishInputPayload {
            run_id,
            page_node_key: page.to_string(),
            revision_id: rev.clone(),
            render_validation: Some(RenderPreviewValidateOutputPayload {
                verdict: "render_ready".to_string(),
                ..Default::default()
            }),
            publish_artifact: Some(PublishArtifact {
                artifact_key,
                page_node_key: page.to_string(),
                revision_id: rev,
                artifact_type: "static_manifest".to_string(),
                artifact_uri: candidate.to_str().unwrap().to_string(),
                manifest_json: meta.to_string(),
                status: "built_pending_validation".to_string(),
            }),
        }
    }

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let root = std::env::temp_dir().join(format!("alegria-a17-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let _env = crate::env_overrides::EnvOverrideGuard::apply([
        (
            "SEO_STATIC_OUTPUT_DIR".to_string(),
            root.join("public").to_str().unwrap().into(),
        ),
        (
            "SEO_STATIC_RELEASE_ROOT".to_string(),
            root.to_str().unwrap().into(),
        ),
        (
            "SEO_STATIC_BASE_URL".to_string(),
            "https://staging.alegria.by".to_string(),
        ),
    ]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let http = Arc::new(PublicHttpState::default());
    let server_http = http.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            Router::new()
                .route("/{*path}", get(serve_file))
                .route("/", get(serve_file))
                .with_state(server_http),
        )
        .await;
    });
    let delivery = ControlledDelivery {
        base_url: base_url.clone(),
        http,
        fault: Mutex::new(None),
    };

    let marker = Uuid::new_v4().simple().to_string();
    let page = format!("publish-a17-page-{marker}");
    let scope = format!("publish-a17-scope-{marker}");
    let blueprint = format!("publish-a17-blueprint-{marker}");
    let brief = format!("publish-a17-brief-{marker}");
    sqlx::query("INSERT INTO site.site_scopes (scope_signature,locale) VALUES ($1,'ru')")
        .bind(&scope)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) VALUES ($1,'country_hub_page','informational')")
        .bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_nodes (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) VALUES ($1,$2,$3,'country_hub_page','informational','root','/','approved')")
        .bind(&page).bind(&scope).bind(&blueprint).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) VALUES ($1,$2,$3,'First page')")
        .bind(&brief).bind(&page).bind(&blueprint).execute(pool).await.unwrap();
    let first = revision(pool, &root, &page, &brief, &blueprint, 1).await;
    sqlx::query("INSERT INTO site.cms_pages (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status) VALUES ($1,$2,'/','ru','country_hub_page','informational',$3,$4,'approved')")
        .bind(&page).bind(&scope).bind(format!("doc-{marker}")).bind(&first.revision_id)
        .execute(pool).await.unwrap();

    let planned = seo_steps::finalize_publish_step::execute(&first);
    assert_eq!(planned.verdict, "ready_for_release");
    let published =
        persist_finalize_publish_output_with_delivery(pool, &first, &planned, &delivery)
            .await
            .unwrap();
    assert_eq!(published.verdict, "published");
    assert_eq!(
        reqwest::get(&base_url).await.unwrap().text().await.unwrap(),
        "release-1"
    );
    let first_outbox: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='seo_page_published'"
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(first_outbox, 1);

    let second = revision(pool, &root, &page, &brief, &blueprint, 2).await;
    let planned = seo_steps::finalize_publish_step::execute(&second);
    for fault in [Fault::Upload, Fault::Readback] {
        delivery.set_fault(fault);
        let blocked =
            persist_finalize_publish_output_with_delivery(pool, &second, &planned, &delivery)
                .await
                .unwrap();
        assert_eq!(blocked.verdict, "blocked");
        assert!(blocked
            .blocking_reasons
            .contains(&"public_deployment_unverified".to_string()));
        assert_eq!(
            reqwest::get(&base_url).await.unwrap().text().await.unwrap(),
            "release-1"
        );
        let live: (String, String) = sqlx::query_as(
            "SELECT current_revision_id,current_status FROM site.cms_pages WHERE page_node_key=$1",
        )
        .bind(&page)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(live, (first.revision_id.clone(), "published".to_string()));
        // A fresh validation of the same immutable candidate makes a retry possible.
        sqlx::query("UPDATE site.publish_artifacts SET status='built_pending_validation' WHERE artifact_key=$1")
            .bind(second.publish_artifact.as_ref().unwrap().artifact_key.as_str())
            .execute(pool).await.unwrap();
        sqlx::query(
            "UPDATE site.cms_page_revisions SET revision_status='approved' WHERE revision_id=$1",
        )
        .bind(&second.revision_id)
        .execute(pool)
        .await
        .unwrap();
    }
    let published =
        persist_finalize_publish_output_with_delivery(pool, &second, &planned, &delivery)
            .await
            .unwrap();
    assert_eq!(published.verdict, "published");
    assert_eq!(
        reqwest::get(&base_url).await.unwrap().text().await.unwrap(),
        "release-2"
    );
    let pointers: (String, String) = sqlx::query_as(
        "SELECT current_revision_id,rollback_revision_id FROM site.cms_pages WHERE page_node_key=$1"
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(
        pointers,
        (second.revision_id.clone(), first.revision_id.clone())
    );
    let outbox_before: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='seo_page_published'"
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(outbox_before, 2);
    // A lost acknowledgement must not produce a blocked event or duplicate effect.
    let replay = persist_finalize_publish_output_with_delivery(pool, &second, &planned, &delivery)
        .await
        .unwrap();
    assert_eq!(replay.verdict, "published");
    let outbox_after: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM system.sync_outbox WHERE aggregate_key=$1 AND event_type='seo_page_published'"
    ).bind(&page).fetch_one(pool).await.unwrap();
    assert_eq!(outbox_after, outbox_before);

    // Simulate process death after external switch, before committing the
    // third release in PostgreSQL. Startup must restore the second release.
    let committed_release = std::fs::read_link(root.join("public")).unwrap();
    let uncommitted_release = root.join("crash-published");
    std::fs::create_dir(&uncommitted_release).unwrap();
    for (name, bytes) in [
        ("index.html", b"uncommitted".as_slice()),
        ("sitemap.xml", b"<sitemap/>".as_slice()),
        ("robots.txt", b"User-agent: *\n".as_slice()),
    ] {
        std::fs::write(uncommitted_release.join(name), bytes).unwrap();
    }
    delivery.deploy(&uncommitted_release, None).unwrap();
    let next_pointer = root.join(".a17-switch");
    std::os::unix::fs::symlink(&uncommitted_release, &next_pointer).unwrap();
    std::fs::rename(&next_pointer, root.join("public")).unwrap();
    std::fs::write(
        root.join(".alegria-pending-publication.json"),
        serde_json::json!({
            "new_release": uncommitted_release,
            "previous": committed_release.clone(),
            "page_node_key": page,
            "revision_id": "uncommitted-third-revision",
            "artifact_key": "uncommitted-third-artifact",
        })
        .to_string(),
    )
    .unwrap();
    recover_pending_publication_with_delivery(pool, &delivery)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_link(root.join("public")).unwrap(),
        committed_release
    );
    assert_eq!(
        reqwest::get(&base_url).await.unwrap().text().await.unwrap(),
        "release-2"
    );
    assert!(!root.join(".alegria-pending-publication.json").exists());
    server.abort();
    std::fs::remove_dir_all(&root).unwrap();
}
