// Re-render existing published CMS revisions when only the static renderer
// changes. Content and approval records remain immutable: this is a release
// operation, not another editorial publication.

const RENDER_REFRESH_OWNER: &str = "__published_renderer_refresh__";
const RENDER_REFRESH_JOURNAL: &str = ".alegria-pending-render-refresh.json";

fn published_renderer_files(
    directory: &ReleasePath,
    expected_origin: &str,
    expected_builder: &str,
) -> Result<Vec<ReleaseArtifact>, String> {
    let original = release_file(directory, "alegria-static-manifest.json", "")?;
    let manifest: serde_json::Value = serde_json::from_slice(&original.bytes)
        .map_err(|err| format!("renderer manifest JSON: {err}"))?;
    if manifest.get("base_url").and_then(|v| v.as_str()) != Some(expected_origin)
        || manifest.get("builder").and_then(|v| v.as_str()) != Some(expected_builder)
    {
        return Err("renderer version or public origin differs from expected release".into());
    }
    let pages = manifest.get("pages").and_then(|v| v.as_array())
        .ok_or("renderer manifest pages missing")?;
    if pages.is_empty() || manifest.get("page_count").and_then(|v| v.as_u64()) != Some(pages.len() as u64) {
        return Err("renderer manifest must list existing published pages".into());
    }
    let mut paths = std::collections::HashSet::new();
    let mut artifacts = Vec::new();
    for page in pages {
        if page.get("status").and_then(|v| v.as_str()) != Some("published")
            || page.get("page_node_key").and_then(|v| v.as_str()).is_none_or(str::is_empty)
            || page.get("revision_id").and_then(|v| v.as_str()).is_none_or(str::is_empty)
        {
            return Err("renderer refresh requires previously published CMS revisions".into());
        }
        let route = page.get("canonical_url_path").and_then(|v| v.as_str())
            .ok_or("published page has no canonical path")?;
        if !route.starts_with('/') || route.contains("//") || route.contains('\\')
            || route.contains('?') || route.contains('#')
            || route.split('/').any(|part| part == "." || part == "..")
        {
            return Err("renderer refresh has unsafe public route".into());
        }
        let relative = if route == "/" {
            "index.html".to_string()
        } else {
            format!("{}/index.html", route.trim_matches('/'))
        };
        if !paths.insert(relative.clone()) {
            return Err("renderer refresh has duplicate public routes".into());
        }
        artifacts.push(release_file(directory, &relative, route)?);
    }
    for (relative, route) in [("sitemap.xml", "/sitemap.xml"), ("robots.txt", "/robots.txt")] {
        artifacts.push(release_file(directory, relative, route)?);
    }
    artifacts.push(original);
    Ok(artifacts)
}

// A renderer-only release changes bytes, not the editorial decision. Keep the
// original approved Temporal run and page revision as the content provenance;
// the native first-site release doctor must continue proving that identity.
fn keep_published_content_provenance(
    artifacts: &mut [ReleaseArtifact],
    original: &serde_json::Value,
) -> Result<(), String> {
    let run_id = original.get("publication_run_id").and_then(|v| v.as_str())
        .ok_or("published release has no canonical publication run")?;
    let canonical = uuid::Uuid::parse_str(run_id)
        .map_err(|_| "published release has invalid publication run")?.to_string();
    if canonical != run_id {
        return Err("published release has noncanonical publication run".into());
    }
    let page_key = original.get("publication_page_node_key").and_then(|v| v.as_str())
        .filter(|value| !value.is_empty())
        .ok_or("published release has no origin page")?;
    let revision = original.get("publication_revision_id").and_then(|v| v.as_str())
        .filter(|value| !value.is_empty())
        .ok_or("published release has no origin revision")?;
    let file = artifacts.iter_mut()
        .find(|file| file.relative_path == "alegria-static-manifest.json")
        .ok_or("renderer release has no manifest")?;
    let mut manifest: serde_json::Value = serde_json::from_slice(&file.bytes)
        .map_err(|err| format!("renderer manifest JSON: {err}"))?;
    let matching = manifest.get("pages").and_then(|v| v.as_array())
        .ok_or("renderer release pages missing")?
        .iter()
        .filter(|page| page.get("page_node_key").and_then(|v| v.as_str()) == Some(page_key)
            && page.get("revision_id").and_then(|v| v.as_str()) == Some(revision)
            && page.get("status").and_then(|v| v.as_str()) == Some("published"))
        .count();
    if matching != 1 {
        return Err("renderer release lost its originally published CMS revision".into());
    }
    manifest["publication_run_id"] = serde_json::json!(run_id);
    manifest["publication_page_node_key"] = serde_json::json!(page_key);
    manifest["publication_revision_id"] = serde_json::json!(revision);
    manifest["renderer_refresh_version"] =
        serde_json::json!(static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION);
    file.bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| format!("serialize renderer provenance: {err}"))?;
    Ok(())
}

