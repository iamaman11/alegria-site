// Contract-only mixed-owner preflight. This does NOT claim a live PG->Qdrant
// producer/consumer E2E: their real artifact schemas are incompatible today.
// The existing native Qdrant Temporal E2E continues to cover real provider I/O.

fn mixed_v6_fixture() -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    let run_id = Uuid::new_v4();
    mixed_v6_fixture_for(
        run_id,
        "ci-mixed-typed-tenant",
        format!("ci-mixed-typed-{run_id}"),
        Uuid::new_v4(),
        Uuid::new_v4(),
        format!("ci-mixed-keyword-{run_id}"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
}

fn mixed_v6_fixture_for(
    run_id: Uuid,
    tenant: &str,
    context: String,
    pg_admission: Uuid,
    qdrant_admission: Uuid,
    keyword: String,
    build_id: &str,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        Operation, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1,
        PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use qdrant_contracts::{QdrantKeywordProjectionContractV2, QDRANT_GENERIC_OWNER_QUEUE_V1};
    use std::collections::{BTreeMap, BTreeSet};
    use truth_domain::review_effect_contract::{
        ReconcileTruthReviewsContractV1, ReconcileTruthReviewsInput,
        TruthReviewScopeArtifactV1,
    };

    let scope = RunScope {
        tenant_id: tenant.into(),
        run_id: run_id.to_string(),
    };
    let pg_contract = ReconcileTruthReviewsContractV1::contract();
    let qdrant_contract = QdrantKeywordProjectionContractV2::contract();

    let mut contracts = ContractCatalog::default();
    contracts.register::<ReconcileTruthReviewsContractV1>().unwrap();
    contracts.register::<QdrantKeywordProjectionContractV2>().unwrap();
    let external_artifacts = BTreeMap::from([
        (
            "pg_scope".into(),
            ExternalArtifact {
                schema: pg_contract.inputs[&operation_protocol::PortId::from("truth_review_scope")].clone(),
                scope: scope.clone(),
                provenance: "ci-pg-scope-v6".into(),
            }),
        (
            "qdrant_command".into(),
            ExternalArtifact {
                schema: qdrant_contract.inputs[&operation_protocol::PortId::from("keyword_command")].clone(),
                scope: scope.clone(),
                provenance: "ci-qdrant-command-v6".into(),
            }),
    ]);
    let nodes = vec![
        OperationNode {
            instance: "pg_review".into(),
            operation: pg_contract.reference.clone(),
            inputs: BTreeMap::from([(
                "truth_review_scope".into(),
                ArtifactBinding::External { artifact: "pg_scope".into() },
            )]),
            outputs: BTreeMap::from([("review_queue_receipt".into(), "pg_receipt".into())]),
            after: BTreeSet::new(),
            permitted_effects: pg_contract.effects.clone(),
        },
        OperationNode {
            instance: "qdrant_projection".into(),
            operation: qdrant_contract.reference.clone(),
            inputs: BTreeMap::from([(
                "keyword_command".into(),
                ArtifactBinding::External { artifact: "qdrant_command".into() },
            )]),
            outputs: BTreeMap::from([("projection_receipt".into(), "qdrant_receipt".into())]),
            after: BTreeSet::from(["pg_review".into()]),
            permitted_effects: qdrant_contract.effects.clone(),
        },
    ];
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: pg_contract.effects.union(&qdrant_contract.effects).cloned().collect(),
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(&contracts)
    .unwrap();

    let pg_command = ReconcileTruthReviewsInput {
        admission_id: pg_admission,
        run_id,
        scope: scope.clone(),
        context_key: context.clone(),
        operation_instance: "pg_review".into(),
        plan_hash: plan.hash.as_str().into(),
    };
    let qdrant_command = qdrant_contracts::QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: qdrant_admission,
        tenant_id: scope.tenant_id.clone(),
        context_key: context.clone(),
        keyword_key: keyword,
        vector: vec![0.123; 1024],
    };
    let payloads: BTreeMap<operation_protocol::ArtifactId, Vec<u8>> = BTreeMap::from([
        ("pg_scope".into(), serde_json::to_vec(&TruthReviewScopeArtifactV1 {
            run_id, context_key: context,
        }).unwrap()),
        ("qdrant_command".into(), serde_json::to_vec(&qdrant_command).unwrap()),
    ]);
    let admitted = plan.external_artifacts.iter().map(|(id, meta)| (
        id.clone(),
        AdmittedArtifactV1 {
            schema: meta.schema.clone(),
            scope: meta.scope.clone(),
            provenance: meta.provenance.clone(),
            payload: payloads[id].clone(),
        },
    )).collect();

    // Routes originate in trusted runner configuration, never in the plan.
    let mut routes = TrustedRouteCatalog::default();
    for (contract, task_queue, build_id) in [
        (pg_contract.clone(), "alegria-pg-owner-v1", build_id),
        (qdrant_contract.clone(), QDRANT_GENERIC_OWNER_QUEUE_V1, build_id),
    ] {
        let route = PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
            task_queue: task_queue.into(),
            build_id: build_id.into(),
        };
        routes.register(contract, route).unwrap();
    }
    let owner_commands = BTreeMap::from([
        ("pg_review".into(), OwnerCommandWireV1 {
            operation: pg_contract.reference.clone(),
            owner: pg_contract.owner.clone(),
            contract_hash: pg_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&pg_command).unwrap(),
        }),
        ("qdrant_projection".into(), OwnerCommandWireV1 {
            operation: qdrant_contract.reference.clone(),
            owner: qdrant_contract.owner.clone(),
            contract_hash: qdrant_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&qdrant_command).unwrap(),
        }),
    ]);
    let pinned_routes = routes
        .trusted_snapshot_for([&pg_contract.reference, &qdrant_contract.reference])
        .unwrap();
    (
        OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted,
            owner_commands,
            pinned_routes,
        },
        routes,
    )
}

/// Distinct V2 shape: two independent effect owner roots, not a hardcoded
/// PG-before-Qdrant chain. Both commands and trusted contracts are the same
/// as their individually admitted V1 counterparts. V2 native admission must
/// separately attest the NEW parent/child Histories before this plan is run.
#[cfg(feature = "e2e")]
fn mixed_v2_independent_fixture_for(
    run_id: Uuid,
    tenant: &str,
    context: String,
    pg_admission: Uuid,
    qdrant_admission: Uuid,
    keyword: String,
    build_id: &str,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExternalArtifact, Operation,
        OwnerCommandWireV1, PinnedOwnerRouteV1, TrustedRouteCatalog,
        OWNER_OPERATION_CHILD_TYPE_V1,
    };
    use postgres_store::vector_projection_owner_v1::{
        PgReadVectorProjectionV1, PgVectorProjectionCommandV1,
    };
    use qdrant_contracts::{QdrantKeywordProjectionContractV2,
        QdrantKeywordProjectionCommandV1, QDRANT_GENERIC_OWNER_QUEUE_V1};
    use std::collections::{BTreeMap, BTreeSet};

    let producer_keyword = format!("pg-source-{keyword}");
    let external_keyword = format!("external-{keyword}");
    let (mut run, _) = mixed_produced_fixture_for(
        run_id, tenant, &context, &producer_keyword,
        pg_admission, qdrant_admission, build_id,
    );
    let pg_contract = PgReadVectorProjectionV1::contract();
    let qdrant_contract = QdrantKeywordProjectionContractV2::contract();
    let mut contracts = ContractCatalog::default();
    contracts.register::<PgReadVectorProjectionV1>().unwrap();
    contracts.register::<QdrantKeywordProjectionContractV2>().unwrap();
    let qdrant_command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: qdrant_admission,
        tenant_id: tenant.to_owned(),
        context_key: context,
        keyword_key: external_keyword,
        vector: vec![0.2375; 1024],
    };
    run.plan.external_artifacts.insert("external_vector_command".into(), ExternalArtifact {
        schema: qdrant_contract.inputs[&"keyword_command".into()].clone(),
        scope: run.plan.scope.clone(),
        provenance: "ci-native-v2-independent-qdrant-external".into(),
    });
    run.admitted.insert("external_vector_command".into(), AdmittedArtifactV1 {
        schema: qdrant_contract.inputs[&"keyword_command".into()].clone(),
        scope: run.plan.scope.clone(),
        provenance: "ci-native-v2-independent-qdrant-external".into(),
        payload: serde_json::to_vec(&qdrant_command).unwrap(),
    });
    let consumer = run.plan.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "qdrant_vector_upsert")
        .expect("independent V2 plan has Qdrant owner node");
    consumer.operation = qdrant_contract.reference.clone();
    consumer.instance = "qdrant_external_upsert".into();
    consumer.inputs = BTreeMap::from([(
        "keyword_command".into(), ArtifactBinding::External {
            artifact: "external_vector_command".into(),
        },
    )]);
    consumer.after = BTreeSet::new();
    consumer.permitted_effects = qdrant_contract.effects.clone();
    run.plan.allowed_effects = pg_contract.effects.union(&qdrant_contract.effects)
        .cloned().collect();
    run.plan = run.plan.seal(&contracts)
        .expect("independent typed PG vector read and Qdrant external upsert must seal");
    let pg_wire = run.owner_commands.get_mut(&"pg_vector_read".into()).unwrap();
    let mut pg_command: PgVectorProjectionCommandV1 =
        serde_json::from_slice(&pg_wire.command_bytes).unwrap();
    pg_command.plan_hash = run.plan.hash.as_str().to_owned();
    pg_wire.command_bytes = serde_json::to_vec(&pg_command).unwrap();
    run.owner_commands.remove(&"qdrant_vector_upsert".into()).unwrap();
    run.owner_commands.insert("qdrant_external_upsert".into(), OwnerCommandWireV1 {
        operation: qdrant_contract.reference.clone(),
        owner: qdrant_contract.owner.clone(),
        contract_hash: qdrant_contract.fingerprint().unwrap(),
        command_bytes: serde_json::to_vec(&qdrant_command).unwrap(),
    });
    let mut routes = TrustedRouteCatalog::default();
    for (contract, task_queue) in [
        (pg_contract.clone(), "alegria-pg-owner-v1"),
        (qdrant_contract.clone(), QDRANT_GENERIC_OWNER_QUEUE_V1),
    ] {
        routes.register(contract.clone(), PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
            task_queue: task_queue.to_owned(),
            build_id: build_id.to_owned(),
        }).unwrap();
    }
    run.pinned_routes = routes.trusted_snapshot_for([
        &pg_contract.reference, &qdrant_contract.reference,
    ]).unwrap();
    run.plan.validate(routes.catalog()).expect("V2 branches are disjoint, typed, and unordered");
    (run, routes)
}

