// Atomic local static release promotion with independent public HTTPS proof.
// The web server must serve SEO_STATIC_OUTPUT_DIR; no separate deploy runner.
use std::path::{Path as ReleasePath, PathBuf as ReleasePathBuf};

#[derive(Debug)]
struct ReleaseArtifact {
    relative_path: String,
    public_route: String,
    bytes: Vec<u8>,
}

fn publication_origin(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|err| format!("invalid origin: {err}"))?;
    let host = url.host_str().ok_or("origin has no host")?.to_ascii_lowercase();
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || host == "localhost"
        || host == "example.com"
        || host.ends_with(".example")
        || host.ends_with(".test")
        || host.ends_with(".invalid")
        || host.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return Err("public origin must be an external HTTPS hostname without userinfo or path".into());
    }
    Ok(url)
}

fn publication_files(
    candidate: &ReleasePath,
    page_node_key: &str,
    revision_id: &str,
    expected_origin: &str,
) -> Result<Vec<ReleaseArtifact>, String> {
    let manifest_path = candidate.join("alegria-static-manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&manifest_path).map_err(|err| format!("candidate manifest: {err}"))?,
    )
    .map_err(|err| format!("candidate manifest JSON: {err}"))?;
    if manifest.get("base_url").and_then(|v| v.as_str()) != Some(expected_origin) {
        return Err("candidate manifest origin differs from configured origin".into());
    }
    let pages = manifest.get_mut("pages")
        .and_then(|v| v.as_array_mut())
        .ok_or("candidate manifest pages missing")?;
    if pages.is_empty() {
        return Err("candidate release has no pages".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut target_count = 0;
    let mut artifacts = Vec::new();
    for page in pages.iter_mut() {
        let key = page.get("page_node_key").and_then(|v| v.as_str()).unwrap_or("");
        let revision = page.get("revision_id").and_then(|v| v.as_str()).unwrap_or("");
        let status = page.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let is_target = key == page_node_key && revision == revision_id;
        if is_target {
            if status != "approved" {
                return Err("target candidate must have persisted human approval".into());
            }
            target_count += 1;
            page["status"] = serde_json::Value::String("published".to_string());
        } else if status != "published" {
            return Err("candidate includes non-published dependency".into());
        }
        let route = page.get("canonical_url_path")
            .and_then(|v| v.as_str()).ok_or("page has no canonical path")?;
        if !route.starts_with('/')
            || route.contains("//")
            || route.contains('\\')
            || route.contains('?')
            || route.contains('#')
            || route.split('/').any(|part| part == "." || part == "..")
        {
            return Err("candidate has unsafe public route".into());
        }
        let rel = if route == "/" {
            "index.html".to_string()
        } else {
            format!("{}/index.html", route.trim_matches('/'))
        };
        if !seen.insert(rel.clone()) {
            return Err("candidate has duplicate page routes".into());
        }
        artifacts.push(release_file(candidate, &rel, route)?);
    }
    if target_count != 1 {
        return Err("candidate manifest must contain exactly one approved target revision".into());
    }
    let actual_page_count = pages.len() as u64;
    if manifest.get("page_count").and_then(|v| v.as_u64()) != Some(actual_page_count) {
        return Err("candidate manifest page_count mismatch".into());
    }
    for (rel, route) in [("sitemap.xml", "/sitemap.xml"), ("robots.txt", "/robots.txt")] {
        artifacts.push(release_file(candidate, rel, route)?);
    }
    artifacts.push(ReleaseArtifact {
        relative_path: "alegria-static-manifest.json".to_string(),
        public_route: String::new(),
        bytes: serde_json::to_vec_pretty(&manifest)
            .map_err(|err| format!("serialize public manifest: {err}"))?,
    });
    Ok(artifacts)
}

fn release_file(
    candidate: &ReleasePath,
    relative_path: &str,
    route: &str,
) -> Result<ReleaseArtifact, String> {
    let file = candidate.join(relative_path);
    let meta = std::fs::symlink_metadata(&file)
        .map_err(|err| format!("missing candidate artifact {relative_path}: {err}"))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(format!("candidate artifact is not a regular file: {relative_path}"));
    }
    Ok(ReleaseArtifact {
        relative_path: relative_path.to_string(),
        public_route: route.to_string(),
        bytes: std::fs::read(file).map_err(|err| format!("read candidate artifact: {err}"))?,
    })
}

fn create_immutable_release(
    candidate: &ReleasePath,
    files: &[ReleaseArtifact],
) -> Result<ReleasePathBuf, String> {
    let base = candidate.parent().ok_or("candidate has no release root")?;
    let release_name = format!(
        "{}-published",
        candidate.file_name().and_then(|n| n.to_str()).ok_or("invalid candidate release id")?
    );
    let release = base.join(&release_name);
    if release.exists() {
        for file in files {
            if std::fs::read(release.join(&file.relative_path)).ok().as_deref()
                != Some(file.bytes.as_slice())
            {
                return Err("existing immutable release differs from approved candidate".into());
            }
        }
        return Ok(release);
    }
    let stage = base.join(format!(".{release_name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&stage).map_err(|err| format!("create release stage: {err}"))?;
    let result = (|| -> Result<(), String> {
        for file in files {
            let out = stage.join(&file.relative_path);
            std::fs::create_dir_all(out.parent().ok_or("invalid staged path")?)
                .map_err(|err| format!("stage parent: {err}"))?;
            std::fs::write(out, &file.bytes).map_err(|err| format!("stage file: {err}"))?;
        }
        std::fs::write(stage.join(".alegria_static_site"), b"alegria_immutable_release@1\n")
            .map_err(|err| format!("write release marker: {err}"))?;
        std::fs::rename(&stage, &release)
            .map_err(|err| format!("commit immutable release: {err}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result?;
    Ok(release)
}

#[cfg(unix)]
fn swap_public_release(
    public: &ReleasePath,
    release: &ReleasePath,
) -> Result<Option<ReleasePathBuf>, String> {
    use std::os::unix::fs::symlink;
    let previous = match std::fs::symlink_metadata(public) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Some(std::fs::read_link(public).map_err(|err| format!("read live pointer: {err}"))?)
        }
        Ok(_) => return Err("public output path must be an Alegria-managed symlink, not an existing directory".into()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(format!("check public output path: {err}")),
    };
    let parent = public.parent().ok_or("public path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|err| format!("create public parent: {err}"))?;
    let temp = parent.join(format!(".alegria-live-{}", uuid::Uuid::new_v4()));
    symlink(release, &temp).map_err(|err| format!("create live link: {err}"))?;
    if let Err(err) = std::fs::rename(&temp, public) {
        let _ = std::fs::remove_file(temp);
        return Err(format!("activate live release: {err}"));
    }
    Ok(previous)
}

#[cfg(unix)]
#[cfg(unix)]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PendingPublication {
    new_release: ReleasePathBuf,
    previous: Option<ReleasePathBuf>,
    page_node_key: String,
    revision_id: String,
    artifact_key: String,
}

#[cfg(unix)]
fn pending_publication_path(public: &ReleasePath) -> Result<ReleasePathBuf, String> {
    Ok(public.parent().ok_or("public output path has no parent")?
        .join(".alegria-pending-publication.json"))
}

#[cfg(unix)]
fn write_pending_publication(
    path: &ReleasePath,
    pending: &PendingPublication,
) -> Result<(), String> {
    use std::io::Write;
    let content = serde_json::to_vec(pending)
        .map_err(|err| format!("serialize pending publication: {err}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("pending publication must first be reconciled: {err}"))?;
    file.write_all(&content)
        .and_then(|_| file.sync_all())
        .map_err(|err| format!("persist pending publication: {err}"))?;
    Ok(())
}

#[cfg(unix)]
fn deploy_cloudflare_static_release(
    release: &ReleasePath,
    previous: Option<&ReleasePath>,
) -> Result<(), String> {
    // Deployment is a synchronous critical section under the PostgreSQL
    // advisory lock: cancellation must not leave a detached upload running.
    // The adapter does not accept raw HTML or a candidate directory.
    let release_root = std::env::var("SEO_STATIC_RELEASE_ROOT")
        .map_err(|_| "SEO_STATIC_RELEASE_ROOT must be configured for Cloudflare publication")?;
    let script = std::env::var("SEO_STATIC_DEPLOY_SCRIPT")
        .unwrap_or_else(|_| "/app/automation/deploy_seo_static_cloudflare.py".into());
    let mut command = std::process::Command::new("python3");
    command.arg(script).arg(release)
        .arg("--release-root").arg(release_root).arg("--deploy");
    if let Some(old) = previous {
        command.arg("--previous-release").arg(old);
    }
    let result = command.output().map_err(|err| format!("start Cloudflare deployment: {err}"))?;
    if !result.status.success() {
        // The CLI's raw stderr may contain sensitive provider/account metadata.
        return Err(format!(
            "Cloudflare deployment/readback failed with status {}; inspect worker logs",
            result.status
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn require_cloudflare_publication_backend() -> Result<(), String> {
    if std::env::var("SEO_STATIC_PUBLISH_BACKEND").ok().as_deref()
        != Some("cloudflare_workers_static_assets")
    {
        return Err("SEO_STATIC_PUBLISH_BACKEND must be cloudflare_workers_static_assets".into());
    }
    Ok(())
}

#[cfg(unix)]
pub async fn recover_pending_publication(pool: &PgPool) -> Result<(), DomainError> {
    let public = ReleasePathBuf::from(std::env::var("SEO_STATIC_OUTPUT_DIR")
        .unwrap_or_else(|_| "app/rust/dist/static-site".to_string()));
    let journal_path = pending_publication_path(&public).map_err(validation_failure)?;
    if !journal_path.exists() {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(21917, hashtext($1))")
        .bind(public.to_string_lossy().as_ref())
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    if !journal_path.exists() {
        tx.commit().await.map_err(classify_sqlx)?;
        return Ok(());
    }
    let journal: PendingPublication = serde_json::from_slice(
        &std::fs::read(&journal_path)
            .map_err(|err| validation_failure(format!("read pending publication: {err}")))?,
    ).map_err(|err| validation_failure(format!("parse pending publication: {err}")))?;
    // Both database state and the file pointer must agree. Never infer
    // commitment from the presence of a ready-looking release directory.
    let committed: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
          SELECT 1 FROM site.cms_pages p
          JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
          JOIN site.publish_artifacts a ON a.revision_id = r.revision_id
          WHERE p.page_node_key = $1 AND p.current_revision_id = $2
            AND p.current_status = 'published'
            AND r.revision_status = 'published'
            AND a.artifact_key = $3 AND a.status = 'ready'
        )
        "#,
    )
    .bind(&journal.page_node_key)
    .bind(&journal.revision_id)
    .bind(&journal.artifact_key)
    .fetch_one(&mut *tx)
    .await
    .map_err(classify_sqlx)?;
    let live = match std::fs::read_link(&public) {
        Ok(target) => Some(target),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(validation_failure(format!("read live publication pointer: {err}"))),
    };
    require_cloudflare_publication_backend().map_err(validation_failure)?;
    if committed {
        if live.as_deref() != Some(journal.new_release.as_path()) {
            return Err(validation_failure("committed publication no longer matches live pointer"));
        }
        // The live Worker is the public truth: a local symlink check is not
        // sufficient after a worker crash. Idempotently restore the committed
        // Cloudflare asset version and independently verify public HTTPS.
        deploy_cloudflare_static_release(&journal.new_release, None)
            .map_err(validation_failure)?;
    } else {
        if live.as_deref() != Some(journal.new_release.as_path())
            && live.as_deref() != journal.previous.as_deref()
        {
            return Err(validation_failure("pending publication has unknown live pointer; manual recovery required"));
        }
        // An interrupted first publication has no prior public version.
        // Preserve the journal and fail closed until a human can inspect
        // the dedicated staging Worker and revoke any partial first upload.
        let previous = journal.previous.as_deref().ok_or_else(|| {
            validation_failure("first remote publication is uncommitted; manual recovery required")
        })?;
        deploy_cloudflare_static_release(previous, None).map_err(validation_failure)?;
        if live.as_deref() == Some(journal.new_release.as_path()) {
            restore_public_release(&public, Some(previous))
                .map_err(validation_failure)?;
        }
    }
    std::fs::remove_file(&journal_path)
        .map_err(|err| validation_failure(format!("clear reconciled publication journal: {err}")))?;
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(())
}

fn restore_public_release(
    public: &ReleasePath,
    previous: Option<&ReleasePath>,
) -> Result<(), String> {
    use std::os::unix::fs::symlink;
    if let Some(previous) = previous {
        let parent = public.parent().ok_or("public path has no parent")?;
        let temp = parent.join(format!(".alegria-rollback-{}", uuid::Uuid::new_v4()));
        symlink(previous, &temp).map_err(|err| format!("create rollback link: {err}"))?;
        std::fs::rename(&temp, public).map_err(|err| format!("restore prior live release: {err}"))
    } else {
        std::fs::remove_file(public).map_err(|err| format!("remove unverified first release: {err}"))
    }
}

async fn verify_live_release(
    origin: &reqwest::Url,
    files: &[ReleaseArtifact],
) -> Result<(), String> {
    let expected_origin = origin.origin();
    let client = reqwest::Client::builder()
        // Static directory indexes may redirect /topic to /topic/. Never
        // follow redirects to a different origin or to plaintext HTTP.
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 3 || attempt.url().origin() != expected_origin {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .timeout(std::time::Duration::from_secs(20))
        .build().map_err(|err| format!("configure HTTPS readback: {err}"))?;
    for file in files.iter().filter(|item| !item.public_route.is_empty()) {
        let url = origin.join(file.public_route.trim_start_matches('/'))
            .map_err(|err| format!("readback URL: {err}"))?;
        let response = client.get(url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send().await.map_err(|err| format!("HTTPS readback request: {err}"))?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("HTTPS readback failed: status {}", response.status()));
        }
        let body = response.bytes().await.map_err(|err| format!("read public bytes: {err}"))?;
        if body.as_ref() != file.bytes.as_slice() {
            return Err(format!("public HTTPS content differs: {}", file.public_route));
        }
    }
    Ok(())
}

// The activation guard restores the prior release if DB finalization fails.
// Only a committed DB transaction may disarm it.
struct PublicationActivation {
    #[cfg(unix)]
    public_path: ReleasePathBuf,
    #[cfg(unix)]
    previous: Option<ReleasePathBuf>,
    #[cfg(unix)]
    journal_path: ReleasePathBuf,
    #[cfg(unix)]
    remote_activated: bool,
    armed: bool,
}

impl PublicationActivation {
    // COMMIT may have succeeded even when the database connection drops or the
    // activity is cancelled. Once COMMIT begins, leave the on-disk journal and
    // public pointer intact until the next database-aware reconciliation.
    fn defer_to_recovery(&mut self) {
        self.armed = false;
    }

    fn confirm(&mut self) {
        self.armed = false;
        #[cfg(unix)]
        if let Err(err) = std::fs::remove_file(&self.journal_path) {
            tracing::error!(reason = %err, "committed publication journal awaits startup reconciliation");
        }
    }
}

#[cfg(unix)]
impl Drop for PublicationActivation {
    fn drop(&mut self) {
        if self.armed {
            if self.remote_activated {
                if let Some(previous) = self.previous.as_deref() {
                    if let Err(err) = deploy_cloudflare_static_release(previous, None) {
                        tracing::error!(reason = %err, "remote rollback failed; journal retained for recovery");
                        return;
                    }
                } else {
                    // A first release has no remote predecessor; do not clear
                    // its journal or declare a successful rollback.
                    tracing::error!("first Cloudflare publication has no prior release; manual recovery required");
                    return;
                }
            }
            match restore_public_release(&self.public_path, self.previous.as_deref()) {
                Ok(()) => {
                    if let Err(err) = std::fs::remove_file(&self.journal_path) {
                        tracing::error!(reason = %err, "failed to clear rolled-back publication journal");
                    }
                }
                Err(err) => tracing::error!(reason = %err, "failed to restore prior release after DB failure"),
            }
        }
    }
}

#[cfg(unix)]
async fn promote_and_verify_public_release(
    artifact: &PublishArtifact,
    page_node_key: &str,
    revision_id: &str,
) -> Result<PublicationActivation, String> {
    let meta: serde_json::Value = serde_json::from_str(&artifact.manifest_json)
        .map_err(|err| format!("persisted candidate metadata: {err}"))?;
    let candidate_raw = meta.get("candidate_output_dir")
        .and_then(|v| v.as_str()).ok_or("missing candidate release path")?;
    let public_raw = meta.get("public_output_dir")
        .and_then(|v| v.as_str()).ok_or("missing public output path")?;
    let origin_raw = meta.get("base_url")
        .and_then(|v| v.as_str()).ok_or("missing public origin")?;
    if meta.get("page_node_key").and_then(|v| v.as_str()) != Some(page_node_key)
        || meta.get("revision_id").and_then(|v| v.as_str()) != Some(revision_id)
        || artifact.artifact_uri != candidate_raw
        || artifact.status != "built_pending_validation"
    {
        return Err("candidate artifact does not match approved page/revision".into());
    }
    let origin = publication_origin(origin_raw)?;
    // No implicit deploy to an unconfigured origin, and never to a placeholder.
    if std::env::var("SEO_STATIC_BASE_URL").ok().as_deref() != Some(origin_raw) {
        return Err("SEO_STATIC_BASE_URL must explicitly name the public HTTPS origin".into());
    }
    let public = ReleasePathBuf::from(public_raw);
    let configured_public = std::env::var("SEO_STATIC_OUTPUT_DIR")
        .unwrap_or_else(|_| "app/rust/dist/static-site".to_string());
    if ReleasePath::new(&configured_public) != public {
        return Err("candidate public output path must equal the configured worker path".into());
    }
    if public.as_os_str().is_empty() {
        return Err("public output path is empty".into());
    }
    let candidate = std::fs::canonicalize(candidate_raw)
        .map_err(|err| format!("candidate release does not exist: {err}"))?;
    if !candidate.is_dir() || public == candidate {
        return Err("candidate must be isolated from live output".into());
    }
    let files = publication_files(&candidate, page_node_key, revision_id, origin_raw)?;
    let release = create_immutable_release(&candidate, &files)?;
    let journal_path = pending_publication_path(&public)?;
    let previous = match std::fs::read_link(&public) {
        Ok(link) => Some(link),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(format!("read current live release: {err}")),
    };
    let pending = PendingPublication {
        new_release: release.clone(),
        previous: previous.clone(),
        page_node_key: page_node_key.to_string(),
        revision_id: revision_id.to_string(),
        artifact_key: artifact.artifact_key.clone(),
    };
    write_pending_publication(&journal_path, &pending)?;
    if let Err(err) = swap_public_release(&public, &release) {
        let _ = std::fs::remove_file(&journal_path);
        return Err(err);
    }
    // Construct the rollback guard before the HTTPS await. The journal also
    // survives a worker/process crash until startup reconciliation.
    let mut activation = PublicationActivation {
        public_path: public,
        previous,
        journal_path,
        remote_activated: false,
        armed: true,
    };
    // Only the explicitly configured native Cloudflare deployment may
    // promote files to the public origin. A local symlink is not deployment.
    require_cloudflare_publication_backend()?;
    activation.remote_activated = true;
    deploy_cloudflare_static_release(&release, activation.previous.as_deref())?;
    verify_live_release(&origin, &files).await?;
    Ok(activation)
}

#[cfg(not(unix))]
async fn promote_and_verify_public_release(
    _artifact: &PublishArtifact,
    _page_node_key: &str,
    _revision_id: &str,
) -> Result<PublicationActivation, String> {
    Err("atomic static deployment requires a Unix filesystem".into())
}

#[cfg(test)]
mod publication_release_tests {
    use super::*;

    #[test]
    fn rejects_placeholder_and_unsafe_origins() {
        for origin in [
            "https://example.com", "https://site.test", "http://real.example.org",
            "https://localhost", "https://127.0.0.1",
            "https://user:password@staging.alegria.by", "https://staging.alegria.by/path",
        ] {
            assert!(publication_origin(origin).is_err(), "{origin}");
        }
        assert!(publication_origin("https://staging.alegria.by").is_ok());
    }

    #[test]
    fn candidate_rejects_unapproved_and_missing_pages() {
        let root = std::env::temp_dir().join(format!("alegria-candidate-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"page").unwrap();
        std::fs::write(root.join("sitemap.xml"), b"sitemap").unwrap();
        std::fs::write(root.join("robots.txt"), b"robots").unwrap();
        let manifest = serde_json::json!({
            "base_url": "https://staging.alegria.by",
            "page_count": 1,
            "pages": [{
                "page_node_key": "page", "revision_id": "revision", "status": "approved",
                "canonical_url_path": "/"
            }]
        });
        std::fs::write(root.join("alegria-static-manifest.json"), manifest.to_string()).unwrap();
        assert!(publication_files(&root, "page", "revision", "https://staging.alegria.by").is_ok());
        assert!(publication_files(&root, "page", "wrong", "https://staging.alegria.by").is_err());
        let mut wrong = manifest;
        wrong["pages"][0]["canonical_url_path"] = serde_json::json!("/missing");
        std::fs::write(root.join("alegria-static-manifest.json"), wrong.to_string()).unwrap();
        assert!(publication_files(&root, "page", "revision", "https://staging.alegria.by").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn immutable_release_switch_and_rollback_preserve_previous_site() {
        let root = std::env::temp_dir().join(format!("alegria-promotion-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let previous_release = root.join("old");
        std::fs::create_dir(&previous_release).unwrap();
        std::fs::write(previous_release.join("index.html"), b"prior").unwrap();
        let candidate = root.join("candidate");
        std::fs::create_dir(&candidate).unwrap();
        let files = vec![ReleaseArtifact {
            relative_path: "index.html".into(), public_route: "/".into(), bytes: b"new".to_vec(),
        }];
        let new_release = create_immutable_release(&candidate, &files).unwrap();
        let public = root.join("live");
        let first = swap_public_release(&public, &previous_release).unwrap();
        assert!(first.is_none());
        let previous = swap_public_release(&public, &new_release).unwrap();
        assert_eq!(std::fs::read(public.join("index.html")).unwrap(), b"new");
        restore_public_release(&public, previous.as_deref()).unwrap();
        assert_eq!(std::fs::read(public.join("index.html")).unwrap(), b"prior");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cancelled_publication_restores_prior_site_and_clears_journal() {
        let root = std::env::temp_dir().join(format!("alegria-cancel-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        std::fs::write(old.join("index.html"), b"old").unwrap();
        std::fs::write(new.join("index.html"), b"new").unwrap();
        let live = root.join("live");
        swap_public_release(&live, &old).unwrap();
        let journal = pending_publication_path(&live).unwrap();
        let pending = PendingPublication {
            new_release: new.clone(),
            previous: Some(old.clone()),
            page_node_key: "page".into(),
            revision_id: "candidate".into(),
            artifact_key: "artifact".into(),
        };
        write_pending_publication(&journal, &pending).unwrap();
        assert!(write_pending_publication(&journal, &pending).is_err());
        let previous = swap_public_release(&live, &new).unwrap();
        {
            let _activation = PublicationActivation {
                public_path: live.clone(),
                previous,
                journal_path: journal.clone(),
                remote_activated: false,
                armed: true,
            };
            assert_eq!(std::fs::read(live.join("index.html")).unwrap(), b"new");
        }
        assert_eq!(std::fs::read(live.join("index.html")).unwrap(), b"old");
        assert!(!journal.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn committed_publication_keeps_new_site_and_clears_journal() {
        let root = std::env::temp_dir().join(format!("alegria-commit-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let new = root.join("new");
        std::fs::create_dir(&new).unwrap();
        std::fs::write(new.join("index.html"), b"new").unwrap();
        let live = root.join("live");
        let journal = pending_publication_path(&live).unwrap();
        write_pending_publication(&journal, &PendingPublication {
            new_release: new.clone(),
            previous: None,
            page_node_key: "page".into(),
            revision_id: "candidate".into(),
            artifact_key: "artifact".into(),
        }).unwrap();
        swap_public_release(&live, &new).unwrap();
        {
            let mut activation = PublicationActivation {
                public_path: live.clone(),
                previous: None,
                journal_path: journal.clone(),
                remote_activated: false,
                armed: true,
            };
            activation.confirm();
        }
        assert_eq!(std::fs::read(live.join("index.html")).unwrap(), b"new");
        assert!(!journal.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn uncertain_commit_preserves_journal_and_public_release_for_recovery() {
        let root = std::env::temp_dir().join(format!("alegria-ambiguous-commit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        let live = root.join("live");
        let journal = pending_publication_path(&live).unwrap();
        swap_public_release(&live, &old).unwrap();
        write_pending_publication(&journal, &PendingPublication {
            new_release: new.clone(),
            previous: Some(old.clone()),
            page_node_key: "page".into(),
            revision_id: "new-revision".into(),
            artifact_key: "new-artifact".into(),
        }).unwrap();
        let previous = swap_public_release(&live, &new).unwrap();
        {
            let mut receipt = PublicationActivation {
                public_path: live.clone(),
                previous,
                journal_path: journal.clone(),
                remote_activated: false,
                armed: true,
            };
            // A cancelled/dropped COMMIT future has an unknown DB result.
            receipt.defer_to_recovery();
        }
        assert_eq!(std::fs::read_link(&live).unwrap(), new);
        assert!(journal.is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