#[cfg(unix)]
fn renderer_refresh_journal(public: &ReleasePath) -> Result<ReleasePathBuf, String> {
    Ok(public.parent().ok_or("public static path has no parent")?.join(RENDER_REFRESH_JOURNAL))
}

#[cfg(unix)]
fn managed_previous_release(
    public: &ReleasePath,
    root: &ReleasePath,
) -> Result<ReleasePathBuf, String> {
    let previous = prepare_public_pointer(public, false)?
        .ok_or("renderer refresh requires a previously published release")?;
    let canonical = std::fs::canonicalize(&previous)
        .map_err(|err| format!("resolve previous immutable release: {err}"))?;
    if canonical.parent() != Some(root) || !canonical
        .file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with("-published"))
    {
        return Err("previous release is not an immediate managed release-root child".into());
    }
    Ok(canonical)
}

#[cfg(unix)]
pub async fn recover_pending_renderer_refresh(pool: &PgPool) -> Result<(), DomainError> {
    recover_pending_renderer_refresh_with_delivery(pool, &CloudflareStaticDelivery).await
}

#[cfg(unix)]
pub async fn recover_pending_renderer_refresh_with_delivery(
    pool: &PgPool,
    delivery: &dyn PublicReleaseDelivery,
) -> Result<(), DomainError> {
    let Some(public_raw) = std::env::var("SEO_STATIC_OUTPUT_DIR").ok() else {
        return Ok(());
    };
    let public = ReleasePathBuf::from(public_raw);
    let journal_path = renderer_refresh_journal(&public).map_err(validation_failure)?;
    if !journal_path.exists() {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(21917, hashtext($1))")
        .bind(public.to_string_lossy().as_ref())
        .execute(&mut *tx).await.map_err(classify_sqlx)?;
    if !journal_path.exists() {
        return tx.commit().await.map_err(classify_sqlx);
    }
    let pending = read_pending_publication(&journal_path).map_err(validation_failure)?;
    if pending.page_node_key != RENDER_REFRESH_OWNER {
        return Err(validation_failure("pending renderer journal has invalid owner"));
    }
    let previous = pending.previous.as_deref()
        .ok_or_else(|| validation_failure("renderer refresh cannot recover without prior release"))?;
    let root = std::fs::canonicalize(std::env::var("SEO_STATIC_RELEASE_ROOT")
        .map_err(|_| validation_failure("missing renderer release root"))?)
        .map_err(|err| validation_failure(format!("renderer release root: {err}")))?;
    let previous = std::fs::canonicalize(previous)
        .map_err(|err| validation_failure(format!("prior renderer release: {err}")))?;
    let updated = std::fs::canonicalize(&pending.new_release)
        .map_err(|err| validation_failure(format!("new renderer release: {err}")))?;
    if previous.parent() != Some(root.as_path()) || updated.parent() != Some(root.as_path())
        || updated != pending.new_release
    {
        return Err(validation_failure("renderer journal escaped immutable release root"));
    }
    let origin_raw = std::env::var("SEO_STATIC_BASE_URL")
        .map_err(|_| validation_failure("missing renderer origin"))?;
    publication_origin(&origin_raw).map_err(validation_failure)?;
    let expected = published_renderer_files(&updated, &origin_raw, &pending.revision_id)
        .map_err(validation_failure)?;
    verify_existing_immutable_release(&updated, &expected).map_err(validation_failure)?;
    let live = std::fs::read_link(&public)
        .map_err(|err| validation_failure(format!("read renderer public pointer: {err}")))?;
    if live != updated && live != previous {
        return Err(validation_failure("renderer refresh has unknown public pointer"));
    }
    delivery.preflight().map_err(|err| validation_failure(err.to_string()))?;
    // No DB content was ever changed. A pending renderer refresh is always
    // uncommitted: restore the last published release and retry on startup.
    delivery.deploy(&previous, None).map_err(|err| validation_failure(err.to_string()))?;
    if live == updated {
        restore_public_release(&public, Some(&previous)).map_err(validation_failure)?;
    }
    std::fs::remove_file(&journal_path)
        .map_err(|err| validation_failure(format!("clear renderer refresh journal: {err}")))?;
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(())
}

/// One owner for navigation signals and the resulting published HTML refresh.
/// LISTEN is the prompt path; the bounded poll also catches missed events on
/// disconnect or restart. The production Worker supplies Cloudflare delivery,
/// while integration tests use the existing delivery port to verify real HTML.
#[cfg(unix)]
pub async fn run_published_navigation_refresh_loop(
    pool: std::sync::Arc<PgPool>,
    database_url: String,
    delivery: std::sync::Arc<dyn PublicReleaseDelivery>,
) {
    use sqlx::postgres::PgListener;
    use tokio::time::{timeout, Duration};

    const FALLBACK: Duration = Duration::from_secs(60);
    let mut listener: Option<PgListener> = None;
    loop {
        if listener.is_none() {
            match PgListener::connect(&database_url).await {
                Ok(mut connection) => {
                    match connection
                        .listen(super::sqlx_seo_adapter::NAVIGATION_NOTIFY_CHANNEL)
                        .await
                    {
                        Ok(()) => listener = Some(connection),
                        Err(err) => tracing::warn!(reason = %err, "navigation LISTEN unavailable"),
                    }
                }
                Err(err) => tracing::warn!(reason = %err, "navigation LISTEN disconnected"),
            }
        }

        match refresh_published_renderer_if_stale_with_delivery(&pool, delivery.as_ref()).await {
            Ok(true) => {
                tracing::info!("published static HTML refreshed for navigation or renderer change")
            }
            Ok(false) => {}
            Err(err) => tracing::warn!(reason = %err, "published static HTML refresh deferred"),
        }
        if let Some(connection) = listener.as_mut() {
            match timeout(FALLBACK, connection.recv()).await {
                Ok(Ok(_)) | Err(_) => {}
                Ok(Err(err)) => {
                    tracing::warn!(reason = %err, "navigation notification lost; reconnecting");
                    listener = None;
                }
            }
        } else {
            tokio::time::sleep(FALLBACK).await;
        }
    }
}

#[cfg(unix)]
pub async fn refresh_published_renderer_if_stale_with_delivery(
    pool: &PgPool,
    delivery: &dyn PublicReleaseDelivery,
) -> Result<bool, DomainError> {
    let (Ok(public_raw), Ok(origin_raw), Ok(release_root_raw)) = (
        std::env::var("SEO_STATIC_OUTPUT_DIR"),
        std::env::var("SEO_STATIC_BASE_URL"),
        std::env::var("SEO_STATIC_RELEASE_ROOT"),
    ) else {
        return Ok(false);
    };
    let public = ReleasePathBuf::from(public_raw);
    if !public.exists() {
        return Ok(false); // No public site has been published yet.
    }
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(21917, hashtext($1))")
        .bind(public.to_string_lossy().as_ref())
        .execute(&mut *tx).await.map_err(classify_sqlx)?;
    let pending = pending_publication_path(&public).map_err(validation_failure)?;
    let refresh_journal = renderer_refresh_journal(&public).map_err(validation_failure)?;
    if pending.exists() || refresh_journal.exists() {
        return Err(validation_failure("existing publication must be reconciled before renderer refresh"));
    }
    let current_manifest = std::fs::read(public.join("alegria-static-manifest.json"))
        .map_err(|err| validation_failure(format!("read currently published manifest: {err}")))?;
    let current: serde_json::Value = serde_json::from_slice(&current_manifest)
        .map_err(|err| validation_failure(format!("parse published manifest: {err}")))?;
    let origin_page = current.get("publication_page_node_key").and_then(|v| v.as_str())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| validation_failure("published release lacks CMS origin page"))?;
    let scope_signature: String = sqlx::query_scalar(
        "SELECT scope_signature FROM site.cms_pages WHERE page_node_key=$1",
    )
    .bind(origin_page)
    .fetch_one(&mut *tx)
    .await
    .map_err(classify_sqlx)?;
    let navigation = super::sqlx_static_site_adapter::published_navigation_fingerprint_in_tx(
        &mut tx, &scope_signature,
    ).await?;
    if current.get("builder").and_then(|v| v.as_str())
        == Some(static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION)
        && current.get("navigation_fingerprint").and_then(|v| v.as_str())
            == Some(navigation.as_str())
    {
        return Ok(false);
    }
    delivery.preflight().map_err(|err| validation_failure(err.to_string()))?;
    let origin = publication_origin(&origin_raw).map_err(validation_failure)?;
    let root = std::fs::canonicalize(release_root_raw)
        .map_err(|err| validation_failure(format!("renderer release root: {err}")))?;
    let previous = managed_previous_release(&public, &root).map_err(validation_failure)?;
    let candidate = root.join(format!("renderer-{}", uuid::Uuid::new_v4().simple()));
    static_site_builder_adapter::build_static_site_for_scope(
        pool, &candidate, &origin_raw, &scope_signature,
    )
        .await.map_err(|err| validation_failure(format!("build published renderer candidate: {err}")))?;
    let mut files = published_renderer_files(
        &candidate, &origin_raw, static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION,
    ).map_err(validation_failure)?;
    let candidate_navigation = files.iter()
        .find(|file| file.relative_path == "alegria-static-manifest.json")
        .and_then(|file| serde_json::from_slice::<serde_json::Value>(&file.bytes).ok())
        .and_then(|manifest| manifest.get("navigation_fingerprint").and_then(|v| v.as_str()).map(str::to_string))
        .ok_or_else(|| validation_failure("renderer candidate lacks navigation fingerprint"))?;
    let current_navigation = super::sqlx_static_site_adapter::published_navigation_fingerprint_in_tx(
        &mut tx, &scope_signature,
    ).await?;
    if candidate_navigation != current_navigation {
        return Err(validation_failure("navigation changed during renderer candidate build"));
    }
    keep_published_content_provenance(&mut files, &current).map_err(validation_failure)?;
    let release = create_immutable_release(&candidate, &files).map_err(validation_failure)?;
    write_pending_publication(&refresh_journal, &PendingPublication {
        new_release: release.clone(),
        previous: Some(previous.clone()),
        page_node_key: RENDER_REFRESH_OWNER.to_string(),
        revision_id: static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION.to_string(),
        artifact_key: uuid::Uuid::new_v4().to_string(),
    }).map_err(validation_failure)?;
    if let Err(err) = swap_public_release(&public, &release) {
        let _ = std::fs::remove_file(&refresh_journal);
        return Err(validation_failure(err));
    }
    let mut activation = PublicationActivation {
        delivery,
        public_path: public,
        previous: Some(previous.clone()),
        journal_path: refresh_journal,
        // Upload may succeed before reporting an error: always restore the
        // previous Cloudflare release when this guarded operation fails.
        remote_activated: true,
        armed: true,
    };
    delivery.deploy(&release, Some(&previous))
        .map_err(|err| validation_failure(err.to_string()))?;
    delivery.verify(origin.as_str(), &files).await
        .map_err(|err| validation_failure(err.to_string()))?;
    tx.commit().await.map_err(classify_sqlx)?;
    activation.confirm();
    Ok(true)
}