/// A TRUE typed Produced edge: the PostgreSQL owner returns the bytes of a
/// durable precomputed projection under `projection.vector_projection@1`.
/// The Qdrant owner receives those exact output bytes via Temporal's child
/// result -> parent -> next child chain. Its command contains an admission ID
/// only; no duplicate/external vector can silently replace PG's data.
#[cfg(feature = "e2e")]
fn mixed_produced_fixture_for(
    run_id: Uuid,
    tenant: &str,
    context: &str,
    keyword: &str,
    pg_admission: Uuid,
    qdrant_admission: Uuid,
    build_id: &str,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        Operation, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1,
        PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use postgres_store::vector_projection_owner_v1::{
        PgReadVectorProjectionV1, PgVectorProjectionCommandV1,
    };
    use projection_contracts::VectorProjectionLookupV1;
    use qdrant_contracts::{
        QdrantProducedVectorCommandV1, QdrantProducedVectorProjectionContractV1,
        QDRANT_GENERIC_OWNER_QUEUE_V1,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let scope = RunScope { tenant_id: tenant.to_owned(), run_id: run_id.to_string() };
    let pg_contract = PgReadVectorProjectionV1::contract();
    let qdrant_contract = QdrantProducedVectorProjectionContractV1::contract();
    let mut contracts = ContractCatalog::default();
    contracts.register::<PgReadVectorProjectionV1>().unwrap();
    contracts.register::<QdrantProducedVectorProjectionContractV1>().unwrap();
    let lookup_bytes = serde_json::to_vec(&VectorProjectionLookupV1 {
        context_key: context.to_owned(), keyword_key: keyword.to_owned(),
    }).unwrap();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: pg_contract.effects.union(&qdrant_contract.effects).cloned().collect(),
        external_artifacts: BTreeMap::from([(
            "source_lookup".into(),
            ExternalArtifact {
                schema: pg_contract.inputs[&"projection_lookup".into()].clone(),
                scope: scope.clone(),
                provenance: "ci-native-pg-durable-vector-lookup".into(),
            },
        )]),
        pinned_contracts: Vec::new(),
        nodes: vec![
            OperationNode {
                instance: "pg_vector_read".into(),
                operation: pg_contract.reference.clone(),
                inputs: BTreeMap::from([(
                    "projection_lookup".into(),
                    ArtifactBinding::External { artifact: "source_lookup".into() },
                )]),
                outputs: BTreeMap::from([(
                    "vector_projection".into(), "pg_vector_produced".into(),
                )]),
                after: BTreeSet::new(),
                permitted_effects: pg_contract.effects.clone(),
            },
            OperationNode {
                instance: "qdrant_vector_upsert".into(),
                operation: qdrant_contract.reference.clone(),
                inputs: BTreeMap::from([(
                    "vector_projection".into(),
                    ArtifactBinding::Produced {
                        producer: "pg_vector_read".into(),
                        port: "vector_projection".into(),
                        artifact: "pg_vector_produced".into(),
                    },
                )]),
                outputs: BTreeMap::from([(
                    "projection_receipt".into(), "qdrant_projection_receipt".into(),
                )]),
                after: BTreeSet::from(["pg_vector_read".into()]),
                permitted_effects: qdrant_contract.effects.clone(),
            },
        ],
        hash: PlanHash(String::new()),
    }.seal(&contracts).expect("semantic PG projection output matches Qdrant input schema");

    let pg_command = PgVectorProjectionCommandV1 {
        admission_id: pg_admission,
        run_id,
        scope: scope.clone(),
        context_key: context.to_owned(),
        keyword_key: keyword.to_owned(),
        operation_instance: "pg_vector_read".into(),
        plan_hash: plan.hash.as_str().into(),
    };
    let qdrant_command = QdrantProducedVectorCommandV1 {
        schema_revision: 1,
        admission_id: qdrant_admission,
    };
    let mut routes = TrustedRouteCatalog::default();
    for (contract, task_queue) in [
        (pg_contract.clone(), "alegria-pg-owner-v1"),
        (qdrant_contract.clone(), QDRANT_GENERIC_OWNER_QUEUE_V1),
    ] {
        routes.register(contract.clone(), PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
            task_queue: task_queue.to_owned(),
            build_id: build_id.to_owned(),
        }).unwrap();
    }
    let owner_commands = BTreeMap::from([
        ("pg_vector_read".into(), OwnerCommandWireV1 {
            operation: pg_contract.reference.clone(),
            owner: pg_contract.owner.clone(),
            contract_hash: pg_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&pg_command).unwrap(),
        }),
        ("qdrant_vector_upsert".into(), OwnerCommandWireV1 {
            operation: qdrant_contract.reference.clone(),
            owner: qdrant_contract.owner.clone(),
            contract_hash: qdrant_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&qdrant_command).unwrap(),
        }),
    ]);
    let admitted = BTreeMap::from([(
        "source_lookup".into(), AdmittedArtifactV1 {
            schema: pg_contract.inputs[&"projection_lookup".into()].clone(),
            scope,
            provenance: "ci-native-pg-durable-vector-lookup".into(),
            payload: lookup_bytes,
        },
    )]);
    let pinned_routes = routes.trusted_snapshot_for([
        &pg_contract.reference, &qdrant_contract.reference,
    ]).unwrap();
    (OperationDagRunV6 {
        schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        plan,
        admitted,
        owner_commands,
        pinned_routes,
    }, routes)
}

/// Install exactly the owner routes used by this test under the SAME
/// production trust policy as a deployed provider-free DAG parent. Only the
/// disposable GitHub Actions runner may authorize this operation; there is no
/// production-code fallback to a caller-writable JSON configuration path.
#[cfg(feature = "e2e")]
fn mixed_v6_install_ci_trusted_catalog(
    run: &operation_protocol::OperationDagRunV6,
    routes: &operation_protocol::TrustedRouteCatalog,
) -> PathBuf {
    assert_eq!(env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(env::var("ALEGRIA_CI_ROOT_CATALOG_INSTALL_OK").as_deref(), Ok("true"));
    let entries = run.pinned_routes.iter().map(|route| {
        let contract = routes.catalog().contract(&route.operation)
            .expect("test fixture route has an independently registered operation contract");
        serde_json::json!({"contract": contract, "route": route})
    }).collect::<Vec<_>>();
    let document = serde_json::json!({"schema_revision": 1, "entries": entries});
    let source = secret("ci-native-owner-route-catalog", &document.to_string());
    let trusted_path = PathBuf::from("/etc/alegria/owner-routes-v1.json");
    let status = Command::new("sudo")
        .args(["install", "-o", "root", "-g", "root", "-m", "0644"])
        .arg(&source).arg(&trusted_path)
        .status().expect("CI runner must expose constrained root-owned catalog install");
    fs::remove_file(&source).expect("remove ephemeral trust snapshot source file");
    assert!(status.success(), "cannot install root-owned production-trusted route snapshot");
    trusted_path
}

/// Root-owned union is installed once before the native V2 coordinator starts.
/// Its policy admits three independent typed operation contracts, never a
/// special-case PostgreSQL/Qdrant composition hardwired in the coordinator.
#[cfg(feature = "e2e")]
fn mixed_v2_install_ci_trusted_catalog(
    cases: &[(&operation_protocol::OperationDagRunV6, &operation_protocol::TrustedRouteCatalog)],
) -> PathBuf {
    assert_eq!(env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(env::var("ALEGRIA_CI_ROOT_CATALOG_INSTALL_OK").as_deref(), Ok("true"));
    let mut entries = std::collections::BTreeMap::new();
    for (run, routes) in cases {
        for route in &run.pinned_routes {
            let contract = routes.catalog().contract(&route.operation)
                .expect("V2 route has independent trusted operation contract");
            let entry = serde_json::json!({"contract": contract, "route": route});
            if let Some(older) = entries.insert(route.operation.clone(), entry.clone()) {
                assert_eq!(older, entry, "identical operation must have identical V2 route");
            }
        }
    }
    let document = serde_json::json!({"schema_revision": 1,
        "entries": entries.into_values().collect::<Vec<_>>()});
    let source = secret("ci-native-v2-owner-route-catalog", &document.to_string());
    let trusted_path = PathBuf::from("/etc/alegria/owner-routes-v1.json");
    let status = Command::new("sudo")
        .args(["install", "-o", "root", "-g", "root", "-m", "0644"])
        .arg(&source).arg(&trusted_path).status().unwrap();
    fs::remove_file(&source).unwrap();
    assert!(status.success(), "V2 requires root-installed immutable trusted contract union");
    trusted_path
}

/// V2 operator locators inherit the production security boundary: a private
/// regular file is not enough when an ancestor (such as /tmp, mode 1777) is
/// writable by another user. Put disposable CI requests under a trusted
/// checkout ancestor and a dedicated 0700 directory. The old V1 fixture's
/// temp files are deliberately unaffected.
#[cfg(feature = "e2e")]
struct MixedV2PrivateLocatorFiles {
    root: PathBuf,
}

#[cfg(feature = "e2e")]
impl MixedV2PrivateLocatorFiles {
    fn new() -> Self {
        assert_eq!(env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        let workspace = PathBuf::from(env::var("GITHUB_WORKSPACE")
            .expect("CI must provide trusted checkout ancestry for strict V2 operator files"));
        assert!(workspace.is_absolute());
        let root = workspace.join(format!(".ci-native-v2-private-{}", Uuid::new_v4()));
        fs::create_dir(&root).expect("create CI-only private locator directory");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self { root }
    }

    fn request(&self, prefix: &str, contents: &str) -> PathBuf {
        let file = self.root.join(format!("{prefix}-{}.json", Uuid::new_v4()));
        fs::write(&file, contents).expect("write typed operator locator in private directory");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        file
    }
}

#[cfg(feature = "e2e")]
impl Drop for MixedV2PrivateLocatorFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The committed baseline schema deliberately does not auto-enable pilot
/// owner admissions. Apply each immutable opt-in migration in release order
/// to a fresh disposable database, not by granting execution in production.
#[cfg(feature = "e2e")]
async fn mixed_v6_install_native_admissions(pool: &sqlx::PgPool) {
    for (label, statement) in [
        ("Qdrant V2 compatibility", include_str!("../../../../../db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql")),
        ("PG generic owner", include_str!("../../../../../db/migrations/20261125_0018_pg_generic_owner_v1.sql")),
        ("Qdrant generic owner", include_str!("../../../../../db/migrations/20261125_0019_qdrant_generic_owner_v3.sql")),
        ("independent DAG run governance", include_str!("../../../../../db/migrations/20261125_0020_operation_dag_run_governance_v1.sql")),
        ("PG-produced typed projection", include_str!("../../../../../db/migrations/20261125_0021_pg_vector_projection_v1.sql")),
    ] {
        sqlx::raw_sql(statement).execute(pool).await
            .unwrap_or_else(|error| panic!("disposable native {label} migration failed: {error}"));
    }
}

/// V2 is an additive, separately governed admission path. Installing its SQL
/// in an ephemeral native test never upgrades the frozen V1 Histories or grants
/// V2 effects to an old V1 operator/Worker identity.
#[cfg(feature = "e2e")]
async fn mixed_v2_install_native_admissions(pool: &sqlx::PgPool) {
    mixed_v6_install_native_admissions(pool).await;
    for (label, statement) in [
        ("independent V2 generic business-run governance", include_str!(
            "../../../../../db/migrations/20261125_0022_pg_owner_v2_admission.sql")),
        ("independent V2 Qdrant effect admission", include_str!(
            "../../../../../db/migrations/20261125_0023_qdrant_owner_v2_admission.sql")),
    ] {
        sqlx::raw_sql(statement).execute(pool).await
            .unwrap_or_else(|error| panic!("disposable native {label} migration failed: {error}"));
    }
}

#[cfg(feature = "e2e")]
async fn mixed_v6_seed_canonical_truth_source(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    tenant: &str,
    context: &str,
    canonical_source_workflow_run_id: &str,
) {
    let marker = run_id.simple().to_string();
    let source = format!("https://ci-{marker}.example.gov/fees");
    let candidate = format!("mixed_pg_candidate_{marker}");
    let text = "Consular fee is 35 EUR";
    sqlx::query(
        "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ES','tourist','BY')",
    ).bind(context).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)")
        .bind(context).bind(tenant).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO kb.sources(source_key,source_type,source_label,trust_level)
         VALUES ($1,'government','Native mixed owner Truth source',5)",
    ).bind(&source).execute(pool).await.unwrap();
    let page: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages(url,domain,dtype,tenant_id)
         VALUES ($1,$2,'government',$3) RETURNING id",
    ).bind(&source).bind(format!("ci-{marker}.example.gov"))
        .bind(tenant).fetch_one(pool).await.unwrap();
    let section: i64 = sqlx::query_scalar(
        "INSERT INTO raw.sections(page_id,content_md,content_hash,tenant_id)
         VALUES ($1,$2,'mixed-owner-immutable-source',$3) RETURNING id",
    ).bind(page).bind(text).bind(tenant).fetch_one(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO extracted.rule_candidates
           (rule_candidate_id,context_key,raw_section_id,role,concept_canonical_key,
            raw_mention,confidence,evidence_section_id,evidence_quote,span_start,span_end,
            source_key,source_snapshot_hash,llm_provider,llm_model,prompt_version,
            epistemic_status,tenant_id)
         VALUES ($1,$2,$3,'FEE_ITEM','consular_fee','35 EUR',0.98,$3,$4,0,$5,
                 $6,'mixed-owner-immutable-source','chat_manual','gpt-6',
                 'ci_native_mixed@1','needs_hitl',$7)",
    ).bind(&candidate).bind(context).bind(section).bind(text)
        .bind(text.len() as i32).bind(&source).bind(tenant)
        .execute(pool).await.unwrap();
    // The SEO source run is a distinct business execution. Its original
    // workflow_run_id NEVER changes when the separately governed generic
    // Temporal DAG starts; the 0020 issuer must attest the new server History.
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
           (run_id,workflow_run_id,workflow_type,context_key,status,tenant_id)
         VALUES ($1,$2,'seo_site_build',$3,'verifying',$4)",
    ).bind(run_id).bind(canonical_source_workflow_run_id)
        .bind(context).bind(tenant).execute(pool).await.unwrap();
}

