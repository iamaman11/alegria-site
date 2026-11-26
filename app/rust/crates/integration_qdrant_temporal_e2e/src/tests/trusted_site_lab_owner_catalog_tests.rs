/// Exercise the SAME operator-owned catalog boundary that Site Lab uses for
/// authoring. These are real PostgreSQL/Qdrant contracts and shared artifact
/// types, not mock provider operations or permission to execute either owner.
/// No Temporal, PostgreSQL, or Qdrant container is started by this test.
struct SiteLabV2CiRootCatalog(std::path::PathBuf);

impl SiteLabV2CiRootCatalog {
    fn new() -> Self {
        assert_eq!(env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert_eq!(
            env::var("ALEGRIA_CI_ROOT_CATALOG_INSTALL_OK").as_deref(),
            Ok("true")
        );
        // Unique, exact filename in the established immutable catalog root;
        // no caller-supplied path or mutable DAG input becomes trust authority.
        Self(PathBuf::from(format!(
            "/etc/alegria/site-lab-provider-contracts-{}.json",
            Uuid::new_v4()
        )))
    }

    fn install(&self, document: &serde_json::Value) {
        let source = secret("ci-site-lab-provider-contracts", &document.to_string());
        let installed = Command::new("sudo")
            .args(["-n", "install", "-o", "root", "-g", "root", "-m", "0644"])
            .arg(&source)
            .arg(&self.0)
            .status()
            .expect("disposable CI must permit operator-owned catalog install");
        fs::remove_file(&source).expect("remove exact private manifest staging file");
        assert!(
            installed.success(),
            "cannot install trusted CI provider manifest"
        );
    }
}

impl Drop for SiteLabV2CiRootCatalog {
    fn drop(&mut self) {
        // This guard owns only its uniquely named disposable CI fixture.
        let removed = Command::new("sudo")
            .args(["-n", "rm", "-f", "--"])
            .arg(&self.0)
            .status();
        if !matches!(removed, Ok(status) if status.success()) {
            eprintln!("CI could not remove its isolated Site Lab trust-catalog fixture");
        }
    }
}

#[test]
#[serial]
fn mixed_v2_native_site_lab_catalog_pins_real_pg_qdrant_without_owner_admission() {
    use operation_protocol::{
        Operation, OperationContract, PinnedOwnerRouteV1, PortId, OWNER_OPERATION_CHILD_TYPE_V1,
    };
    use postgres_store::{
        truth_review_effect_v3::PG_OWNER_TASK_QUEUE_V1,
        vector_projection_owner_v1::{PgReadVectorProjectionV1, OUTPUT_PORT},
    };
    use qdrant_contracts::{
        QdrantProducedVectorProjectionContractV1, QDRANT_GENERIC_OWNER_QUEUE_V1,
        QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1,
    };
    use serde_json::json;

    let build_id = env::var("GITHUB_SHA").expect("real owner contracts pin exact CI source SHA");
    assert_eq!(build_id.len(), 40);
    assert!(build_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(build_id.bytes().any(|byte| byte != b'0'));

    let pg = PgReadVectorProjectionV1::contract();
    let qdrant = QdrantProducedVectorProjectionContractV1::contract();
    assert_eq!(pg.reference.id.as_str(), "pg.vector_projection.read");
    assert_eq!(pg.reference.revision, 1);
    assert_eq!(
        qdrant.reference.id.as_str(),
        "qdrant.vector_projection.upsert_produced"
    );
    assert_eq!(qdrant.reference.revision, 1);
    // A native PostgreSQL output can feed a Qdrant input using the same
    // revisioned provider-neutral artifact, without either owner importing
    // the other's implementation or bypassing its own admission checks.
    let produced = pg.outputs.get(&PortId::from(OUTPUT_PORT)).unwrap();
    let consumed = qdrant
        .inputs
        .get(&PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1))
        .unwrap();
    assert_eq!(produced, consumed);
    assert_eq!(produced.id.as_str(), "projection.vector_projection");
    assert_eq!(produced.revision, 1);

    let contract_route = |contract: &OperationContract, queue: &str| PinnedOwnerRouteV1 {
        operation: contract.reference.clone(),
        owner: contract.owner.clone(),
        contract_hash: contract.fingerprint().unwrap(),
        workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
        task_queue: queue.into(),
        build_id: build_id.clone(),
    };
    // These closed schemas deliberately expose ZERO user-editable admission
    // parameters. Actual provider capabilities are described by their real
    // typed ports; publishing a route alone does not supply owner permissions.
    let empty_authoring_schema = json!({"type":"object","additionalProperties":false});
    let entries = [
        (&pg, PG_OWNER_TASK_QUEUE_V1),
        (&qdrant, QDRANT_GENERIC_OWNER_QUEUE_V1),
    ]
    .map(|(contract, queue)| {
        json!({
            "contract":contract,
            "route":contract_route(contract, queue),
            "parameter_schema":empty_authoring_schema
        })
    });
    let manifest = json!({"schema_revision":2,"entries":entries});
    let trusted_file = SiteLabV2CiRootCatalog::new();

    // A caller-writable copy of even a correct signed-shape document remains
    // untrusted, regardless of its contract hashes or schema revisions.
    let untrusted = secret("untrusted-site-lab-owner-manifest", &manifest.to_string());
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&untrusted).is_err());
    fs::remove_file(&untrusted).unwrap();

    trusted_file.install(&manifest);
    let loaded = trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0)
        .expect("root-owned v2 manifest must accept real independently typed owner contracts");
    assert_eq!(loaded.routes.catalog().contracts().count(), 2);
    assert_eq!(loaded.parameter_schemas.len(), 2);
    for (contract, queue) in [
        (&pg, PG_OWNER_TASK_QUEUE_V1),
        (&qdrant, QDRANT_GENERIC_OWNER_QUEUE_V1),
    ] {
        assert_eq!(
            loaded.routes.catalog().contract(&contract.reference),
            Some(contract)
        );
        assert_eq!(
            loaded.parameter_schemas[&contract.reference],
            empty_authoring_schema
        );
        let pinned = loaded
            .routes
            .trusted_snapshot_for([&contract.reference])
            .expect("each installed operation has one version-pinned route");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].task_queue, queue);
        assert_eq!(pinned[0].build_id, build_id);
        assert_eq!(pinned[0].contract_hash, contract.fingerprint().unwrap());
    }

    let mut missing_schema = manifest.clone();
    missing_schema["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("parameter_schema");
    trusted_file.install(&missing_schema);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());

    let mut malformed_schema = manifest.clone();
    malformed_schema["entries"][1]["parameter_schema"] = json!({"type":"string"});
    trusted_file.install(&malformed_schema);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());

    let mut forged_hash = manifest.clone();
    forged_hash["entries"][1]["route"]["contract_hash"] = json!("forged-fingerprint");
    trusted_file.install(&forged_hash);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());

    let mut mismatched_version = manifest.clone();
    mismatched_version["entries"][1]["route"]["operation"]["revision"] = json!(2);
    trusted_file.install(&mismatched_version);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());

    let mut duplicate = manifest.clone();
    let first = duplicate["entries"][0].clone();
    duplicate["entries"][1] = first;
    trusted_file.install(&duplicate);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());

    let mut old_revision = manifest;
    old_revision["schema_revision"] = json!(1);
    trusted_file.install(&old_revision);
    assert!(trusted_owner_catalog::load_trusted_owner_bundle(&trusted_file.0).is_err());
}