#[cfg(test)]
mod renderer_refresh_tests {
    use super::*;

    #[test]
    fn refresh_accepts_only_existing_published_revisions() {
        let dir = std::env::temp_dir().join(format!("alegria-render-refresh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("ru/visa/spain")).unwrap();
        std::fs::write(dir.join("ru/visa/spain/index.html"), "published page").unwrap();
        std::fs::write(dir.join("sitemap.xml"), "sitemap").unwrap();
        std::fs::write(dir.join("robots.txt"), "robots").unwrap();
        let mut manifest = serde_json::json!({
            "builder": static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION,
            "base_url": "https://staging.alegria.by",
            "page_count": 1,
            "pages": [{
                "page_node_key": "visa", "revision_id": "revision",
                "canonical_url_path": "/ru/visa/spain/", "status": "published"
            }]
        });
        let file = dir.join("alegria-static-manifest.json");
        std::fs::write(&file, manifest.to_string()).unwrap();
        let files = published_renderer_files(
            &dir, "https://staging.alegria.by", static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION,
        ).unwrap();
        assert_eq!(files.len(), 4);
        let provenance = serde_json::json!({
            "publication_run_id": "f628a0b7-c405-4cce-8b3b-a95d0ac7db6c",
            "publication_page_node_key": "visa",
            "publication_revision_id": "revision",
        });
        let mut refreshed = files;
        keep_published_content_provenance(&mut refreshed, &provenance).unwrap();
        let refreshed_manifest: serde_json::Value = serde_json::from_slice(
            &refreshed.iter().find(|file| file.relative_path == "alegria-static-manifest.json")
                .unwrap().bytes,
        ).unwrap();
        assert_eq!(refreshed_manifest["publication_run_id"], provenance["publication_run_id"]);
        assert_eq!(refreshed_manifest["publication_page_node_key"], "visa");
        assert_eq!(refreshed_manifest["publication_revision_id"], "revision");
        assert_eq!(refreshed_manifest["renderer_refresh_version"],
            static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION);
        let wrong_revision = serde_json::json!({
            "publication_run_id": provenance["publication_run_id"],
            "publication_page_node_key": "visa",
            "publication_revision_id": "different-revision",
        });
        assert!(keep_published_content_provenance(&mut refreshed, &wrong_revision).is_err());
        manifest["pages"][0]["status"] = "approved".into();
        std::fs::write(&file, manifest.to_string()).unwrap();
        assert!(published_renderer_files(
            &dir, "https://staging.alegria.by", static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION,
        ).is_err());
        manifest["pages"][0]["status"] = "published".into();
        manifest["pages"][0]["canonical_url_path"] = "/../../escape".into();
        std::fs::write(&file, manifest.to_string()).unwrap();
        assert!(published_renderer_files(
            &dir, "https://staging.alegria.by", static_site_builder_adapter::STATIC_SITE_BUILDER_VERSION,
        ).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