#[cfg(feature = "e2e")]
async fn mixed_v7_seed_pg_vector_projection_source(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    tenant: &str,
    context: &str,
    keyword: &str,
    vector: &[f32],
) {
    let projection = projection_contracts::VectorProjectionV1 {
        tenant_id: tenant.to_owned(),
        context_key: context.to_owned(),
        keyword_key: keyword.to_owned(),
        vector: vector.to_vec(),
    };
    projection.validate().expect("disposable PG vector source must be a semantic 1024-vector");
    sqlx::query(
        "INSERT INTO pipeline.vector_projection_sources_v1
           (run_id,tenant_id,context_key,keyword_key,vector)
         VALUES ($1,$2,$3,$4,$5::jsonb)",
    ).bind(run_id).bind(tenant).bind(context).bind(keyword)
        .bind(sqlx::types::Json(&projection.vector))
        .execute(pool).await.expect("persist projection bytes in independent PG owner source");
    let stored: sqlx::types::Json<Vec<f32>> = sqlx::query_scalar(
        "SELECT vector FROM pipeline.vector_projection_sources_v1
         WHERE run_id=$1 AND tenant_id=$2 AND context_key=$3 AND keyword_key=$4",
    ).bind(run_id).bind(tenant).bind(context).bind(keyword)
        .fetch_one(pool).await.expect("read back actual PG-owned projection row");
    assert_eq!(stored.0, vector, "only PG-stored semantic vectors may become Produced data");
}

#[cfg(feature = "e2e")]
struct MixedV6NativeRoles {
    pg_worker_role: String,
    pg_projection_operator_role: String,
    qdrant_operator_role: String,
    pg_worker_url: String,
    pg_operator_url: String,
    pg_projection_operator_url: String,
    qdrant_worker_secret: PathBuf,
    qdrant_operator_secret: PathBuf,
    governance_operator_url: String,
}

/// Distinct SQL LOGIN identities have deliberately non-overlapping powers:
/// only the PG/Qdrant operators INSERT their owner admissions; only Activity
/// LOGINs may consume effect functions. The generic governance issuer is a
/// fifth LOGIN provisioned by the independent runner/operator contract.
#[cfg(feature = "e2e")]
async fn mixed_v6_native_owner_roles(pg: &PostgresHarness, tenant: &str) -> MixedV6NativeRoles {
    let nonce = Uuid::new_v4().simple().to_string();
    let pg_worker = format!("mixed_pg_worker_{nonce}");
    let pg_operator = format!("mixed_pg_operator_{nonce}");
    let pg_projection_operator = format!("mixed_pg_projection_operator_{nonce}");
    let qdrant_worker = format!("mixed_qdrant_worker_{nonce}");
    let qdrant_operator = format!("mixed_qdrant_operator_{nonce}");
    let governance_operator = format!("mixed_dag_issuer_{nonce}");
    const PASSWORD: &str = "disposable_mixed_owner_password";
    // Each dynamic identifier is a fixed ASCII prefix plus UUID hex, and all
    // SQL executes solely against disposable testcontainers PostgreSQL.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {pg_worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {pg_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {pg_projection_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {qdrant_worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {qdrant_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {governance_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {pg_worker},{pg_operator},{pg_projection_operator},{qdrant_worker},{qdrant_operator},{governance_operator};
         GRANT USAGE ON SCHEMA pipeline,extracted,raw,site TO {pg_worker};
         GRANT USAGE ON SCHEMA pipeline,extracted,raw,kb TO {pg_operator};
         GRANT USAGE ON SCHEMA pipeline TO {pg_projection_operator};
         GRANT USAGE ON SCHEMA pipeline TO {qdrant_worker},{qdrant_operator};
         GRANT USAGE ON SCHEMA pipeline TO {governance_operator};
         GRANT SELECT ON pipeline.execution_runs,pipeline.step_executions,
             pipeline.pg_effect_v3_admissions,pipeline.pg_effect_v3_receipts,
             extracted.rule_candidates,raw.sections,raw.pages,site.seo_hitl_tasks
           TO {pg_worker};
         GRANT SELECT ON kb.sources,pipeline.execution_runs,
             extracted.rule_candidates,raw.sections,raw.pages
           TO {pg_operator};
         GRANT SELECT ON pipeline.execution_runs TO {pg_projection_operator};
         GRANT SELECT,INSERT,UPDATE ON pipeline.pg_effect_v3_admissions TO {pg_operator};
         GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID)
           TO {pg_operator};
         GRANT EXECUTE ON FUNCTION pipeline.consume_pg_truth_review_owner_v1(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
             TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA) TO {pg_worker};
         GRANT SELECT ON pipeline.qdrant_projection_v3_admissions TO {qdrant_worker};
         GRANT INSERT ON pipeline.qdrant_projection_v3_admissions TO {qdrant_operator};
         GRANT SELECT ON pipeline.execution_runs TO {qdrant_operator};
         GRANT EXECUTE ON FUNCTION pipeline.begin_qdrant_projection_v3(UUID,BYTEA,TEXT),
             pipeline.confirm_qdrant_projection_v3(UUID,BYTEA,TEXT) TO {qdrant_worker};
         GRANT EXECUTE ON FUNCTION pipeline.verify_qdrant_operator_resignal_v3(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA,BYTEA,TEXT)
           TO {qdrant_operator};
         GRANT EXECUTE ON FUNCTION pipeline.operation_dag_governance_matches_v1(
             UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BOOLEAN)
           TO {pg_worker},{pg_operator},{pg_projection_operator},{qdrant_worker},{qdrant_operator};
         GRANT INSERT ON pipeline.operation_dag_run_governance_v1 TO {governance_operator};
         GRANT EXECUTE ON FUNCTION pipeline.operation_dag_canonical_source_run_id_v1(UUID,TEXT,TEXT)
           TO {governance_operator};"
    ))) .execute(&pg.pool).await.expect("DBA provisions five isolated operator/Activity identities");
    for role in [&pg_worker, &pg_operator, &pg_projection_operator,
        &qdrant_worker, &qdrant_operator, &governance_operator] {
        sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
            .bind(role).bind(tenant).execute(&pg.pool).await.unwrap();
    }
    let restricted_url = |role: &str| {
        let prefix = "postgres://postgres:postgres_password@";
        assert!(pg.database_url.starts_with(prefix), "must use disposable PostgresHarness DSN");
        pg.database_url.replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    MixedV6NativeRoles {
        pg_worker_role: pg_worker.clone(),
        pg_projection_operator_role: pg_projection_operator.clone(),
        qdrant_operator_role: qdrant_operator.clone(),
        pg_worker_url: restricted_url(&pg_worker),
        pg_operator_url: restricted_url(&pg_operator),
        pg_projection_operator_url: restricted_url(&pg_projection_operator),
        qdrant_worker_secret: secret("mixed-qdrant-worker", &restricted_url(&qdrant_worker)),
        qdrant_operator_secret: secret("mixed-qdrant-operator", &restricted_url(&qdrant_operator)),
        governance_operator_url: restricted_url(&governance_operator),
    }
}

/// The source reader and its operator receive only exact PG projection
/// privileges. Qdrant's operator receives one receipt verifier, never SELECT
/// on the source table nor any PostgreSQL owner's raw projection rows.
#[cfg(feature = "e2e")]
async fn mixed_v7_grant_pg_vector_owner_privileges(
    pool: &sqlx::PgPool,
    roles: &MixedV6NativeRoles,
) {
    // Role identifiers are generated from fixed safe ASCII prefixes + UUID
    // hex by mixed_v6_native_owner_roles; the database is disposable CI only.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON pipeline.vector_projection_sources_v1
             TO {pg_worker}, {pg_projection_operator};
         GRANT INSERT ON pipeline.pg_vector_projection_admissions_v1
             TO {pg_projection_operator};
         GRANT EXECUTE ON FUNCTION pipeline.consume_pg_vector_projection_owner_v1(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BYTEA)
             TO {pg_worker};
         GRANT EXECUTE ON FUNCTION pipeline.operation_artifact_receipt_matches_v1(
             UUID,TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,JSONB)
             TO {qdrant_operator};",
        pg_worker = roles.pg_worker_role,
        pg_projection_operator = roles.pg_projection_operator_role,
        qdrant_operator = roles.qdrant_operator_role,
    ))).execute(pool).await.expect("grant only exact PG projection owner capabilities");
}

/// Separate tenant-authorized *business* issuer, platform governance issuer,
/// PG projection issuer, Qdrant V4 operator and Activity roles. None can mint
/// another actor's authority or issue a legacy V1 provider admission.
#[cfg(feature = "e2e")]
struct MixedV2NativeRoles {
    business_issuer_url: String,
    governance_issuer_url: String,
    pg_projection_issuer_url: String,
    qdrant_worker_secret: PathBuf,
    qdrant_operator_secret: PathBuf,
}

#[cfg(feature = "e2e")]
async fn mixed_v2_native_roles(
    pg: &PostgresHarness,
    tenant: &str,
    existing: &MixedV6NativeRoles,
) -> MixedV2NativeRoles {
    let nonce = Uuid::new_v4().simple().to_string();
    let business = format!("v2_business_{nonce}");
    let governance = format!("v2_governance_{nonce}");
    let pg_issuer = format!("v2_pg_issuer_{nonce}");
    let q_worker = format!("v2_q_worker_{nonce}");
    let q_issuer = format!("v2_q_issuer_{nonce}");
    const PASSWORD: &str = "disposable_mixed_owner_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {business} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {governance} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {pg_issuer} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {q_worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {q_issuer} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {business},{governance},{pg_issuer},{q_worker},{q_issuer};
         GRANT USAGE ON SCHEMA pipeline TO {business},{governance},{pg_issuer},{q_worker},{q_issuer};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business};
         GRANT SELECT ON pipeline.operation_dag_business_runs_v1 TO {governance},{pg_issuer},{q_issuer};
         GRANT INSERT ON pipeline.operation_dag_run_governance_v2 TO {governance};
         GRANT SELECT ON pipeline.vector_projection_sources_v1 TO {pg_issuer};
         GRANT INSERT ON pipeline.pg_vector_projection_admissions_v1 TO {pg_issuer};
         GRANT SELECT ON pipeline.vector_projection_sources_v1 TO {pg_worker};
         GRANT EXECUTE ON FUNCTION pipeline.consume_pg_vector_projection_owner_v2(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BYTEA)
             TO {pg_worker};
         GRANT EXECUTE ON FUNCTION pipeline.operation_dag_governance_matches_v2(
             UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BOOLEAN)
             TO {pg_worker},{pg_issuer},{q_worker},{q_issuer};
         GRANT SELECT ON pipeline.qdrant_projection_v4_admissions TO {q_worker};
         GRANT INSERT ON pipeline.qdrant_projection_v4_admissions TO {q_issuer};
         GRANT EXECUTE ON FUNCTION pipeline.begin_qdrant_projection_v4(UUID,BYTEA,TEXT),
             pipeline.confirm_qdrant_projection_v4(UUID,BYTEA,TEXT) TO {q_worker};
         GRANT EXECUTE ON FUNCTION pipeline.verify_qdrant_operator_resignal_v4(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA,BYTEA,TEXT)
             TO {q_issuer};
         GRANT EXECUTE ON FUNCTION pipeline.operation_artifact_receipt_matches_v2(
             UUID,TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,JSONB)
             TO {q_issuer};",
        pg_worker = existing.pg_worker_role,
    ))) .execute(&pg.pool).await.expect("DBA creates strictly disjoint V2 roles");
    for role in [&business, &governance, &pg_issuer, &q_worker, &q_issuer] {
        sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
            .bind(role).bind(tenant).execute(&pg.pool).await.unwrap();
    }
    let url = |role: &str| {
        let prefix = "postgres://postgres:postgres_password@";
        assert!(pg.database_url.starts_with(prefix));
        pg.database_url.replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    MixedV2NativeRoles {
        business_issuer_url: url(&business),
        governance_issuer_url: url(&governance),
        pg_projection_issuer_url: url(&pg_issuer),
        qdrant_worker_secret: secret("ci-v2-qdrant-worker", &url(&q_worker)),
        qdrant_operator_secret: secret("ci-v2-qdrant-operator", &url(&q_issuer)),
    }
}

#[cfg(feature = "e2e")]
async fn mixed_v2_authorize_business_root(
    roles: &MixedV2NativeRoles,
    run: &operation_protocol::OperationDagRunV6,
    context: &str,
    parent_id: &str,
) {
    let issuer = sqlx::PgPool::connect(&roles.business_issuer_url).await
        .expect("independent tenant-scoped V2 business-run issuer login");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '1 hour')"
    ).bind(Uuid::parse_str(&run.plan.scope.run_id).unwrap())
        .bind(&run.plan.scope.tenant_id).bind(context)
        .bind(run.plan.hash.as_str()).bind(parent_id)
        .execute(&issuer).await.expect("only business-run issuer may mint tenant-root authority");
    issuer.close().await;
}

