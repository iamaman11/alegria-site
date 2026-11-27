//! Shared, deployment-owned catalog for native, statically typed owner child routes.
//!
//! This JSON file is a process configuration trust root, NOT a DAG-manifest
//! field. The worker reads it once; every Workflow Start History pins the
//! exact subset of registered routes, checked again on every replay. A changed
//! catalog must not silently reinterpret an already started Workflow.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use operation_protocol::{
    ContractCatalog, IdempotencyPolicy, OperationContract, OperationRef,
    PinnedOwnerOperationSchemaV3, PinnedOwnerRouteV1, TrustedRouteCatalog,
    OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_QUEUE_V2, OWNER_OPERATION_CHILD_TYPE_V1,
};
use serde::Deserialize;
use serde_json::Value;

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
    /// V3 also accepts pure owner schemas; pure operations do not have an
    /// effectful child route and must never receive a fabricated one.
    #[serde(default)]
    schema_entries: Vec<TrustedSchemaRegistrationV3>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedRegistrationV1 {
    contract: OperationContract,
    route: PinnedOwnerRouteV1,
    /// Optional in historical v1; mandatory in v2 for rich owner parameters.
    #[serde(default)]
    parameter_schema: Option<Value>,
    /// Required for every effectful route in V3; unavailable to V1/V2.
    #[serde(default)]
    owner_schema: Option<PinnedOwnerOperationSchemaV3>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedSchemaRegistrationV3 {
    contract: OperationContract,
    owner_schema: PinnedOwnerOperationSchemaV3,
    /// Owner build identity, not a user-supplied authoring parameter.
    build_id: String,
}

/// The same verified source drives Temporal routing and ChatGPT discovery.
/// Schema presence alone never authorizes an owner call or an effect.
pub struct TrustedOwnerCatalog {
    pub routes: TrustedRouteCatalog,
    pub parameter_schemas: BTreeMap<OperationRef, Value>,
    /// Trusted installation, not a request-provided or self-sealed schema.
    owner_schemas: BTreeMap<OperationRef, PinnedOwnerOperationSchemaV3>,
    schema_contracts: ContractCatalog,
    /// Exact owner build that published the schema. Runtime deployment must
    /// separately attest that it is the build polling the relevant queue.
    schema_build_ids: BTreeMap<OperationRef, String>,
}

impl TrustedOwnerCatalog {
    pub fn owner_schema(&self, operation: &OperationRef) -> Option<&PinnedOwnerOperationSchemaV3> {
        self.owner_schemas.get(operation)
    }

    pub fn owner_schema_build_id(&self, operation: &OperationRef) -> Option<&str> {
        self.schema_build_ids.get(operation).map(String::as_str)
    }

    /// Caller schema can only be checked against a separately installed
    /// owner schema. This method does not grant execution or validate an
    /// input payload: actual DTO payload validation remains owner-local.
    pub fn verify_caller_schema(
        &self,
        operation: &OperationRef,
        caller: &PinnedOwnerOperationSchemaV3,
    ) -> Result<()> {
        let trusted = self
            .owner_schemas
            .get(operation)
            .context("no authoritative owner structural schema installed for this operation")?;
        let contract = self
            .schema_contracts
            .contract(operation)
            .context("missing installed owner contract")?;
        caller
            .verify_against_trusted_owner(trusted, contract)
            .context("caller owner schema differs from trusted installation")
    }
}

fn validate_build_id(build_id: &str) -> Result<()> {
    anyhow::ensure!(
        build_id.len() == 40
            && build_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            && build_id.bytes().any(|byte| byte != b'0'),
        "owner build_id must be a nonzero immutable 40-hex source SHA"
    );
    Ok(())
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
        route.task_queue != OPERATION_DAG_PARENT_QUEUE
            && route.task_queue != OPERATION_DAG_PARENT_QUEUE_V2,
        "owner child must never poll the DAG parent Task Queue"
    );
    validate_build_id(&route.build_id)?;
    catalog.register(entry.contract, entry.route)?;
    Ok(())
}

pub fn load_trusted_owner_catalog(path: &Path) -> Result<TrustedRouteCatalog> {
    Ok(load_trusted_owner_bundle(path)?.routes)
}

pub fn load_trusted_owner_bundle(path: &Path) -> Result<TrustedOwnerCatalog> {
    anyhow::ensure!(
        path.is_absolute(),
        "trusted route catalog path must be absolute"
    );
    validate_config_file_trust(path)?;
    let data = std::fs::read(path)
        .with_context(|| format!("read operator-owned route catalog {}", path.display()))?;
    decode_trusted_catalog(&data)
}

