//! Deployment-owned catalog for native, statically typed owner child routes.
//!
//! This JSON file is a process configuration trust root, NOT a DAG-manifest
//! field. The worker reads it once; every Workflow Start History pins the
//! exact subset of registered routes, checked again on every replay. A changed
//! catalog must not silently reinterpret an already started Workflow.

use std::path::Path;

use anyhow::{Context, Result};
use operation_protocol::{
    OperationContract, PinnedOwnerRouteV1, TrustedRouteCatalog, OPERATION_DAG_PARENT_QUEUE,
    OWNER_OPERATION_CHILD_TYPE_V1,
};
use serde::Deserialize;

const TRUSTED_CATALOG_ROOT: &str = "/etc/alegria";
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;

/// The routing registry is executable authority. Never load an arbitrary
/// user-writable absolute JSON path, even if the manifest pins its bytes.
#[cfg(unix)]
fn validate_config_file_trust(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;

    let trusted_root = Path::new(TRUSTED_CATALOG_ROOT);
    anyhow::ensure!(
        path.starts_with(trusted_root) && path != trusted_root,
        "trusted owner catalog must reside under {TRUSTED_CATALOG_ROOT}"
    );
    anyhow::ensure!(
        path.components()
            .all(|component| !matches!(component, Component::ParentDir | Component::CurDir)),
        "trusted catalog path must not contain dot or parent traversal"
    );
    // Check EVERY parent, not just the JSON file. Symlinks or any writable
    // directory in the chain could replace a previously trusted catalog.
    let mut current = path.parent();
    while let Some(dir) = current {
        let meta = std::fs::symlink_metadata(dir)
            .with_context(|| format!("inspect trusted catalog directory {}", dir.display()))?;
        anyhow::ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && meta.uid() == 0
                && meta.mode() & 0o022 == 0,
            "catalog directory must be root-owned, nonsymlink, non-group/world writable"
        );
        current = dir.parent();
    }
    let file = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect trusted catalog file {}", path.display()))?;
    anyhow::ensure!(
        file.is_file()
            && !file.file_type().is_symlink()
            && file.uid() == 0
            && file.mode() & 0o022 == 0,
        "catalog file must be regular, root-owned, nonsymlink and non-group/world writable"
    );
    anyhow::ensure!(
        file.len() <= MAX_CATALOG_BYTES,
        "trusted owner route catalog exceeds the maximum size"
    );
    Ok(())
}

#[cfg(not(unix))]
fn validate_config_file_trust(_path: &Path) -> Result<()> {
    anyhow::bail!("generic DAG trusted owner catalog requires Unix file ownership checks")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedCatalogFileV1 {
    schema_revision: u16,
    entries: Vec<TrustedRegistrationV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedRegistrationV1 {
    contract: OperationContract,
    route: PinnedOwnerRouteV1,
}

pub(crate) fn load_trusted_owner_catalog(path: &Path) -> Result<TrustedRouteCatalog> {
    anyhow::ensure!(
        path.is_absolute(),
        "trusted route catalog path must be absolute"
    );
    validate_config_file_trust(path)?;
    let data = std::fs::read(path)
        .with_context(|| format!("read operator-owned route catalog {}", path.display()))?;
    let config: TrustedCatalogFileV1 =
        serde_json::from_slice(&data).context("decode trusted owner route catalog")?;
    anyhow::ensure!(
        config.schema_revision == 1,
        "unsupported trusted route catalog version"
    );
    anyhow::ensure!(!config.entries.is_empty(), "trusted route catalog is empty");
    let mut catalog = TrustedRouteCatalog::default();
    for entry in config.entries {
        let route = &entry.route;
        anyhow::ensure!(
            route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1,
            "all independent owner queues must register the same typed owner Workflow Type"
        );
        anyhow::ensure!(
            route.task_queue != OPERATION_DAG_PARENT_QUEUE,
            "owner child must never poll the DAG parent Task Queue"
        );
        anyhow::ensure!(
            route.build_id.len() == 40
                && route.build_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                && route.build_id.bytes().any(|byte| byte != b'0'),
            "owner build_id must be a nonzero immutable 40-hex source SHA"
        );
        catalog.register(entry.contract, entry.route)?;
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_catalog_path_is_never_executable_authority() {
        assert!(load_trusted_owner_catalog(Path::new("./route-catalog.json")).is_err());
        assert!(load_trusted_owner_catalog(Path::new("/tmp/route-catalog.json")).is_err());
        assert!(load_trusted_owner_catalog(Path::new("/etc/alegria")).is_err());
    }
}