#[cfg(feature = "e2e")]
fn mixed_v6_governance_issuer_catalog(
    run: &operation_protocol::OperationDagRunV6,
    routes: &operation_protocol::TrustedRouteCatalog,
) -> PathBuf {
    #[derive(serde::Serialize)]
    struct ExactOperatorCatalog<'a> {
        contracts: Vec<&'a operation_protocol::OperationContract>,
        routes: &'a [operation_protocol::PinnedOwnerRouteV1],
    }
    let contracts = run.pinned_routes.iter().map(|route| {
        routes.catalog().contract(&route.operation)
            .expect("governance issuer needs independently installed owner contract")
    }).collect::<Vec<_>>();
    // The issuer reads this separately credentialed 0600 file and proves
    // real parent Start bytes against its exact registered contracts/routes.
    let canonical = serde_json::to_string(&ExactOperatorCatalog {
        contracts, routes: &run.pinned_routes,
    }).unwrap();
    // The governance issuer rejects *all* writable parent directories, so
    // even a 0600 file under /tmp (1777) is correctly untrusted. GitHub's
    // checkout ancestry is non-writable to other users; give the operator
    // its own 0700 directory and a canonical 0600 catalog below that root.
    assert_eq!(env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    let private_root = PathBuf::from(env::var("GITHUB_WORKSPACE")
        .expect("CI must provide an absolute trusted checkout directory"))
        .join(format!(".ci-native-dag-governance-{}", Uuid::new_v4()));
    fs::create_dir(&private_root).expect("create private governance operator catalog directory");
    fs::set_permissions(&private_root, fs::Permissions::from_mode(0o700)).unwrap();
    let file = private_root.join("typed-owner-routes.json");
    fs::write(&file, canonical).expect("install exact typed governance catalog");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    file
}

/// REAL multi-owner Temporal admission: one provider-free DAG parent,
/// independent PostgreSQL and Qdrant OwnerInvocationV1 Workflows on disjoint
/// Task Queues, and three distinct operators (governance, PG, Qdrant). This
/// deliberately proves an `after` edge only. PG's review receipt is NEVER
/// misrepresented as a Qdrant vector command or a compatible Produced edge.
#[tokio::test]
#[serial]
async fn mixed_v6_native_pg_then_qdrant_independent_operator_effects() {
    use operation_protocol::{operation_dag_parent_workflow_id, OPERATION_DAG_PARENT_QUEUE,
        OPERATION_DAG_PARENT_TYPE};
    use operation_protocol::OperationDagReceiptV1;
    use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;

    let pg = PostgresHarness::start().await.expect("disposable PG owner Truth database");
    mixed_v6_install_native_admissions(&pg.pool).await;
    let qdrant = QdrantHarness::start().await.expect("real Qdrant REST/gRPC");
    let _provider_env = ScopedEnvironment::apply(&[
        ("QDRANT_URL", qdrant.qdrant_url.as_str()),
        ("QDRANT_REST_URL", qdrant.qdrant_rest_url.as_str()),
    ]);
    let temporal = TemporalHarness::start().await.expect("real Temporal with SQL History");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let pg_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let qdrant_bin = binary("INTEGRATION_QDRANT_EFFECT_WORKER_BIN");
    let build_id = env::var("GITHUB_SHA").expect("native CI pins all exact-SHA worker artifacts");
    assert_eq!(build_id.len(), 40);
    assert!(build_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let tenant = "ci-mixed-native-tenant";
    let run_id = Uuid::new_v4();
    let pg_admission = Uuid::new_v4();
    let qdrant_admission = Uuid::new_v4();
    let context = format!("ci-mixed-native-{run_id}");
    let keyword = format!("mixed-keyword-{qdrant_admission}");
    let source_run_id = Uuid::new_v4().to_string();
    mixed_v6_seed_canonical_truth_source(
        &pg.pool, run_id, tenant, &context, &source_run_id,
    ).await;
    let (run, routes) = mixed_v6_fixture_for(
        run_id, tenant, context.clone(), pg_admission, qdrant_admission,
        keyword.clone(), &build_id,
    );
    let parent_id = operation_dag_parent_workflow_id(&run_id.to_string());
    routes.validate_run_routes(&run).expect("immutable owner Type/Queue/build snapshot");
    assert_eq!(run.plan.nodes[1].after.len(), 1);
    assert!(matches!(run.plan.nodes[1].inputs[&"keyword_command".into()],
        operation_protocol::ArtifactBinding::External { .. }),
        "only independent external vector inputs are actually compatible today");
    let catalog = mixed_v6_install_ci_trusted_catalog(&run, &routes);
    let issuer_catalog = mixed_v6_governance_issuer_catalog(&run, &routes);
    let roles = mixed_v6_native_owner_roles(&pg, tenant).await;

    let parent_process = Command::new(&parent_bin)
        .env("ALLOW_OPERATION_DAG_V1", "true")
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V4", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V5", "false")
        .env("ALLOW_QDRANT_DAG_ORCHESTRATOR_V1", "false")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE)
        .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog)
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("native provider-neutral Temporal parent");
    let _parent_guard = ChildGuard(Some(parent_process));
    let pg_process = Command::new(&pg_bin)
        .env("ALEGRIA_ENABLE_PG_EFFECT_V3", "true")
        .env("ALEGRIA_ENABLE_PG_OWNER_V1", "true")
        .env("PG_EFFECT_WORKER_DATABASE_URL", &roles.pg_worker_url)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("separately credentialed PG Workflow+Activity worker");
    let _pg_guard = ChildGuard(Some(pg_process));
    let qdrant_process = Command::new(&qdrant_bin)
        .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1", "true")
        .env("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V3", "1")
        .env("QDRANT_GENERIC_OWNER_DATABASE_URL_FILE", &roles.qdrant_worker_secret)
        .env("QDRANT_GENERIC_OWNER_TENANT_ID", tenant)
        .env("QDRANT_URL", &qdrant.qdrant_url)
        .env("QDRANT_REST_URL", &qdrant.qdrant_rest_url)
        .env("QDRANT_SKIP_COMPATIBILITY_CHECK", "true")
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("separate Qdrant generic Workflow+Activity worker");
    let _qdrant_guard = ChildGuard(Some(qdrant_process));

    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-native-mixed-v6-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.expect("connect to actual Temporal server");
    let parent = client.start_workflow(
        UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE),
        RawValue::from_value(&run, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE, parent_id.clone()).build(),
    ).await.expect("start owner-neutral Temporal DAG parent");
    let parent_run = parent.info().run_id.clone().expect("server-issued Temporal parent Run ID");
    assert_ne!(parent_id, run_id.to_string());
    assert_ne!(parent_run, source_run_id);
    // The old SEO business execution remains untouched after creating the
    // separate live DAG parent. Only the independent operator may create the
    // tenant-governed 0020 link to exact parent server History.
    let unchanged: String = sqlx::query_scalar(
        "SELECT workflow_run_id FROM pipeline.execution_runs WHERE run_id=$1",
    ).bind(run_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(unchanged, source_run_id);
    let governance_locator = secret("mixed-dag-governance-request", &serde_json::json!({
        "canonical_run_id": run_id,
        "context_key": context,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "expires_in_seconds": 600,
    }).to_string());
    let governance = Command::new(&pg_bin)
        .arg("--operator-admit-dag-run-v1").arg(&governance_locator)
        .env("ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V1", "true")
        .env("OPERATION_DAG_GOVERNANCE_OPERATOR_DATABASE_URL", &roles.governance_operator_url)
        .env("OPERATION_DAG_GOVERNANCE_TENANT_ID", tenant)
        .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_URL", &temporal.temporal_url)
        .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env("OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE", &issuer_catalog)
        .env_remove("DATABASE_URL")
        .output().expect("execute separately privileged platform DAG governance operator");
    assert!(governance.status.success(),
        "exact server-History governance issuer failed: stdout={} stderr={}",
        String::from_utf8_lossy(&governance.stdout),
        String::from_utf8_lossy(&governance.stderr));
    let admitted_runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.operation_dag_run_governance_v1
         WHERE canonical_run_id=$1 AND parent_workflow_run_id=$2",
    ).bind(run_id).bind(&parent_run).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(admitted_runs, 1, "only the independent issuer can bind actual parent History");

    let (pg_child_id, pg_child_run) = timeout(Duration::from_secs(60), async {
        loop {
            let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("actual generic parent Temporal History");
            if let Some(first) = history.iter().find_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else { return None; };
                attrs.workflow_execution.as_ref().map(|value|
                    (value.workflow_id.clone(), value.run_id.clone()))
            }) { break first; }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("generic parent never scheduled independent PG owner");
    assert!(pg_child_id.starts_with("alegria-owner-operation-v1-"));
    let pending_qdrant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_attempts",
    ).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(pending_qdrant, 0, "Qdrant effect cannot precede PG node completion");
    let pg_locator = serde_json::json!({
        "admission_id": pg_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": pg_child_id,
        "child_workflow_run_id": pg_child_run,
        "expires_in_seconds": 300,
    });
    let pg_request = secret("mixed-pg-owner-request", &pg_locator.to_string());
    let wrong_pg_request = secret("mixed-forged-pg-owner-request", &serde_json::json!({
        "admission_id": pg_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": pg_child_id,
        "child_workflow_run_id": Uuid::new_v4().to_string(),
        "expires_in_seconds": 300,
    }).to_string());
    let pg_issuer = |request: &PathBuf| Command::new(&pg_bin)
        .arg("--operator-issue-owner-v1").arg(request)
        .env("ALEGRIA_ENABLE_PG_OWNER_OPERATOR_V1", "true")
        .env("PG_EFFECT_OPERATOR_DATABASE_URL", &roles.pg_operator_url)
        .env("PG_EFFECT_OPERATOR_TENANT_ID", tenant)
        .env("PG_OWNER_EXPECTED_BUILD_ID", &build_id)
        .env("PG_EFFECT_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
        .env("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .output().expect("run independent PG History-proving operator");
    let forged_pg = pg_issuer(&wrong_pg_request);
    assert!(!forged_pg.status.success(), "PG operator accepted forged server child Run ID");
    let pg_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_admissions WHERE admission_id=$1",
    ).bind(pg_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(pg_before, 0, "untrusted child locator must not create PG admission");
    let issued_pg = pg_issuer(&pg_request);
    assert!(issued_pg.status.success(),
        "PG V5 operator rejected real child History: stdout={} stderr={}",
        String::from_utf8_lossy(&issued_pg.stdout),
        String::from_utf8_lossy(&issued_pg.stderr));

    // The second child is started only AFTER the first native PG owner has
    // completed under its separate SQL/History admission, not merely after
    // someone sent a Signal or approved a static DAG edge.
    let (qdrant_child_id, qdrant_child_run) = timeout(Duration::from_secs(80), async {
        loop {
            let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("parent History after real PG owner effect");
            let started: Vec<_> = history.iter().filter_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else { return None; };
                attrs.workflow_execution.as_ref().map(|value|
                    (value.workflow_id.clone(), value.run_id.clone()))
            }).collect();
            if started.len() >= 2 { break started[1].clone(); }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("Qdrant child never started after PG receipt");
    assert!(qdrant_child_id.starts_with("alegria-owner-operation-v1-"));
    assert_ne!(qdrant_child_id, pg_child_id, "owner execution keys must be unique");
    let pg_receipt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_receipts
         WHERE admission_id=$1 AND admission_schema_revision=5",
    ).bind(pg_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(pg_receipt, 1, "Qdrant child started before durable PG rev5 receipt");
    let qdrant_locator = serde_json::json!({
        "admission_id": qdrant_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": qdrant_child_id,
        "child_workflow_run_id": qdrant_child_run,
        "expires_in_seconds": 300,
    });
    let qdrant_request = secret("mixed-qdrant-owner-request", &qdrant_locator.to_string());
    let wrong_qdrant_request = secret("mixed-forged-qdrant-owner-request", &serde_json::json!({
        "admission_id": qdrant_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": qdrant_child_id,
        "child_workflow_run_id": Uuid::new_v4().to_string(),
        "expires_in_seconds": 300,
    }).to_string());
    let qdrant_issuer = |request: &PathBuf, operator_tenant: &str, verb: &str| Command::new(&qdrant_bin)
        .arg(verb).arg(request)
        .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OPERATOR_V1", "true")
        .env("QDRANT_GENERIC_OPERATOR_DATABASE_URL_FILE", &roles.qdrant_operator_secret)
        .env("QDRANT_GENERIC_OPERATOR_TENANT_ID", operator_tenant)
        .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
        .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env("QDRANT_GENERIC_OPERATOR_TRUSTED_BUILD_ID", &build_id)
        .env_remove("DATABASE_URL")
        .output().expect("run independent Qdrant V3 History-proving operator");
    assert!(!qdrant_issuer(&wrong_qdrant_request, tenant, "--operator-issue-v3")
        .status.success(), "forged Qdrant child Run ID was admitted");
    assert!(!qdrant_issuer(&qdrant_request, "foreign-tenant", "--operator-issue-v3")
        .status.success(), "Qdrant V3 operator accepted a cross-tenant issuer");
    assert!(!qdrant_issuer(&qdrant_request, tenant, "--operator-resignal-v3")
        .status.success(), "recovery without committed V3 admission was authorized");
    let before_qdrant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_admissions WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(before_qdrant, 0, "forged/cross-tenant locators cannot authorize provider I/O");
    let issued_qdrant = qdrant_issuer(&qdrant_request, tenant, "--operator-issue-v3");
    assert!(issued_qdrant.status.success(),
        "Qdrant V3 operator rejected exact real Histories: stdout={} stderr={}",
        String::from_utf8_lossy(&issued_qdrant.stdout),
        String::from_utf8_lossy(&issued_qdrant.stderr));

    let completion = timeout(Duration::from_secs(80),
        parent.get_result(WorkflowGetResultOptions::default()))
        .await.expect("native mixed DAG never completed")
        .expect("native mixed owner DAG Workflow failed");
    let receipt: OperationDagReceiptV1 = completion.to_value(&PayloadConverter::serde_json());
    assert_eq!(receipt.plan_hash, run.plan.hash);
    assert_eq!(receipt.order.len(), 2);
    assert_eq!(receipt.order[0].as_str(), "pg_review");
    assert_eq!(receipt.order[1].as_str(), "qdrant_projection");
    assert_eq!(receipt.owner_results.len(), 2);
    assert!(receipt.artifacts.contains_key(&"pg_receipt".into()));
    assert!(receipt.artifacts.contains_key(&"qdrant_receipt".into()));
    let real_history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.expect("read completed real parent History");
    let completed_pg = real_history.iter().find(|event| matches!(
        event.attributes.as_ref(),
        Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(_))
    )).expect("PG child did not complete in real parent History").event_id;
    let second_child_started = real_history.iter().filter(|event| matches!(
        event.attributes.as_ref(),
        Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(_))
    )).nth(1).expect("Qdrant child missing from parent History").event_id;
    assert!(completed_pg < second_child_started,
        "Temporal server History did not prove actual PG completion before Qdrant child start");
    let confirmed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_receipts
         WHERE admission_id=$1 AND exact_provider_readback=TRUE",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(confirmed, 1, "Qdrant child returned without immutable SQL/readback receipt");
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_attempts WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(attempt_count, 1, "durable unknown-outcome barrier must be created once");
    let command: qdrant_contracts::QdrantKeywordProjectionCommandV1 =
        serde_json::from_slice(&run.owner_commands[&"qdrant_projection".into()].command_bytes)
            .expect("exact typed Qdrant command bytes");
    let sha: String = sqlx::query_scalar(
        "SELECT command_sha256 FROM pipeline.qdrant_projection_v3_admissions WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert!(vector_qdrant::projection_commands::projection_readback_exact(
        "seo_keyword_clusters_4", &qdrant_admission.to_string(), "keyword_cluster",
        &format!("tenant/{tenant}/context/{context}/keyword/{keyword}"),
        &command.vector, &serde_json::json!({
            "tenant_id": tenant, "context_key": context, "keyword_key": keyword,
            "operator_admission_id": qdrant_admission.to_string(),
            "approved_command_sha256": sha,
        }),
    ).await.expect("real Qdrant provider readback after two native Temporal owners"));
    let worker_dsn = fs::read_to_string(&roles.qdrant_worker_secret).unwrap();
    let worker_pool = sqlx::PgPool::connect(&worker_dsn).await.unwrap();
    let replay_state: String = sqlx::query_scalar(
        "SELECT pipeline.begin_qdrant_projection_v3($1,$2,$3)",
    ).bind(qdrant_admission)
        .bind(&run.owner_commands[&"qdrant_projection".into()].command_bytes)
        .bind(&receipt.owner_results[&"qdrant_projection".into()].dispatch_execution_key)
        .fetch_one(&worker_pool).await.expect("re-read durable V3 provider attempt state");
    assert_eq!(replay_state, "confirmed", "confirmed retry must never re-upsert provider data");
    assert_eq!(sqlx::query_scalar::<_,i64>(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_attempts WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap(), 1);
    eprintln!("CI_MIXED_V6_NATIVE_PG_THEN_QDRANT_HISTORY_AND_SQL_RECEIPTS=PASS");
}

