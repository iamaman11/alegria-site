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

#[cfg(unix)]
fn verify_immutable_directory(path: &Path, trusted_uid: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect trusted catalog directory {}", path.display()))?;
    anyhow::ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == trusted_uid
            && meta.mode() & 0o022 == 0,
        "catalog directory must be trusted-owned, nonsymlink, non-group/world writable"
    );
    Ok(())
}

#[cfg(unix)]
fn verify_immutable_file(path: &Path, trusted_uid: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect trusted catalog file {}", path.display()))?;
    anyhow::ensure!(
        meta.is_file()
            && !meta.file_type().is_symlink()
            && meta.uid() == trusted_uid
            && meta.mode() & 0o022 == 0,
        "catalog file must be regular, trusted-owned, nonsymlink and non-group/world writable"
    );
    anyhow::ensure!(
        meta.len() <= MAX_CATALOG_BYTES,
        "trusted owner route catalog exceeds the maximum size"
    );
    Ok(())
}

/// The routing registry is executable authority. Never load an arbitrary
/// user-writable absolute JSON path, even if the manifest pins its bytes.
#[cfg(unix)]
fn validate_config_file_trust(path: &Path) -> Result<()> {
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
        verify_immutable_directory(dir, 0)?;
        current = dir.parent();
    }
    verify_immutable_file(path, 0)
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

fn register_trusted_entry(
    catalog: &mut TrustedRouteCatalog,
    entry: TrustedRegistrationV1,
) -> Result<()> {
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
    Ok(())
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
        register_trusted_entry(&mut catalog, entry)?;
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{IdempotencyPolicy, OperationId, OperationOwnerId, OperationRef};
    use std::collections::{BTreeMap, BTreeSet};

    fn registered_test_owner() -> TrustedRegistrationV1 {
        let contract = OperationContract {
            reference: OperationRef {
                id: OperationId::from("test.owner.pure"),
                revision: 1,
            },
            owner: OperationOwnerId::from("test-owner"),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        };
        let route = PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.to_string(),
            task_queue: "alegria-test-owner-v1".to_string(),
            build_id: "0123456789abcdef0123456789abcdef01234567".to_string(),
        };
        TrustedRegistrationV1 { contract, route }
    }

    #[test]
    fn untrusted_catalog_path_is_never_executable_authority() {
        assert!(load_trusted_owner_catalog(Path::new("./route-catalog.json")).is_err());
        assert!(load_trusted_owner_catalog(Path::new("/tmp/route-catalog.json")).is_err());
        assert!(load_trusted_owner_catalog(Path::new("/etc/alegria")).is_err());
        #[cfg(unix)]
        {
            let traversal =
                validate_config_file_trust(Path::new("/etc/alegria/../bad.json")).unwrap_err();
            assert!(traversal.to_string().contains("parent traversal"));
        }
    }

    #[test]
    fn only_fixed_native_owner_type_and_independent_queue_can_be_registered() {
        register_trusted_entry(&mut TrustedRouteCatalog::default(), registered_test_owner())
            .unwrap();

        let mut arbitrary_type = registered_test_owner();
        arbitrary_type.route.workflow_type = "CallerSelectedWorkflow".to_string();
        assert!(
            register_trusted_entry(&mut TrustedRouteCatalog::default(), arbitrary_type).is_err()
        );

        let mut parent_queue = registered_test_owner();
        parent_queue.route.task_queue = OPERATION_DAG_PARENT_QUEUE.to_string();
        assert!(register_trusted_entry(&mut TrustedRouteCatalog::default(), parent_queue).is_err());

        let mut ephemeral_build = registered_test_owner();
        ephemeral_build.route.build_id = "development".to_string();
        assert!(
            register_trusted_entry(&mut TrustedRouteCatalog::default(), ephemeral_build).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_and_directory_permission_checks_reject_symlinks_and_writable_trust_roots() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "alegria-owner-catalog-permissions-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let uid = std::fs::symlink_metadata(&dir).unwrap().uid();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        verify_immutable_directory(&dir, uid).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(verify_immutable_directory(&dir, uid).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let file = dir.join("trusted.json");
        std::fs::write(&file, b"{}").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        verify_immutable_file(&file, uid).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(verify_immutable_file(&file, uid).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        let link = dir.join("redirect.json");
        symlink(&file, &link).unwrap();
        assert!(verify_immutable_file(&link, uid).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
