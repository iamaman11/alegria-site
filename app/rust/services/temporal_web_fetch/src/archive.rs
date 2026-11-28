//! Operator-owned immutable raw-byte archive. This is deliberately *below*
//! the source/Temporal receipt boundary: a file existing on disk is not a
//! provider attestation, an authorization grant, or a successful DAG result.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::transport::NativePageCaptureV1;

const ARCHIVE_DIRECTORY: &str = "web-page-snapshots-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPageBytesV1 {
    /// Relative to the trusted operator archive root; never a client URL.
    pub relative_snapshot_key: String,
    pub content_hash: String,
    pub body_bytes: u32,
    /// Immutable source evidence, separately content-addressed under tenant.
    pub relative_evidence_key: String,
    pub robots_evidence_hash: String,
    pub redirect_chain_hash: String,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct SourceEvidenceV1<'a> {
    revision: u16,
    visited_urls: &'a [String],
    robots_by_url:
        &'a std::collections::BTreeMap<String, crate::transport::RobotsResponseEvidenceV1>,
}

fn invalid_data(description: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, description)
}

/// Persist only bytes that were received by the pinned-HTTPS transport and
/// whose BLAKE3 digest and length match its checked proof. A randomly named
/// same-directory staging file is synced and then atomically hard-linked to
/// the final content-addressed name; another writer can never overwrite it.
/// The caller still needs a separate authorized, durable Temporal/PG receipt.
pub fn persist_captured_html_v1(
    trusted_archive_root: &Path,
    tenant_id: &str,
    capture: &NativePageCaptureV1,
) -> io::Result<StoredPageBytesV1> {
    if !trusted_archive_root.is_absolute() || tenant_id.is_empty() || tenant_id.len() > 128 {
        return Err(invalid_data("operator archive root/tenant must be bounded"));
    }
    let result = &capture.checked_transport;
    let digest = blake3::hash(&capture.html_bytes).to_hex().to_string();
    if digest != result.content_hash || capture.html_bytes.len() as u64 != result.decoded_body_bytes
    {
        return Err(invalid_data(
            "captured bytes do not match checked transport result",
        ));
    }
    if result.robots_hashes_by_url.len() != capture.robots_by_url.len()
        || result.robots_hashes_by_url.iter().any(|(url, hash)| {
            capture
                .robots_by_url
                .get(url)
                .is_none_or(|observation| observation.digest() != *hash)
        })
    {
        return Err(invalid_data(
            "robots observation hashes do not match checked policy",
        ));
    }
    let robots_bytes = serde_json::to_vec(&capture.robots_by_url)
        .map_err(|_| invalid_data("robots evidence serialization failed"))?;
    let robots_evidence_hash = blake3::hash(&robots_bytes).to_hex().to_string();
    let redirect_bytes = serde_json::to_vec(&result.visited_urls)
        .map_err(|_| invalid_data("redirect chain serialization failed"))?;
    let redirect_chain_hash = blake3::hash(&redirect_bytes).to_hex().to_string();
    let evidence = SourceEvidenceV1 {
        revision: 1,
        visited_urls: &result.visited_urls,
        robots_by_url: &capture.robots_by_url,
    };
    let evidence_bytes = serde_json::to_vec(&evidence)
        .map_err(|_| invalid_data("source evidence serialization failed"))?;
    let evidence_hash = blake3::hash(&evidence_bytes).to_hex().to_string();
    let body_bytes = u32::try_from(capture.html_bytes.len())
        .map_err(|_| invalid_data("snapshot byte length exceeds u32"))?;
    let tenant_hash = blake3::hash(tenant_id.as_bytes()).to_hex().to_string();
    let rel_dir = PathBuf::from(ARCHIVE_DIRECTORY).join(&tenant_hash);
    let directory = trusted_archive_root.join(&rel_dir);
    create_private_directory(&directory)?;

    let filename = format!("{digest}.html");
    persist_immutable_bytes(&directory, &filename, &digest, &capture.html_bytes)?;
    let evidence_filename = format!("{evidence_hash}.evidence.json");
    persist_immutable_bytes(
        &directory,
        &evidence_filename,
        &evidence_hash,
        &evidence_bytes,
    )?;
    sync_archive_directory(&directory)?;

    let relative_snapshot_key = rel_dir.join(filename).to_string_lossy().replace('\\', "/");
    let relative_evidence_key = rel_dir
        .join(evidence_filename)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(StoredPageBytesV1 {
        relative_snapshot_key,
        content_hash: digest,
        body_bytes,
        relative_evidence_key,
        robots_evidence_hash,
        redirect_chain_hash,
    })
}