fn decode_trusted_catalog(data: &[u8]) -> Result<TrustedOwnerCatalog> {
    let config: TrustedCatalogFileV1 =
        serde_json::from_slice(data).context("decode trusted owner route catalog")?;
    anyhow::ensure!(
        (1..=3).contains(&config.schema_revision),
        "unsupported trusted route catalog version"
    );
    anyhow::ensure!(
        !config.entries.is_empty()
            || (config.schema_revision == 3 && !config.schema_entries.is_empty()),
        "trusted owner catalog is empty"
    );
    anyhow::ensure!(
        config.schema_revision == 3 || config.schema_entries.is_empty(),
        "standalone pure owner schemas require trusted catalog v3"
    );
    let mut catalog = TrustedRouteCatalog::default();
    let mut parameter_schemas = BTreeMap::new();
    let mut owner_schemas = BTreeMap::new();
    let mut schema_contracts = ContractCatalog::default();
    let mut schema_build_ids = BTreeMap::new();
    for entry in config.entries {
        let operation = entry.contract.reference.clone();
        if config.schema_revision == 1 {
            anyhow::ensure!(
                entry.parameter_schema.is_none(),
                "v1 owner routes cannot declare parameter schemas"
            );
        } else {
            let schema = entry
                .parameter_schema
                .as_ref()
                .context("v2 owner must declare its own parameter schema")?;
            anyhow::ensure!(
                schema.get("type").and_then(Value::as_str) == Some("object"),
                "owner parameter schema must describe an object"
            );
        }
        if config.schema_revision < 3 {
            anyhow::ensure!(
                entry.owner_schema.is_none(),
                "historical trusted catalogs cannot claim v3 structural owner schemas"
            );
        } else {
            let pinned = entry
                .owner_schema
                .as_ref()
                .context("v3 owner route must provide independently installed structural schema")?;
            pinned
                .validate(&entry.contract)
                .context("v3 effectful owner schema does not match installed contract")?;
            schema_contracts.register_contract(entry.contract.clone())?;
            anyhow::ensure!(
                owner_schemas
                    .insert(operation.clone(), pinned.clone())
                    .is_none(),
                "duplicate trusted owner structural schema"
            );
            schema_build_ids.insert(operation.clone(), entry.route.build_id.clone());
        }
        if let Some(schema) = &entry.parameter_schema {
            parameter_schemas.insert(operation, schema.clone());
        }
        register_trusted_entry(&mut catalog, entry)?;
    }
    for entry in config.schema_entries {
        validate_build_id(&entry.build_id)?;
        anyhow::ensure!(
            entry.contract.idempotency == IdempotencyPolicy::Pure
                && entry.contract.effects.is_empty(),
            "standalone schema registration is only for pure owners; effectful owners require a checked child route"
        );
        entry
            .owner_schema
            .validate(&entry.contract)
            .context("v3 standalone owner schema does not match installed pure contract")?;
        let operation = entry.contract.reference.clone();
        schema_contracts.register_contract(entry.contract)?;
        schema_build_ids.insert(operation.clone(), entry.build_id);
        anyhow::ensure!(
            owner_schemas
                .insert(operation, entry.owner_schema)
                .is_none(),
            "duplicate trusted owner structural schema"
        );
    }
    Ok(TrustedOwnerCatalog {
        routes: catalog,
        parameter_schemas,
        owner_schemas,
        schema_contracts,
        schema_build_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        ArtifactTypeId, IdempotencyPolicy, OperationId, OperationOwnerId, OperationRef,
        OwnerDtoDescriptorFormatV3, OwnerDtoDescriptorV3, OwnerOperationSchemaV3,
        OWNER_SCHEMA_ATTESTATION_REVISION_V3,
    };
    use serde_json::json;
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
        TrustedRegistrationV1 {
            contract,
            route,
            parameter_schema: None,
            owner_schema: None,
        }
    }

    fn manifest(revision: u16, schema: Option<Value>) -> Vec<u8> {
        let entry = registered_test_owner();
        let mut record = json!({"contract":entry.contract,"route":entry.route});
        if let Some(schema) = schema {
            record["parameter_schema"] = schema;
        }
        serde_json::to_vec(&json!({"schema_revision":revision,"entries":[record]})).unwrap()
    }

    fn pinned_schema(contract: &OperationContract) -> PinnedOwnerOperationSchemaV3 {
        OwnerOperationSchemaV3 {
            schema_revision: OWNER_SCHEMA_ATTESTATION_REVISION_V3,
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            v2_contract_fingerprint: contract.fingerprint().unwrap(),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            failure: OwnerDtoDescriptorV3 {
                schema_id: ArtifactTypeId::from("test.owner.infallible"),
                revision: 1,
                format: OwnerDtoDescriptorFormatV3::JsonSchema2020_12,
                descriptor_bytes: b"false".to_vec(),
            },
        }
        .seal(contract)
        .unwrap()
    }

    #[test]
    fn pure_owner_schema_installs_without_fabricated_effectful_route() {
        let registration = registered_test_owner();
        let pinned = pinned_schema(&registration.contract);
        let trusted_v3 = serde_json::to_vec(&json!({
            "schema_revision": 3,
            "entries": [],
            "schema_entries": [{
                "contract": registration.contract,
                "owner_schema": pinned,
                "build_id": "0123456789abcdef0123456789abcdef01234567"
            }]
        }))
        .unwrap();
        let loaded = decode_trusted_catalog(&trusted_v3).unwrap();
        let reference = registered_test_owner().contract.reference;
        assert_eq!(loaded.routes.catalog().contracts().count(), 0);
        let installed = loaded.owner_schema(&reference).unwrap();
        assert_eq!(
            loaded.owner_schema_build_id(&reference),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        loaded.verify_caller_schema(&reference, installed).unwrap();

        // A caller can mint an internally valid new schema, but may not
        // replace the independently installed owner descriptor.
        let mut forged = installed.schema.clone();
        forged.failure.descriptor_bytes =
            br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}"#
                .to_vec();
        let forged = forged
            .seal(loaded.schema_contracts.contract(&reference).unwrap())
            .unwrap();
        assert!(loaded.verify_caller_schema(&reference, &forged).is_err());
        assert!(loaded
            .verify_caller_schema(
                &OperationRef {
                    id: OperationId::from("unknown"),
                    revision: 1,
                },
                installed
            )
            .is_err());
    }

    #[test]
    fn v3_route_requires_owner_schema_and_ties_it_to_contract() {
        let entry = registered_test_owner();
        let pinned = pinned_schema(&entry.contract);
        let route = entry.route.clone();
        let contract = entry.contract.clone();
        let schema = json!({"type":"object"});
        let good = serde_json::to_vec(&json!({
            "schema_revision": 3,
            "entries": [{
                "contract": contract,
                "route": route,
                "parameter_schema": schema,
                "owner_schema": pinned
            }]
        }))
        .unwrap();
        let loaded = decode_trusted_catalog(&good).unwrap();
        assert_eq!(loaded.routes.catalog().contracts().count(), 1);
        assert_eq!(loaded.owner_schemas.len(), 1);

        let mut no_owner_schema: Value = serde_json::from_slice(&good).unwrap();
        no_owner_schema["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("owner_schema");
        assert!(decode_trusted_catalog(&serde_json::to_vec(&no_owner_schema).unwrap()).is_err());

        let mut forged = no_owner_schema;
        forged["entries"][0]["owner_schema"] = json!(pinned_schema(&entry.contract));
        forged["entries"][0]["owner_schema"]["schema"]["owner"] = json!("caller-not-owner");
        assert!(decode_trusted_catalog(&serde_json::to_vec(&forged).unwrap()).is_err());
    }

    #[test]
    fn legacy_catalogs_cannot_claim_owner_schemas_and_v3_rejects_invalid_builds() {
        let registration = registered_test_owner();
        let pinned = pinned_schema(&registration.contract);
        for version in [1, 2] {
            let mut historical: Value = serde_json::from_slice(&manifest(
                version,
                (version == 2).then(|| json!({"type":"object"})),
            ))
            .unwrap();
            historical["entries"][0]["owner_schema"] = json!(pinned);
            assert!(decode_trusted_catalog(&serde_json::to_vec(&historical).unwrap()).is_err());
        }
        let invalid = serde_json::to_vec(&json!({
            "schema_revision": 3,
            "entries": [],
            "schema_entries": [{
                "contract": registration.contract,
                "owner_schema": pinned,
                "build_id": "caller-selected"
            }]
        }))
        .unwrap();
        assert!(decode_trusted_catalog(&invalid).is_err());
    }

    #[test]
    fn historical_v1_routes_remain_usable_but_have_no_unverified_schema() {
        let loaded = decode_trusted_catalog(&manifest(1, None)).unwrap();
        assert_eq!(loaded.routes.catalog().contracts().count(), 1);
        assert!(loaded.parameter_schemas.is_empty());
        assert!(decode_trusted_catalog(&manifest(1, Some(json!({"type":"object"})))).is_err());
    }

    #[test]
    fn v2_owner_schema_is_required_typed_and_pinned_to_registered_route() {
        let schema = json!({"type":"object","properties":{"top_k":{"type":"integer"}}});
        let loaded = decode_trusted_catalog(&manifest(2, Some(schema.clone()))).unwrap();
        let operation = registered_test_owner().contract.reference;
        assert_eq!(loaded.parameter_schemas[&operation], schema);
        assert_eq!(loaded.routes.catalog().contracts().count(), 1);
        assert!(decode_trusted_catalog(&manifest(2, None)).is_err());
        assert!(decode_trusted_catalog(&manifest(2, Some(json!({"type":"string"})))).is_err());
        assert!(decode_trusted_catalog(&manifest(3, Some(schema))).is_err());
    }

    #[test]
    fn duplicate_or_forged_owner_registration_cannot_appear_in_catalog() {
        let entry = registered_test_owner();
        let payload = json!({"contract":entry.contract,"route":entry.route,"parameter_schema":{"type":"object"}});
        let duplicates = serde_json::to_vec(
            &json!({"schema_revision":2,"entries":[payload.clone(),payload.clone()]}),
        )
        .unwrap();
        assert!(decode_trusted_catalog(&duplicates).is_err());
        let mut forged = payload;
        forged["route"]["contract_hash"] = json!("false-fingerprint");
        let forged = serde_json::to_vec(&json!({"schema_revision":2,"entries":[forged]})).unwrap();
        assert!(decode_trusted_catalog(&forged).is_err());
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
