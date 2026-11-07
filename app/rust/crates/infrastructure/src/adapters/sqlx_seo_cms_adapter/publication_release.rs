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
    if manifest.get("page_count").and_then(|v| v.as_u64()) != Some(pages.len() as u64) {
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
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
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

#[cfg(unix)]
async fn promote_and_verify_public_release(
    artifact: &PublishArtifact,
    page_node_key: &str,
    revision_id: &str,
) -> Result<(), String> {
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
    let previous = swap_public_release(&public, &release)?;
    if let Err(err) = verify_live_release(&origin, &files).await {
        restore_public_release(&public, previous.as_deref())?;
        return Err(err);
    }
    Ok(())
}

#[cfg(not(unix))]
async fn promote_and_verify_public_release(
    _artifact: &PublishArtifact,
    _page_node_key: &str,
    _revision_id: &str,
) -> Result<(), String> {
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
}