#[test]
fn mixed_v6_typed_owner_preflight_rejects_cross_tenant_and_fake_produced_dataflow() {
    use operation_protocol::{ArtifactBinding, PlanError, RouteCatalogError};
    use operation_protocol::{GenericDagError, OperationDagMachine};

    let (run, routes) = mixed_v6_fixture();
    let workflow_id = operation_protocol::operation_dag_parent_workflow_id(&run.plan.scope.run_id);
    // Legitimate but LIMITED "after" plan: neither owner consumes the other's
    // output, and the trust-anchored routes are not caller-configurable.
    run.plan.validate(routes.catalog()).expect("genuine owner contracts");
    routes.validate_run_routes(&run).expect("exact owner/queue route snapshot");
    let machine = OperationDagMachine::new(&workflow_id, run.clone(), routes.clone())
        .expect("generic Temporal owner plan accepts two independently routed operations");
    assert_eq!(machine.next_instance().unwrap().as_str(), "pg_review");

    // Even the later node's cross-tenant inputs fail before any effects.
    let mut cross_tenant = run.plan.clone();
    cross_tenant.external_artifacts.get_mut(&"qdrant_command".into())
        .unwrap().scope.tenant_id = "another-tenant".into();
    let cross_tenant = cross_tenant.seal(routes.catalog()).unwrap();
    assert!(matches!(
        cross_tenant.validate(routes.catalog()),
        Err(PlanError::CrossScopeArtifact(artifact)) if artifact.as_str() == "qdrant_command"
    ));
    let mut wrong_scope = run.clone();
    wrong_scope.plan = cross_tenant;
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, wrong_scope, routes.clone()),
        Err(GenericDagError::Plan(PlanError::CrossScopeArtifact(artifact)))
            if artifact.as_str() == "qdrant_command"
    ), "generic runner must reject a later node's foreign artifact before first child");

    // The mutable payload envelope is checked, too: changing only admitted
    // data may not bypass the separately pinned run scope.
    let mut foreign_admission = run.clone();
    foreign_admission.admitted.get_mut(&"qdrant_command".into()).unwrap()
        .scope.tenant_id = "another-tenant".into();
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, foreign_admission, routes.clone()),
        Err(GenericDagError::InvalidArtifact(artifact))
            if artifact.as_str() == "qdrant_command"
    ));

    // No semantic conversion exists: PG Truth receipt is NOT a Qdrant command.
    let mut incompatible = run.plan.clone();
    incompatible.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "qdrant_projection")
        .unwrap().inputs.insert(
            "keyword_command".into(),
            ArtifactBinding::Produced {
                producer: "pg_review".into(),
                port: "review_queue_receipt".into(),
                artifact: "pg_receipt".into(),
            },
        );
    let incompatible = incompatible.seal(routes.catalog()).unwrap();
    assert!(matches!(
        incompatible.validate(routes.catalog()),
        Err(PlanError::ArtifactSchemaMismatch { instance, port, artifact })
            if instance.as_str() == "qdrant_projection"
            && port.as_str() == "keyword_command"
            && artifact.as_str() == "pg_receipt"
    ), "never reinterpret PG receipt as Qdrant command to fake DAG dataflow");
    let mut false_dataflow = run.clone();
    false_dataflow.plan = incompatible;
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, false_dataflow, routes.clone()),
        Err(GenericDagError::Plan(PlanError::ArtifactSchemaMismatch { instance, port, artifact }))
            if instance.as_str() == "qdrant_projection"
            && port.as_str() == "keyword_command"
            && artifact.as_str() == "pg_receipt"
    ));

    let mut forged_route = run.clone();
    forged_route.pinned_routes.last_mut().unwrap().task_queue = "attacker-queue".into();
    assert!(matches!(
        routes.validate_run_routes(&forged_route),
        Err(RouteCatalogError::UntrustedRouteSnapshot)
    ));
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, forged_route, routes),
        Err(GenericDagError::Route(RouteCatalogError::UntrustedRouteSnapshot))
    ));
}

/// A valid business DAG may select Qdrant before PostgreSQL. The registered
/// owner contracts and Task Queues do not encode a provider order: changing
/// the versioned plan, not either owner binary, changes the execution order.
/// This contract-level test does NOT claim parallel execution or native
/// History evidence for the reversed plan; native History tests separately
/// attest actual owner scheduling and effects.
#[test]
fn mixed_v6_owner_order_is_plan_defined_not_pg_qdrant_hardcoded() {
    use operation_protocol::OperationDagMachine;
    use truth_domain::review_effect_contract::ReconcileTruthReviewsInput;

    let (mut run, routes) = mixed_v6_fixture();
    let pg_node = run.plan.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "pg_review").unwrap();
    pg_node.after.insert("qdrant_projection".into());
    let qdrant_node = run.plan.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "qdrant_projection").unwrap();
    qdrant_node.after.clear();
    run.plan = run.plan.seal(routes.catalog()).expect("reversed owner DAG must be valid");

    // PG's owner-local command binds the plan hash, so the new plan is
    // admitted only when that command independently pins the new hash.
    let pg_wire = run.owner_commands.get_mut(&"pg_review".into()).unwrap();
    let mut pg_command: ReconcileTruthReviewsInput =
        serde_json::from_slice(&pg_wire.command_bytes).unwrap();
    pg_command.plan_hash = run.plan.hash.as_str().to_owned();
    pg_wire.command_bytes = serde_json::to_vec(&pg_command).unwrap();

    run.plan.validate(routes.catalog()).expect("Qdrant then PG is a legal DAG");
    let workflow_id = operation_protocol::operation_dag_parent_workflow_id(&run.plan.scope.run_id);
    let machine = OperationDagMachine::new(&workflow_id, run, routes)
        .expect("the generic parent must not bake in PG before Qdrant");
    assert_eq!(machine.next_instance().unwrap().as_str(), "qdrant_projection");
}

