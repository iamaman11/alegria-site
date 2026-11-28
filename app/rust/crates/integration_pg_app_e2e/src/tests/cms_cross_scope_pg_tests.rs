/// A revision can approve a link into another published scope without
/// silently publishing that scope's pages, sitemap entries or navigation.
/// External targets are metadata only; losing their published revision must
/// fail the candidate build closed instead of dropping a mandatory link.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn cms_revision_cross_scope_published_link_is_not_a_release_page() {
    use infrastructure::adapters::{sqlx_seo_adapter, static_site_builder_adapter};

    let pg = PostgresHarness::start().await.unwrap();
    let pool = &pg.pool;
    let marker = Uuid::new_v4().simple().to_string();
    let blueprint = format!("cross-scope-blueprint-{marker}");
    let source_scope = format!("cross-scope-source-{marker}");
    let target_scope = format!("cross-scope-foreign-{marker}");
    let source_page = format!("cross-scope-approved-{marker}");
    let target_page = format!("cross-scope-published-{marker}");
    let source_revision = format!("cross-scope-approved-revision-{marker}");
    let target_revision = format!("cross-scope-published-revision-{marker}");
    let source_path = format!("/ru/{marker}/home/");
    let target_path = format!("/en/{marker}/foreign/");
    sqlx::query(
        "INSERT INTO site.page_blueprints (blueprint_key,page_type_key,dominant_intent) \
         VALUES ($1,'country_hub_page','informational')",
    )
    .bind(&blueprint)
    .execute(pool)
    .await
    .unwrap();
    for (scope, market, locale, page, path, revision, status) in [
        (
            &source_scope, "cross-a", "ru", &source_page,
            &source_path, &source_revision, "approved",
        ),
        (
            &target_scope, "cross-b", "en", &target_page,
            &target_path, &target_revision, "published",
        ),
    ] {
        let brief = format!("{page}-brief");
        let draft = format!("{page}-draft");
        sqlx::query(
            "INSERT INTO site.site_scopes (scope_signature,market,locale) VALUES ($1,$2,$3)",
        )
        .bind(scope)
        .bind(market)
        .bind(locale)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO site.page_nodes \
             (page_node_key,scope_signature,blueprint_key,page_type_key,dominant_intent,canonical_slug,canonical_url_path,lifecycle_state) \
             VALUES ($1,$2,$3,'country_hub_page','informational','landing',$4,$5)",
        )
        .bind(page)
        .bind(scope)
        .bind(&blueprint)
        .bind(path)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO site.page_briefs (page_brief_key,page_node_key,blueprint_key,title) \
             VALUES ($1,$2,$3,'Cross-scope editorial brief')",
        )
        .bind(&brief)
        .bind(page)
        .bind(&blueprint)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO site.page_drafts \
             (page_draft_key,page_node_key,page_brief_key,body_markdown,qa_verdict) \
             VALUES ($1,$2,$3,'Editorial body','publish_ready')",
        )
        .bind(&draft)
        .bind(page)
        .bind(&brief)
        .execute(pool)
        .await
        .unwrap();
        let approved_links = if page == &source_page {
            serde_json::json!([{
                "target_page_key": target_page,
                "canonical_url_path": target_path,
                "label": "Published foreign destination",
                "link_role": "contextual"
            }])
        } else {
            serde_json::json!([])
        };
        sqlx::query(
            "INSERT INTO site.cms_page_revisions \
             (revision_id,page_node_key,page_draft_key,page_brief_key,title,h1,blueprint_key,revision_status,body_payload,required_link_payload) \
             VALUES ($1,$2,$3,$4,'Cross-scope source','Cross-scope source',$5,$6,$7,$8)",
        )
        .bind(revision)
        .bind(page)
        .bind(&draft)
        .bind(&brief)
        .bind(&blueprint)
        .bind(status)
        .bind(serde_json::json!({"markdown":"Editorial body"}))
        .bind(approved_links)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO site.cms_pages \
             (page_node_key,scope_signature,canonical_url_path,locale_code,page_type_key,dominant_intent_key,cms_document_id,current_revision_id,current_status) \
             VALUES ($1,$2,$3,$4,'country_hub_page','informational',$5,$6,$7)",
        )
        .bind(page)
        .bind(scope)
        .bind(path)
        .bind(locale)
        .bind(format!("cross-scope-doc-{page}"))
        .bind(if status == "published" { Some(revision.as_str()) } else { None })
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
        sqlx_seo_adapter::persist_global_navigation_from_active_pages(pool, market, locale)
            .await
            .unwrap();
    }

    let static_dir = std::env::temp_dir().join(format!("cms-cross-scope-{marker}"));
    let built = static_site_builder_adapter::build_static_site_candidate_incremental(
        pool, &static_dir, "https://example.test", &source_page, &source_revision,
    )
    .await
    .unwrap();
    assert_eq!(built.previews.len(), 1);
    let source_html = &built.previews[0].rendered_html;
    let href = format!("href=\"{}\"", target_path.trim_end_matches('/'));
    assert_eq!(source_html.matches(&href).count(), 1, "cross-scope link was lost or leaked into menu");
    assert!(source_html.contains("Published foreign destination"));
    assert_eq!(
        (built.previews[0].required_link_count, built.previews[0].rendered_link_count),
        (1, 1)
    );
    let foreign_html_path = format!("en/{marker}/foreign/index.html");
    assert!(
        built.artifacts.iter().all(|artifact| artifact.relative_path != foreign_html_path),
        "a linked foreign scope must not become an emitted release page"
    );
    for release_manifest in ["sitemap.xml", "alegria-static-manifest.json"] {
        let bytes = &built.artifacts.iter()
            .find(|artifact| artifact.relative_path == release_manifest)
            .expect("generated release manifest")
            .bytes;
        assert!(
            !String::from_utf8_lossy(bytes).contains(&target_path),
            "foreign target leaked into {release_manifest}"
        );
    }

    sqlx::query("UPDATE site.cms_pages SET current_status='deprecated' WHERE page_node_key=$1")
        .bind(&target_page)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        static_site_builder_adapter::build_static_site_candidate_incremental(
            pool, &static_dir, "https://example.test", &source_page, &source_revision,
        )
        .await
        .is_err(),
        "an approved cross-scope link to an unpublished target must block"
    );
}
