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
struct MixedV6NativeRoles {
    pg_worker_url: String,
    pg_operator_url: String,
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
    let qdrant_worker = format!("mixed_qdrant_worker_{nonce}");
    let qdrant_operator = format!("mixed_qdrant_operator_{nonce}");
    let governance_operator = format!("mixed_dag_issuer_{nonce}");
    const PASSWORD: &str = "disposable_mixed_owner_password";
    // Each dynamic identifier is a fixed ASCII prefix plus UUID hex, and all
    // SQL executes solely against disposable testcontainers PostgreSQL.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {pg_worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {pg_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {qdrant_worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {qdrant_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {governance_operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {pg_worker},{pg_operator},{qdrant_worker},{qdrant_operator},{governance_operator};
         GRANT USAGE ON SCHEMA pipeline,extracted,raw,site TO {pg_worker};
         GRANT USAGE ON SCHEMA pipeline,extracted,raw,kb TO {pg_operator};
         GRANT USAGE ON SCHEMA pipeline TO {qdrant_worker},{qdrant_operator};
         GRANT USAGE ON SCHEMA pipeline TO {governance_operator};
         GRANT SELECT ON pipeline.execution_runs,pipeline.step_executions,
             pipeline.pg_effect_v3_admissions,pipeline.pg_effect_v3_receipts,
             extracted.rule_candidates,raw.sections,raw.pages,site.seo_hitl_tasks
           TO {pg_worker};
         GRANT SELECT ON kb.sources,pipeline.execution_runs,
             extracted.rule_candidates,raw.sections,raw.pages
           TO {pg_operator};
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
           TO {pg_worker},{pg_operator},{qdrant_worker},{qdrant_operator};
         GRANT INSERT ON pipeline.operation_dag_run_governance_v1 TO {governance_operator};
         GRANT EXECUTE ON FUNCTION pipeline.operation_dag_canonical_source_run_id_v1(UUID,TEXT,TEXT)
           TO {governance_operator};"
    ))) .execute(&pg.pool).await.expect("DBA provisions five isolated operator/Activity identities");
    for role in [&pg_worker, &pg_operator, &qdrant_worker, &qdrant_operator, &governance_operator] {
        sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
            .bind(role).bind(tenant).execute(&pg.pool).await.unwrap();
    }
    let restricted_url = |role: &str| {
        let prefix = "postgres://postgres:postgres_password@";
        assert!(pg.database_url.starts_with(prefix), "must use disposable PostgresHarness DSN");
        pg.database_url.replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    MixedV6NativeRoles {
        pg_worker_url: restricted_url(&pg_worker),
        pg_operator_url: restricted_url(&pg_operator),
        qdrant_worker_secret: secret("mixed-qdrant-worker", &restricted_url(&qdrant_worker)),
        qdrant_operator_secret: secret("mixed-qdrant-operator", &restricted_url(&qdrant_operator)),
        governance_operator_url: restricted_url(&governance_operator),
    }
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
    use seo_application::operation_adapter::OperationDagReceiptV1;
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

    let client = temporal_gateway::connect_client(
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
    use seo_application::operation_adapter::{GenericDagError, OperationDagMachine};

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