#[test]
fn mixed_v7_real_vector_produced_contract_is_typed_before_effects() {
    use operation_protocol::{ArtifactBinding, PlanError};
    use operation_protocol::{GenericDagError, OperationDagMachine};

    let run_id = Uuid::new_v4();
    let (run, routes) = mixed_produced_fixture_for(
        run_id, "ci-produced-tenant", "ci-produced-context", "ci-vector-keyword",
        Uuid::new_v4(), Uuid::new_v4(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert_eq!(run.plan.external_artifacts.len(), 1,
        "Qdrant may not have a separately supplied vector command artifact");
    let consumer = run.plan.nodes.iter()
        .find(|node| node.instance.as_str() == "qdrant_vector_upsert").unwrap();
    assert!(matches!(consumer.inputs[&"vector_projection".into()],
        ArtifactBinding::Produced { ref producer, ref port, ref artifact }
        if producer.as_str() == "pg_vector_read"
        && port.as_str() == "vector_projection"
        && artifact.as_str() == "pg_vector_produced"));
    run.plan.validate(routes.catalog()).expect("matched semantically typed owner contracts");
    let workflow_id = operation_protocol::operation_dag_parent_workflow_id(&run_id.to_string());
    let machine = OperationDagMachine::new(&workflow_id, run.clone(), routes.clone())
        .expect("valid Provenance Production plan must preflight before owner effects");
    assert_eq!(machine.next_instance().unwrap().as_str(), "pg_vector_read");

    // The one admitted External is a lookup, not a Qdrant-ready vector. A
    // malicious client cannot swap it onto the consumer's Produced port.
    let mut malformed = run.plan.clone();
    malformed.nodes.iter_mut().find(|node|
        node.instance.as_str() == "qdrant_vector_upsert").unwrap()
        .inputs.insert("vector_projection".into(),
            ArtifactBinding::External { artifact: "source_lookup".into() });
    let malformed = malformed.seal(routes.catalog()).unwrap();
    assert!(matches!(malformed.validate(routes.catalog()),
        Err(PlanError::ArtifactSchemaMismatch { instance, port, artifact })
        if instance.as_str() == "qdrant_vector_upsert"
        && port.as_str() == "vector_projection"
        && artifact.as_str() == "source_lookup"));
    let mut false_external = run.clone();
    false_external.plan = malformed;
    assert!(matches!(OperationDagMachine::new(&workflow_id, false_external, routes.clone()),
        Err(GenericDagError::Plan(PlanError::ArtifactSchemaMismatch { .. }))));

    let mut foreign_lookup = run;
    foreign_lookup.admitted.get_mut(&"source_lookup".into()).unwrap()
        .scope.tenant_id = "foreign-tenant".into();
    assert!(matches!(OperationDagMachine::new(&workflow_id, foreign_lookup, routes),
        Err(GenericDagError::InvalidArtifact(id)) if id.as_str() == "source_lookup"));
}

/// Fully native positive Produced path with independent SQL/History authority:
/// PG owns the source bytes, Temporal receives them as a typed owner result,
/// and only Temporal delivers them as a Produced input to Qdrant's owner.
/// Neither owner has the other owner's provider credentials or SDK/command.
#[tokio::test]
#[serial]
async fn mixed_v7_native_temporal_produced_pg_vector_to_qdrant_readback() {
    use operation_protocol::{operation_dag_parent_workflow_id, OPERATION_DAG_PARENT_QUEUE,
        OPERATION_DAG_PARENT_TYPE};
    use projection_contracts::VectorProjectionV1;
    use operation_protocol::OperationDagReceiptV1;
    use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;

    let pg = PostgresHarness::start().await.expect("disposable PG artifact source");
    mixed_v6_install_native_admissions(&pg.pool).await;
    let qdrant = QdrantHarness::start().await.expect("real Qdrant provider");
    let _provider_env = ScopedEnvironment::apply(&[
        ("QDRANT_URL", qdrant.qdrant_url.as_str()),
        ("QDRANT_REST_URL", qdrant.qdrant_rest_url.as_str()),
    ]);
    let temporal = TemporalHarness::start().await.expect("real Temporal History server");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let pg_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let qdrant_bin = binary("INTEGRATION_QDRANT_EFFECT_WORKER_BIN");
    let build_id = env::var("GITHUB_SHA").expect("CI exact-source owner build ID");
    assert_eq!(build_id.len(), 40);

    let run_id = Uuid::new_v4();
    let pg_admission = Uuid::new_v4();
    let qdrant_admission = Uuid::new_v4();
    let tenant = "ci-produced-vector-tenant";
    let context = format!("ci-produced-vector-{run_id}");
    let keyword = format!("pg-produced-keyword-{qdrant_admission}");
    let vector = (0..1024).map(|index| 0.125 + index as f32 / 16384.0)
        .collect::<Vec<_>>();
    let source_run_id = Uuid::new_v4().to_string();
    mixed_v6_seed_canonical_truth_source(&pg.pool, run_id, tenant, &context, &source_run_id).await;
    mixed_v7_seed_pg_vector_projection_source(
        &pg.pool, run_id, tenant, &context, &keyword, &vector,
    ).await;
    let (run, routes) = mixed_produced_fixture_for(
        run_id, tenant, &context, &keyword, pg_admission, qdrant_admission, &build_id,
    );
    let parent_id = operation_dag_parent_workflow_id(&run_id.to_string());
    routes.validate_run_routes(&run).expect("exact route registry for both owners");
    assert_eq!(run.plan.external_artifacts.len(), 1,
        "Qdrant vector must come from PG owner result, never from External data");
    let catalog = mixed_v6_install_ci_trusted_catalog(&run, &routes);
    let issuer_catalog = mixed_v6_governance_issuer_catalog(&run, &routes);
    let roles = mixed_v6_native_owner_roles(&pg, tenant).await;
    mixed_v7_grant_pg_vector_owner_privileges(&pg.pool, &roles).await;

    let parent_process = Command::new(&parent_bin)
        .env("ALLOW_OPERATION_DAG_V1", "true")
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V4", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V5", "false")
        .env("ALLOW_QDRANT_DAG_ORCHESTRATOR_V1", "false")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE)
        .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog)
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("start provider-free Temporal DAG coordinator");
    let _parent_guard = ChildGuard(Some(parent_process));
    let pg_process = Command::new(&pg_bin)
        .env("ALEGRIA_ENABLE_PG_EFFECT_V3", "true")
        .env("ALEGRIA_ENABLE_PG_OWNER_V1", "true")
        .env("ALEGRIA_ENABLE_PG_PROJECTION_OWNER_V1", "true")
        .env("PG_EFFECT_WORKER_DATABASE_URL", &roles.pg_worker_url)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("PG-only source owner worker");
    let _pg_guard = ChildGuard(Some(pg_process));
    let qdrant_process = Command::new(&qdrant_bin)
        .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1", "true")
        .env("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V3", "1")
        .env("QDRANT_GENERIC_OWNER_DATABASE_URL_FILE", &roles.qdrant_worker_secret)
        .env("QDRANT_GENERIC_OWNER_TENANT_ID", tenant)
        .env("QDRANT_URL", &qdrant.qdrant_url)
        .env("QDRANT_REST_URL", &qdrant.qdrant_rest_url)
        .env("QDRANT_SKIP_COMPATIBILITY_CHECK", "true")
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("independent Qdrant Produced consumer worker");
    let _qdrant_guard = ChildGuard(Some(qdrant_process));

    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-native-produced-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.expect("connect to real Temporal");
    let parent = client.start_workflow(
        UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE),
        RawValue::from_value(&run, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE, parent_id.clone()).build(),
    ).await.expect("start typed Produced DAG parent");
    let parent_run = parent.info().run_id.clone().expect("server parent Run ID");
    let governance_request = secret("ci-produced-governance", &serde_json::json!({
        "canonical_run_id": run_id,
        "context_key": context,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "expires_in_seconds": 600,
    }).to_string());
    let issued_governance = Command::new(&pg_bin)
        .arg("--operator-admit-dag-run-v1").arg(&governance_request)
        .env("ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V1", "true")
        .env("OPERATION_DAG_GOVERNANCE_OPERATOR_DATABASE_URL", &roles.governance_operator_url)
        .env("OPERATION_DAG_GOVERNANCE_TENANT_ID", tenant)
        .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_URL", &temporal.temporal_url)
        .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env("OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE", &issuer_catalog)
        .env_remove("DATABASE_URL").output().unwrap();
    assert!(issued_governance.status.success(),
        "independent DAG governance: {}", String::from_utf8_lossy(&issued_governance.stderr));

    let (pg_child_id, pg_child_run) = timeout(Duration::from_secs(60), async {
        loop {
            let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("server parent History");
            if let Some(child) = history.iter().find_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else { return None; };
                attrs.workflow_execution.as_ref().map(|execution|
                    (execution.workflow_id.clone(), execution.run_id.clone()))
            }) { break child; }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("PG produced-source Child Workflow never started");
    let pg_request = secret("ci-produced-pg-admission", &serde_json::json!({
        "admission_id": pg_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": pg_child_id,
        "child_workflow_run_id": pg_child_run,
        "expires_in_seconds": 300,
    }).to_string());
    let admitted_pg = Command::new(&pg_bin)
        .arg("--operator-issue-pg-projection-v1").arg(&pg_request)
        .env("ALEGRIA_ENABLE_PG_OWNER_OPERATOR_V1", "true")
        .env("PG_EFFECT_OPERATOR_DATABASE_URL", &roles.pg_projection_operator_url)
        .env("PG_EFFECT_OPERATOR_TENANT_ID", tenant)
        .env("PG_OWNER_EXPECTED_BUILD_ID", &build_id)
        .env("PG_EFFECT_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
        .env("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL").output().unwrap();
    assert!(admitted_pg.status.success(),
        "PG source owner admission: stdout={} stderr={}",
        String::from_utf8_lossy(&admitted_pg.stdout),
        String::from_utf8_lossy(&admitted_pg.stderr));

    let (q_child_id, q_child_run) = timeout(Duration::from_secs(80), async {
        loop {
            let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("server parent History");
            let started = history.iter().filter_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else { return None; };
                attrs.workflow_execution.as_ref().map(|execution|
                    (execution.workflow_id.clone(), execution.run_id.clone()))
            }).collect::<Vec<_>>();
            if started.len() == 2 { break started[1].clone(); }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("Qdrant consumer did not wait for PG Produced result");
    let qdrant_request = secret("ci-produced-qdrant-admission", &serde_json::json!({
        "admission_id": qdrant_admission,
        "canonical_run_id": run_id,
        "parent_workflow_id": parent_id,
        "parent_workflow_run_id": parent_run,
        "child_workflow_id": q_child_id,
        "child_workflow_run_id": q_child_run,
        "expires_in_seconds": 300,
    }).to_string());
    let qdrant_issuer = |request: &PathBuf, operator_tenant: &str| Command::new(&qdrant_bin)
        .arg("--operator-issue-v3").arg(request)
        .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OPERATOR_V1", "true")
        .env("QDRANT_GENERIC_OPERATOR_DATABASE_URL_FILE", &roles.qdrant_operator_secret)
        .env("QDRANT_GENERIC_OPERATOR_TENANT_ID", operator_tenant)
        .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
        .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env("QDRANT_GENERIC_OPERATOR_TRUSTED_BUILD_ID", &build_id)
        .env_remove("DATABASE_URL").output().unwrap();
    assert!(!qdrant_issuer(&qdrant_request, "foreign-tenant").status.success(),
        "foreign tenant must not admit PG-produced projection into Qdrant");
    let admitted_qdrant = qdrant_issuer(&qdrant_request, tenant);
    assert!(admitted_qdrant.status.success(),
        "independent Qdrant Produced admission: stdout={} stderr={}",
        String::from_utf8_lossy(&admitted_qdrant.stdout),
        String::from_utf8_lossy(&admitted_qdrant.stderr));

    let completion = timeout(Duration::from_secs(80),
        parent.get_result(WorkflowGetResultOptions::default()))
        .await.expect("Produced DAG never completed")
        .expect("typed Produced Workflow returned failure");
    let receipt: OperationDagReceiptV1 = completion.to_value(&PayloadConverter::serde_json());
    assert_eq!(receipt.order.len(), 2);
    assert_eq!(receipt.order[0].as_str(), "pg_vector_read");
    assert_eq!(receipt.order[1].as_str(), "qdrant_vector_upsert");
    let produced = receipt.artifacts.get(&"pg_vector_produced".into())
        .expect("Temporal parent must publish PG-produced typed artifact");
    let actual_projection: VectorProjectionV1 = serde_json::from_slice(&produced.payload)
        .expect("PG owner must return exact typed projection bytes");
    assert_eq!(actual_projection.vector, vector);
    assert_eq!(actual_projection.tenant_id, tenant);
    assert_eq!(actual_projection.context_key, context);
    assert_eq!(actual_projection.keyword_key, keyword);
    assert!(receipt.produced_evidence.contains_key(&"pg_vector_produced".into()));
    let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.expect("completed real Temporal parent History");
    let completed_pg = history.iter().find(|event| matches!(
        event.attributes.as_ref(),
        Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(_))
    )).expect("PG ChildCompleted event missing").event_id;
    let started_qdrant = history.iter().filter(|event| matches!(
        event.attributes.as_ref(),
        Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(_))
    )).nth(1).expect("Qdrant ChildStarted event missing").event_id;
    assert!(completed_pg < started_qdrant,
        "Temporal History must show PG produced output before Qdrant consumer start");
    let admitted_vector_sha: String = sqlx::query_scalar(
        "SELECT command_sha256 FROM pipeline.qdrant_projection_v3_admissions WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert!(vector_qdrant::projection_commands::projection_readback_exact(
        "seo_keyword_clusters_4", &qdrant_admission.to_string(), "keyword_cluster",
        &format!("tenant/{tenant}/context/{context}/keyword/{keyword}"),
        &vector, &serde_json::json!({
            "tenant_id": tenant, "context_key": context, "keyword_key": keyword,
            "operator_admission_id": qdrant_admission.to_string(),
            "approved_command_sha256": admitted_vector_sha,
        }),
    ).await.expect("read exact PG-owned vector through Qdrant REST provider"));
    let qdrant_effect_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v3_attempts WHERE admission_id=$1",
    ).bind(qdrant_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(qdrant_effect_count, 1, "Produced replay cannot duplicate Qdrant write");
    eprintln!("CI_MIXED_V7_NATIVE_TRUE_PRODUCED_PG_TEMPORAL_QDRANT=PASS");
}