fn persist_immutable_bytes(
    directory: &Path,
    filename: &str,
    digest: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let target = directory.join(filename);
    let random_stage = directory.join(format!(
        ".{digest}.{}.{}.staging",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let mut staged = create_private_file(&random_stage)?;
    let staged_result = staged.write_all(bytes).and_then(|()| staged.sync_all());
    drop(staged);
    if let Err(error) = staged_result {
        let _ = fs::remove_file(&random_stage);
        return Err(error);
    }

    let publish_result = match fs::hard_link(&random_stage, &target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // Content addressing makes an exact match idempotent. Still read
            // back the existing bytes: an attacker or torn old write must not
            // become trusted solely because the filename matches a hash.
            verify_existing(&target, digest, bytes.len())
        }
        Err(error) => Err(error),
    };
    let _ = fs::remove_file(&random_stage);
    publish_result?;
    Ok(())
}

fn verify_existing(target: &Path, digest: &str, body_bytes: usize) -> io::Result<()> {
    let file = File::open(target)?;
    if file.metadata()?.len() != body_bytes as u64 {
        return Err(invalid_data("immutable snapshot collision or torn write"));
    }
    let mut bytes = Vec::with_capacity(body_bytes);
    file.take(body_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() != body_bytes || blake3::hash(&bytes).to_hex().as_str() != digest {
        return Err(invalid_data(
            "immutable snapshot content differs from digest",
        ));
    }
    Ok(())
}

fn sync_archive_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        // Windows does not expose a portable read-only directory fsync.
        // Both individual files were synced before they were published.
        let _ = directory;
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)?;
    // Do not silently reuse a world-writable/symlinked operator archive.
    let metadata = fs::symlink_metadata(path)?;
    use std::os::unix::fs::PermissionsExt;
    if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "archive directory must be private and not a symlink",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::RobotsResponseEvidenceV1;
    use std::collections::BTreeMap;
    use web_fetch_contracts::runtime_policy::WebFetchTransportResultV1;

    fn capture(bytes: &[u8]) -> NativePageCaptureV1 {
        NativePageCaptureV1 {
            checked_transport: WebFetchTransportResultV1 {
                final_url: "https://www.example.com/".to_owned(),
                content_hash: blake3::hash(bytes).to_hex().to_string(),
                validated_redirects: 0,
                decoded_body_bytes: bytes.len() as u64,
                visited_urls: vec!["https://www.example.com/".into()],
                robots_hashes_by_url: BTreeMap::new(),
            },
            html_bytes: bytes.to_vec(),
            robots_by_url: BTreeMap::new(),
        }
    }

    #[test]
    fn content_addressed_archive_keeps_tenant_and_bytes_immutable() {
        let temp = std::env::temp_dir().join(format!("web-fetch-archive-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&temp).unwrap();
        let page = capture(b"<!doctype html><h1>Safe public page</h1>");
        let first = persist_captured_html_v1(&temp, "tenant-a", &page).unwrap();
        let second = persist_captured_html_v1(&temp, "tenant-a", &page).unwrap();
        assert_eq!(first, second);
        assert!(first
            .relative_snapshot_key
            .contains("web-page-snapshots-v1/"));
        assert!(!first.relative_snapshot_key.contains("tenant-a"));
        let absolute = temp.join(&first.relative_snapshot_key);
        assert_eq!(fs::read(&absolute).unwrap(), page.html_bytes);
        let evidence = temp.join(&first.relative_evidence_key);
        assert!(fs::read(evidence).unwrap().starts_with(b"{\"revision\":1"));
        // Tamper detection also protects an existing path with a valid name.
        fs::write(&absolute, b"BAD").unwrap();
        assert!(persist_captured_html_v1(&temp, "tenant-a", &page).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn archive_refuses_unverified_transport_claim() {
        let mut page = capture(b"<html></html>");
        page.checked_transport.content_hash = "00".repeat(32);
        assert!(persist_captured_html_v1(Path::new("/tmp"), "tenant-a", &page).is_err());
    }

    #[test]
    fn archive_stores_independent_robots_and_redirect_evidence() {
        let temp = std::env::temp_dir().join(format!("web-fetch-archive-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&temp).unwrap();
        let mut page = capture(b"<html>source</html>");
        let robots = RobotsResponseEvidenceV1 {
            status_code: 200,
            body: b"User-agent: *\nDisallow: /private\n".to_vec(),
        };
        let url = "https://www.example.com/".to_owned();
        page.checked_transport
            .robots_hashes_by_url
            .insert(url.clone(), robots.digest());
        page.robots_by_url.insert(url, robots);
        let first = persist_captured_html_v1(&temp, "tenant-a", &page).unwrap();
        let evidence: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.join(&first.relative_evidence_key)).unwrap())
                .unwrap();
        assert_eq!(evidence["revision"], 1);
        assert_eq!(evidence["visited_urls"][0], "https://www.example.com/");
        assert_eq!(evidence["robots_by_url"].as_object().unwrap().len(), 1);

        let other_tenant = persist_captured_html_v1(&temp, "tenant-b", &page).unwrap();
        assert_ne!(
            first.relative_snapshot_key,
            other_tenant.relative_snapshot_key
        );

        page.robots_by_url.clear();
        assert!(persist_captured_html_v1(&temp, "tenant-a", &page).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
}