/// Two fundamentally different REAL V2 DAGs on independent, tenant-scoped
/// PostgreSQL and Qdrant owner queues. The first transports PG's durable
/// typed Produced vector through two Temporal effect Child Histories; the
/// second fans out PG and Qdrant operations without any dependency. Both
/// undergo the universal ValidateOnly Child barrier before effects, and every
/// admission independently attests server-issued Workflow/Run identities.
#[tokio::test]
#[serial]
async fn mixed_v2_native_validated_produced_and_parallel_owner_effects() {
    use operation_protocol::{operation_dag_parent_workflow_id_v2,
        OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2};
    use operation_protocol::OperationDagReceiptV1;
    use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;

    let pg = PostgresHarness::start().await.expect("real V2 PostgreSQL source/authority");
    mixed_v2_install_native_admissions(&pg.pool).await;
    let qdrant = QdrantHarness::start().await.expect("real V2 Qdrant readback provider");
    let _provider_env = ScopedEnvironment::apply(&[
        ("QDRANT_URL", qdrant.qdrant_url.as_str()),
        ("QDRANT_REST_URL", qdrant.qdrant_rest_url.as_str()),
    ]);
    let temporal = TemporalHarness::start().await.expect("real V2 Temporal server/History");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let pg_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let qdrant_bin = binary("INTEGRATION_QDRANT_EFFECT_WORKER_BIN");
    let build_id = env::var("GITHUB_SHA").expect("exact CI source identity");
    assert_eq!(build_id.len(), 40);
    assert!(build_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let tenant = "ci-generic-v2-mixed-tenant";
    let produced_id = Uuid::new_v4();
    let parallel_id = Uuid::new_v4();
    let invalid_id = Uuid::new_v4();
    let foreign_id = Uuid::new_v4();
    let contexts = [format!("native-v2-produced-{produced_id}"),
        format!("native-v2-parallel-{parallel_id}"),
        format!("native-v2-invalid-{invalid_id}"),
        format!("native-v2-foreign-{foreign_id}")];
    for context in &contexts {
        sqlx::query(
            "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
             VALUES ($1,'ES','tourist','BY')"
        ).bind(context).execute(&pg.pool).await.unwrap();
        sqlx::query("INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)")
            .bind(context).bind(tenant).execute(&pg.pool).await.unwrap();
    }
    let produced_pg_admission = Uuid::new_v4();
    let produced_q_admission = Uuid::new_v4();
    let parallel_pg_admission = Uuid::new_v4();
    let parallel_q_admission = Uuid::new_v4();
    let invalid_pg_admission = Uuid::new_v4();
    let invalid_q_admission = Uuid::new_v4();
    let foreign_pg_admission = Uuid::new_v4();
    let foreign_q_admission = Uuid::new_v4();
    let produced_keyword = format!("v2-pg-produced-{produced_q_admission}");
    let parallel_keyword = format!("v2-pg-parallel-{parallel_q_admission}");
    let invalid_keyword = format!("v2-invalid-{invalid_q_admission}");
    let foreign_keyword = format!("v2-foreign-{foreign_q_admission}");
    let produced_vector = (0..1024).map(|i| 0.125 + i as f32 / 16384.0)
        .collect::<Vec<_>>();
    let parallel_vector = vec![0.375_f32; 1024];
    let (produced_run, produced_routes) = mixed_produced_fixture_for(
        produced_id, tenant, &contexts[0], &produced_keyword,
        produced_pg_admission, produced_q_admission, &build_id,
    );
    let (parallel_run, parallel_routes) = mixed_v2_independent_fixture_for(
        parallel_id, tenant, contexts[1].clone(), parallel_pg_admission,
        parallel_q_admission, parallel_keyword.clone(), &build_id,
    );
    let (mut invalid_run, invalid_routes) = mixed_v2_independent_fixture_for(
        invalid_id, tenant, contexts[2].clone(), invalid_pg_admission,
        invalid_q_admission, invalid_keyword.clone(), &build_id,
    );
    // A structurally valid plan with a *semantic* bad owner DTO: only the
    // Qdrant ValidateChild may reject it. PG's earlier root is never started.
    let bad_wire = invalid_run.owner_commands.get_mut(&"qdrant_external_upsert".into()).unwrap();
    let mut bad_command: qdrant_contracts::QdrantKeywordProjectionCommandV1 =
        serde_json::from_slice(&bad_wire.command_bytes).unwrap();
    bad_command.vector.pop();
    let bad_bytes = serde_json::to_vec(&bad_command).unwrap();
    bad_wire.command_bytes = bad_bytes.clone();
    invalid_run.admitted.get_mut(&"external_vector_command".into()).unwrap().payload = bad_bytes;

    // A different semantic fault: the JSON shape and vector are valid, but
    // the owner's concrete DTO claims another tenant than the immutable plan.
    // This must also fail in the owner's pure ValidateChild, without starting
    // the otherwise valid independent PostgreSQL operation.
    let (mut foreign_run, foreign_routes) = mixed_v2_independent_fixture_for(
        foreign_id, tenant, contexts[3].clone(), foreign_pg_admission,
        foreign_q_admission, foreign_keyword.clone(), &build_id,
    );
    let foreign_wire = foreign_run.owner_commands
        .get_mut(&"qdrant_external_upsert".into()).unwrap();
    let mut foreign_command: qdrant_contracts::QdrantKeywordProjectionCommandV1 =
        serde_json::from_slice(&foreign_wire.command_bytes).unwrap();
    foreign_command.tenant_id = "foreign-v2-tenant".into();
    let foreign_bytes = serde_json::to_vec(&foreign_command).unwrap();
    foreign_wire.command_bytes = foreign_bytes.clone();
    foreign_run.admitted.get_mut(&"external_vector_command".into()).unwrap().payload = foreign_bytes;

    mixed_v7_seed_pg_vector_projection_source(
        &pg.pool, produced_id, tenant, &contexts[0], &produced_keyword, &produced_vector,
    ).await;
    mixed_v7_seed_pg_vector_projection_source(
        &pg.pool, parallel_id, tenant, &contexts[1],
        &format!("pg-source-{parallel_keyword}"), &parallel_vector,
    ).await;
    // Both rejected plans have a real, readable PG-owned source. An absence
    // of effects therefore cannot be explained by a missing source row.
    for (run_id, context, keyword) in [
        (invalid_id, &contexts[2], &invalid_keyword),
        (foreign_id, &contexts[3], &foreign_keyword),
    ] {
        mixed_v7_seed_pg_vector_projection_source(
            &pg.pool, run_id, tenant, context,
            &format!("pg-source-{keyword}"), &parallel_vector,
        ).await;
    }
    for (run, routes) in [(&produced_run, &produced_routes),
        (&parallel_run, &parallel_routes), (&invalid_run, &invalid_routes),
        (&foreign_run, &foreign_routes)] {
        routes.validate_run_routes(run).expect("V2 trusted per-owner routes");
    }
    let catalog = mixed_v2_install_ci_trusted_catalog(&[
        (&produced_run, &produced_routes), (&parallel_run, &parallel_routes),
        (&invalid_run, &invalid_routes), (&foreign_run, &foreign_routes),
    ]);
    let produced_issuer_catalog = mixed_v6_governance_issuer_catalog(&produced_run, &produced_routes);
    let parallel_issuer_catalog = mixed_v6_governance_issuer_catalog(&parallel_run, &parallel_routes);
    let private_locators = MixedV2PrivateLocatorFiles::new();
    let base_roles = mixed_v6_native_owner_roles(&pg, tenant).await;
    let roles = mixed_v2_native_roles(&pg, tenant, &base_roles).await;

    let _parent_guard = ChildGuard(Some(Command::new(&parent_bin)
        .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE_V2)
        .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog)
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("V2 owner-neutral Temporal parent worker")));
    let _pg_guard = ChildGuard(Some(Command::new(&pg_bin)
        .env("ALEGRIA_ENABLE_PG_EFFECT_V3", "true")
        .env("ALEGRIA_ENABLE_PG_OWNER_V1", "true")
        .env("ALEGRIA_ENABLE_PG_PROJECTION_OWNER_V1", "true")
        .env("PG_EFFECT_WORKER_DATABASE_URL", &base_roles.pg_worker_url)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("PG typed ValidateChild and effect Child worker")));
    let _qdrant_guard = ChildGuard(Some(Command::new(&qdrant_bin)
        .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OWNER_V1", "true")
        .env("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V4", "1")
        .env("QDRANT_GENERIC_OWNER_DATABASE_URL_FILE", &roles.qdrant_worker_secret)
        .env("QDRANT_GENERIC_OWNER_TENANT_ID", tenant)
        .env("QDRANT_URL", &qdrant.qdrant_url)
        .env("QDRANT_REST_URL", &qdrant.qdrant_rest_url)
        .env("QDRANT_SKIP_COMPATIBILITY_CHECK", "true")
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("Qdrant typed ValidateChild and effect Child worker")));
    let client = connect_client(&temporal.temporal_url,
        format!("ci-mixed-v2-{}", Uuid::new_v4()), &temporal.namespace)
        .await.expect("connect actual Temporal History server");

    for (case, run, routes, run_id, context, pg_admission, q_admission, issuer_catalog,
         q_keyword, q_vector, pg_keyword, pg_vector, independent) in [
        ("produced", &produced_run, &produced_routes, produced_id, &contexts[0],
            produced_pg_admission, produced_q_admission, &produced_issuer_catalog,
            produced_keyword.clone(), produced_vector.clone(), produced_keyword.clone(),
            produced_vector.clone(), false),
        ("parallel", &parallel_run, &parallel_routes, parallel_id, &contexts[1],
            parallel_pg_admission, parallel_q_admission, &parallel_issuer_catalog,
            format!("external-{parallel_keyword}"), vec![0.2375_f32; 1024],
            format!("pg-source-{parallel_keyword}"), parallel_vector.clone(), true),
    ] {
        let parent_id = operation_dag_parent_workflow_id_v2(&run_id.to_string());
        assert_ne!(parent_id,
            operation_protocol::operation_dag_parent_workflow_id(&run_id.to_string()),
            "V1/V2 parent Workflow identities must never collide");
        let parent = client.start_workflow(
            UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
            RawValue::from_value(run, &PayloadConverter::serde_json()),
            WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, parent_id.clone()).build(),
        ).await.expect("start actual V2 Temporal typed parent");
        let parent_run = parent.info().run_id.clone().expect("server V2 parent Run ID");
        mixed_v2_authorize_business_root(&roles, run, context, &parent_id).await;
        let governance_request = private_locators.request("v2-governance-locator", &serde_json::json!({
            "canonical_run_id": run_id, "context_key": context,
            "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
            "expires_in_seconds": 600,
        }).to_string());
        let governance = Command::new(&pg_bin)
            .arg("--operator-admit-dag-run-v2").arg(&governance_request)
            .env("ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V2", "true")
            .env("OPERATION_DAG_GOVERNANCE_V2_OPERATOR_DATABASE_URL", &roles.governance_issuer_url)
            .env("OPERATION_DAG_GOVERNANCE_TENANT_ID", tenant)
            .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_URL", &temporal.temporal_url)
            .env("OPERATION_DAG_GOVERNANCE_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env("OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE", issuer_catalog)
            .env_remove("DATABASE_URL").output().unwrap();
        assert!(governance.status.success(), "{case}: V2 governance: stdout={} stderr={}",
            String::from_utf8_lossy(&governance.stdout),
            String::from_utf8_lossy(&governance.stderr));
        let expected_effect_count = if independent { 2 } else { 1 };
        let (effect_children, validated_through, first_effect_event) =
            timeout(Duration::from_secs(90), async {
                loop {
                    let history = parent.fetch_history(
                        WorkflowFetchHistoryOptions::builder().build())
                        .into_events().await.expect("V2 authoritative parent History");
                    let mut validation_started = 0;
                    let mut validation_completed = Vec::new();
                    let mut effects = Vec::new();
                    for event in &history {
                        match &event.attributes {
                            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                                if a.workflow_id.starts_with("alegria-owner-validate-v1-") => {
                                    validation_started += 1;
                                },
                            Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a)) => {
                                // Child completion event identifies initiated ID; all
                                // validation completions occur before any effect starts.
                                validation_completed.push((a.initiated_event_id, event.event_id));
                            },
                            Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a)) => {
                                if let Some(execution) = a.workflow_execution.as_ref() {
                                    if execution.workflow_id.starts_with("alegria-owner-operation-v2-") {
                                        effects.push((execution.workflow_id.clone(),
                                            execution.run_id.clone(),event.event_id));
                                    }
                                }
                            },
                            _ => {},
                        }
                    }
                    if effects.len() == expected_effect_count {
                        assert_eq!(validation_started, 2,
                            "{case}: two typed ValidateChild workflows are required");
                        assert_eq!(validation_completed.len(), 2,
                            "{case}: both ValidateChild workflows must complete before effects");
                        let last_validation = validation_completed.iter()
                            .map(|(_, event)| *event).max().unwrap();
                        let first_effect = effects.iter().map(|(_,_,event)| *event).min().unwrap();
                        assert!(last_validation < first_effect,
                            "{case}: server History did not prove the whole-plan validation barrier");
                        break (effects, last_validation, first_effect);
                    }
                    sleep(Duration::from_millis(200)).await;
                }
            }).await.expect("V2 ValidateOnly barrier/effect children never reached");
        assert!(first_effect_event > validated_through);
        assert!(effect_children.iter().all(|(workflow_id,_,_)|
            workflow_id.starts_with("alegria-owner-operation-v2-")
            && !workflow_id.starts_with("alegria-owner-operation-v1-")),
            "V1 and V2 Child workflow IDs must remain version-isolated");
        assert_eq!(effect_children.len(), expected_effect_count);
        // The independent case proves *both effects started* before either
        // has an operator admission. A sequential coordinator cannot pass.
        let pending_attempts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_attempts WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        assert_eq!(pending_attempts, 0, "{case}: no speculative Qdrant side effect");
        // Parallel children are started by two independent Task Queues and
        // can arrive in EITHER server ChildStarted order. Classify using the
        // actual canonical ChildInitiated *input* (the typed operation), not
        // the wall-clock order, a guessed route or the caller's assertion.
        let classified_history = parent.fetch_history(
            WorkflowFetchHistoryOptions::builder().build())
            .into_events().await.expect("classify true parallel ChildInitiated inputs");
        let classified = effect_children.iter().map(|(child_id,child_run,started)| {
            let initiated = classified_history.iter().find_map(|event| {
                let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a)) =
                    &event.attributes else { return None; };
                if &a.workflow_id != child_id { return None; }
                let payloads = a.input.as_ref()?;
                if payloads.payloads.len() != 1 { return None; }
                let invocation: operation_protocol::OwnerInvocationV1 =
                    serde_json::from_slice(&payloads.payloads[0].data).ok()?;
                Some(invocation.checked_dispatch.operation.id.as_str().to_owned())
            }).expect("trusted server ChildInitiated input must be typed OwnerInvocationV1");
            (child_id.clone(),child_run.clone(),*started,initiated)
        }).collect::<Vec<_>>();
        let (pg_child_id, pg_child_run, _, _) = classified.iter()
            .find(|(_,_,_,operation)| operation == "pg.vector_projection.read")
            .expect("PG projection owner Child initiated independently of start order");
        let pg_request = private_locators.request("v2-pg-child-locator", &serde_json::json!({
            "admission_id": pg_admission, "canonical_run_id": run_id,
            "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
            "child_workflow_id": pg_child_id, "child_workflow_run_id": pg_child_run,
            "expires_in_seconds": 300,
        }).to_string());
        let pg_issuer = |request: &PathBuf| Command::new(&pg_bin)
            .arg("--operator-issue-pg-projection-v2").arg(request)
            .env("ALEGRIA_ENABLE_PG_VECTOR_PROJECTION_OPERATOR_V2", "true")
            .env("PG_VECTOR_PROJECTION_V2_OPERATOR_DATABASE_URL", &roles.pg_projection_issuer_url)
            .env("PG_EFFECT_OPERATOR_TENANT_ID", tenant)
            .env("PG_OWNER_EXPECTED_BUILD_ID", &build_id)
            .env("PG_EFFECT_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
            .env("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL").output().unwrap();
        let forged_pg = private_locators.request("v2-forged-pg-child", &serde_json::json!({
            "admission_id": pg_admission, "canonical_run_id": run_id,
            "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
            "child_workflow_id": pg_child_id,
            "child_workflow_run_id": Uuid::new_v4().to_string(),
            "expires_in_seconds": 300,
        }).to_string());
        assert!(!pg_issuer(&forged_pg).status.success(),
            "{case}: forged PG server-issued Child Run ID must not create V2 admission");
        let legacy_parent_pg = private_locators.request("v2-reused-legacy-parent", &serde_json::json!({
            "admission_id": pg_admission, "canonical_run_id": run_id,
            "parent_workflow_id": operation_protocol::operation_dag_parent_workflow_id(
                &run_id.to_string()),
            "parent_workflow_run_id": parent_run,
            "child_workflow_id": pg_child_id, "child_workflow_run_id": pg_child_run,
            "expires_in_seconds": 300,
        }).to_string());
        assert!(!pg_issuer(&legacy_parent_pg).status.success(),
            "{case}: historical V1 parent identity cannot authorize a V2 PG child");
        let denied_pg_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.pg_vector_projection_admissions_v1 WHERE admission_id=$1"
        ).bind(pg_admission).fetch_one(&pg.pool).await.unwrap();
        assert_eq!(denied_pg_rows, 0,
            "{case}: forged child and legacy parent must have zero SQL admissions");
        let issued_pg = pg_issuer(&pg_request);
        assert!(issued_pg.status.success(), "{case}: real V2 PG operator: stdout={} stderr={}",
            String::from_utf8_lossy(&issued_pg.stdout),
            String::from_utf8_lossy(&issued_pg.stderr));
        let (q_child_id, q_child_run) = if independent {
            let (id,run,_,_) = classified.iter()
                .find(|(_,_,_,operation)| operation == "qdrant.keyword_projection.upsert")
                .expect("parallel Qdrant Child operation is identified by server payload");
            (id.clone(),run.clone())
        } else {
            timeout(Duration::from_secs(90), async {
                loop {
                    let history = parent.fetch_history(
                        WorkflowFetchHistoryOptions::builder().build())
                        .into_events().await.expect("V2 Produced child History");
                    let pg_completed = history.iter().find(|event|
                        matches!(event.attributes.as_ref(),
                            Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(_)))
                        && event.event_id > first_effect_event);
                    let consumer = history.iter().find_map(|event| {
                        let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a)) =
                            &event.attributes else { return None; };
                        let exec = a.workflow_execution.as_ref()?;
                        (exec.workflow_id.starts_with("alegria-owner-operation-v2-")
                            && exec.workflow_id != *pg_child_id)
                            .then_some((exec.workflow_id.clone(), exec.run_id.clone(), event.event_id))
                    });
                    if let (Some(completed), Some((id,run_id,started))) = (pg_completed,consumer) {
                        assert!(completed.event_id < started,
                            "V2 Qdrant consumed Produced before actual PG Child completion");
                        break (id,run_id);
                    }
                    sleep(Duration::from_millis(200)).await;
                }
            }).await.expect("PG actual Produced result never enabled Qdrant V2 child")
        };
        let q_request = private_locators.request("v2-qdrant-child-locator", &serde_json::json!({
            "admission_id": q_admission, "canonical_run_id": run_id,
            "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
            "child_workflow_id": q_child_id, "child_workflow_run_id": q_child_run,
            "expires_in_seconds": 300,
        }).to_string());
        let q_issuer = |request: &PathBuf, issuer_tenant: &str| Command::new(&qdrant_bin)
            .arg("--operator-issue-v3").arg(request)
            .env("ALEGRIA_ENABLE_QDRANT_GENERIC_OPERATOR_V1", "true")
            .env("ALEGRIA_ENABLE_QDRANT_GENERIC_V2_OPERATOR_V1", "true")
            .env("QDRANT_GENERIC_OPERATOR_DATABASE_URL_FILE", &roles.qdrant_operator_secret)
            .env("QDRANT_GENERIC_OPERATOR_TENANT_ID", issuer_tenant)
            .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
            .env("QDRANT_GENERIC_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env("QDRANT_GENERIC_OPERATOR_TRUSTED_BUILD_ID", &build_id)
            .env_remove("DATABASE_URL").output().unwrap();
        assert!(!q_issuer(&q_request, "foreign-v2-tenant").status.success(),
            "{case}: cross-tenant Qdrant V2 operator must fail closed");
        let q_wrong = private_locators.request("v2-forged-q-child", &serde_json::json!({
            "admission_id": q_admission, "canonical_run_id": run_id,
            "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
            "child_workflow_id": q_child_id,
            "child_workflow_run_id": Uuid::new_v4().to_string(),
            "expires_in_seconds": 300,
        }).to_string());
        assert!(!q_issuer(&q_wrong, tenant).status.success(),
            "{case}: forged Qdrant Child Run ID must fail V2 History proof");
        let q_success = q_issuer(&q_request, tenant);
        assert!(q_success.status.success(), "{case}: real Qdrant V4 issuer: stdout={} stderr={}",
            String::from_utf8_lossy(&q_success.stdout),
            String::from_utf8_lossy(&q_success.stderr));
        let completion = timeout(Duration::from_secs(110),
            parent.get_result(WorkflowGetResultOptions::default()))
            .await.expect("V2 DAG never finished")
            .expect("actual V2 mixed-owner parent failed");
        let receipt: OperationDagReceiptV1 = completion.to_value(&PayloadConverter::serde_json());
        assert_eq!(receipt.plan_hash, run.plan.hash);
        assert_eq!(receipt.owner_results.len(), 2);
        if independent {
            assert_eq!(receipt.order.len(), 2);
            assert!(receipt.order.iter().any(|node| node.as_str() == "pg_vector_read"));
            assert!(receipt.order.iter().any(|node| node.as_str() == "qdrant_external_upsert"));
        } else {
            let produced: projection_contracts::VectorProjectionV1 = serde_json::from_slice(
                &receipt.artifacts[&"pg_vector_produced".into()].payload
            ).expect("actual Temporal Child result was typed projection");
            assert_eq!(produced.vector, pg_vector);
            assert_eq!(produced.keyword_key, pg_keyword);
            assert!(receipt.produced_evidence.contains_key(&"pg_vector_produced".into()));
        }
        let q_command_sha: String = sqlx::query_scalar(
            "SELECT command_sha256 FROM pipeline.qdrant_projection_v4_admissions WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        assert!(vector_qdrant::projection_commands::projection_readback_exact(
            "seo_keyword_clusters_4", &q_admission.to_string(), "keyword_cluster",
            &format!("tenant/{tenant}/context/{context}/keyword/{q_keyword}"),
            &q_vector, &serde_json::json!({
                "tenant_id": tenant, "context_key": context, "keyword_key": q_keyword,
                "operator_admission_id": q_admission.to_string(),
                "approved_command_sha256": q_command_sha,
            }),
        ).await.expect("read actual Qdrant V4 provider effect"));
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_receipts
             WHERE admission_id=$1 AND exact_provider_readback=TRUE"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        let attempts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_attempts WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        assert_eq!((receipts,attempts),(1,1),
            "{case}: V2 owner Activity is idempotently admitted exactly once");
        eprintln!("CI_MIXED_V2_NATIVE_REAL_{case}_PG_TEMPORAL_QDRANT=PASS");
        let _ = routes;
    }

    // A structurally legal plan with either malformed concrete bytes or an
    // exact-shape foreign-tenant DTO must fail at Qdrant ValidateChild before
    // the parallel PostgreSQL Child performs an effect. Assert not only the
    // server History fence but also the durable SQL admission/effect ledgers.
    for (case, rejected_id, rejected_run, pg_admission, q_admission) in [
        ("bad-vector", invalid_id, &invalid_run, invalid_pg_admission, invalid_q_admission),
        ("foreign-tenant", foreign_id, &foreign_run, foreign_pg_admission, foreign_q_admission),
    ] {
        let rejected_parent_id = operation_dag_parent_workflow_id_v2(&rejected_id.to_string());
        let rejected_parent = client.start_workflow(
            UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
            RawValue::from_value(rejected_run, &PayloadConverter::serde_json()),
            WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, rejected_parent_id).build(),
        ).await.expect("start invalid downstream owner V2 DTO test");
        let rejected_result = timeout(Duration::from_secs(45),
            rejected_parent.get_result(WorkflowGetResultOptions::default()))
            .await.expect("invalid downstream V2 ValidateChild must terminate");
        assert!(rejected_result.is_err(),
            "{case}: invalid Qdrant owner DTO cannot pass ValidateOnly");
        let rejected_history = rejected_parent.fetch_history(
            WorkflowFetchHistoryOptions::builder().build())
            .into_events().await.expect("rejected real V2 parent History");
        assert!(rejected_history.iter().any(|event| matches!(
            &event.attributes,
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-validate-v1-")
        )), "{case}: actual owner ValidateChild must initiate before rejecting");
        assert!(!rejected_history.iter().any(|event| matches!(
            &event.attributes,
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-operation-v2-")
        )), "{case}: V2 barrier must forbid ALL PG/Qdrant effect Children");
        let pg_admissions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.pg_vector_projection_admissions_v1 WHERE admission_id=$1"
        ).bind(pg_admission).fetch_one(&pg.pool).await.unwrap();
        let q_admissions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_admissions WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        let q_attempts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_attempts WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        let q_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.qdrant_projection_v4_receipts WHERE admission_id=$1"
        ).bind(q_admission).fetch_one(&pg.pool).await.unwrap();
        assert_eq!((pg_admissions, q_admissions, q_attempts, q_receipts), (0, 0, 0, 0),
            "{case}: a failed semantic preflight must leave both owner ledgers untouched");
    }
    eprintln!("CI_MIXED_V2_NATIVE_VALIDATE_BARRIER_NO_EFFECT=PASS");
}
