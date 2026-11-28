// Physical one-use siteArchitect authorization -> native Temporal V2 parent
// -> independent Acquisition ValidateOnly and owner Child Histories. There
// are deliberately five SQL roles (business, source, approver, request
// issuer, reserver), plus a SELECT-only reader. The private request is not
// treated as authenticated
// ChatGPT identity or permission to mutate the provider without SQL.

#[tokio::test]
#[serial]
async fn one_step_grant_native_v1_approved_acquisition_start_and_replay_refusal() {
    use acquisition::operation_owner_schema_json::acquisition_owner_schemas;
    use acquisition::operations::{ClassifySourcePage, ClassifySourcePageInput};
    use acquisition::source_relevance::PageUtilityClassifierOutput;
    use acquisition::trusted_source_transform_v1::{
        classify_source_page_payload_from_pinned_html_v1, CapturedHtmlPageV1,
    };
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, owner_content_hash, OneStepResourceLimitsV1,
        Operation, OperationDagReceiptV1, PlanHash, SiteArchitectExpectedStartV1,
        OPERATION_DAG_PARENT_QUEUE_V2, SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
    };
    use seo_steps::operation_owner_schema_json::seo_steps_owner_schemas;
    use seo_steps::owner_pure_operations::{
        BuildSeoOpportunities, ExtractProceduralRules, NormalizeSerpQueries,
    };
    use truth_domain::operation_owner_schema_json::truth_pure_operation_owner_schemas;
    use truth_domain::operations::{DetectContradictionsV1, JudgeCompletenessV1};

    #[derive(serde::Serialize)]
    struct ExactStartIntent<'a> {
        schema_revision: u16,
        run: &'a operation_protocol::OperationDagRunV6,
        context_key: &'a str,
        expected: &'a SiteArchitectExpectedStartV1,
        expires_at_unix_seconds: u64,
    }
    #[derive(serde::Serialize)]
    struct SourceIssuerIntent<'a> {
        schema_revision: u16,
        run: &'a operation_protocol::OperationDagRunV6,
        context_key: &'a str,
        source_page_id: i64,
        expires_at_unix_seconds: i64,
    }
    #[derive(serde::Serialize)]
    struct RequestIssuerIntent<'a> {
        schema_revision: u16,
        request_id: Uuid,
        authorization_event_id: Uuid,
        run: &'a operation_protocol::OperationDagRunV6,
        expected: &'a SiteArchitectExpectedStartV1,
        context_key: &'a str,
        expires_at_unix_seconds: i64,
    }
    #[derive(serde::Serialize)]
    struct GrantedStartIntent<'a> {
        schema_revision: u16,
        request_id: Uuid,
        intent: ExactStartIntent<'a>,
    }
    #[derive(serde::Serialize)]
    struct GrantedStatusLocator {
        schema_revision: u16,
        request_id: Uuid,
    }
    let expected_for = |run: &operation_protocol::OperationDagRunV6| SiteArchitectExpectedStartV1 {
        schema_revision: SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
        plan_hash: run.plan.hash.clone(),
        command_hashes: run.owner_commands.iter().map(|(id, wire)| {
            (id.clone(), owner_content_hash(&wire.command_bytes))
        }).collect(),
        source_payload_hashes: run.admitted.iter().map(|(id, artifact)| {
            (id.clone(), owner_content_hash(&artifact.payload))
        }).collect(),
    };

    let build_id = env::var("GITHUB_SHA").expect("physical test pins exact-source native workers");
    assert_eq!(build_id.len(), 40);
    let tenant = "ci-native-acquisition-owner";
    let context = format!("one-step-grant-{}", Uuid::new_v4().simple());
    let record_id = Uuid::new_v4();
    let source_url = "https://ci-native-source.example.test/approved-directory";
    let raw_html = "<html><head><title>Approved directory</title></head><body><main><p>Locations and requirements</p></main></body></html>";
    let raw_hash = owner_content_hash(raw_html.as_bytes());

    let pg = PostgresHarness::start().await.expect("disposable authenticated PG roles");
    mixed_v2_install_native_admissions(&pg.pool).await;
    for (label, migration) in [
        ("immutable raw source", include_str!("../../../../../db/migrations/20261127_0002_operation_dag_external_source_attestation_v1.sql")),
        ("separate user grant", include_str!("../../../../../db/migrations/20261127_0003_operation_dag_one_step_request_grants_v1.sql")),
    ] {
        sqlx::raw_sql(migration).execute(&pg.pool).await
            .unwrap_or_else(|error| panic!("{label} migration failed: {error}"));
    }
    sqlx::query(
        "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ES','tourist','BY')",
    )
    .bind(&context).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context).bind(tenant).execute(&pg.pool).await.unwrap();
    let source_page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages
            (url,final_url,domain,dtype,tenant_id,status_code,content_type,
             raw_html,raw_html_bytes,content_hash)
         VALUES ($1,$1,'ci-native-source.example.test','government',$2,200,'text/html',
                 $3,$4,$5) RETURNING id",
    )
    .bind(source_url).bind(tenant).bind(raw_html)
    .bind(i32::try_from(raw_html.len()).unwrap()).bind(&raw_hash)
    .fetch_one(&pg.pool).await.unwrap();
    let transformed = classify_source_page_payload_from_pinned_html_v1(&CapturedHtmlPageV1 {
        page_id: source_page_id,
        url: source_url.into(), final_url: Some(source_url.into()),
        content_type: Some("text/html".into()), status_code: Some(200),
        raw_html: Some(raw_html.into()),
        raw_html_bytes: Some(i32::try_from(raw_html.len()).unwrap()),
        content_hash: Some(raw_hash),
    }).expect("only pinned captured HTML may derive the typed Acquisition DTO");
    let strict: ClassifySourcePageInput = serde_json::from_slice(&transformed.payload).unwrap();
    assert_eq!(strict.title, "Approved directory");
    let (mut run, routes) = acquisition_native_v2_case(
        ClassifySourcePage::contract(), transformed.payload, &build_id,
    );
    let provenance = format!("source-record-v1:{record_id}");
    run.plan.external_artifacts.get_mut(&"source".into()).unwrap().provenance = provenance.clone();
    run.admitted.get_mut(&"source".into()).unwrap().provenance = provenance;
    run.plan.hash = PlanHash(String::new());
    run.plan = run.plan.clone().seal(routes.catalog()).unwrap();
    routes.validate_run_routes(&run).unwrap();
    let expected = expected_for(&run);
    let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    let run_uuid = Uuid::parse_str(&run.plan.scope.run_id).unwrap();
    let request_id = Uuid::new_v4();
    let authorization_event_id = Uuid::new_v4();
    let limits = OneStepResourceLimitsV1 {
        max_admitted_source_bytes: 65536,
        max_effect_dispatches: 0,
        max_provider_requests: 0,
        currency_code: "USD".into(),
        max_charge_minor_units: 0,
    };
    let maximum_limits = OneStepResourceLimitsV1 {
        max_admitted_source_bytes: 524288,
        max_effect_dispatches: 1,
        max_provider_requests: 1000,
        currency_code: "USD".into(),
        max_charge_minor_units: 10000,
    };

    let operator_catalog = mixed_v6_governance_issuer_catalog(&run, &routes);
    let schemas = acquisition_owner_schemas().unwrap();
    let contract = routes.catalog().contract(&run.pinned_routes[0].operation).unwrap();
    let owner_pin = &schemas[&contract.reference];
    owner_pin.validate(contract).unwrap();
    // The exact same granted Start bridge must also serve separately compiled
    // pure SEO and Truth owners. Only trusted source contracts are installed;
    // source-free experiments are not misrepresented as authenticated facts.
    let mut pure_cases = vec![
        seo_steps_native_v2_case(
            ExtractProceduralRules::contract(),
            br#"{"section_id":"section-1","raw_text":"unverified context","mentions":[{"raw_text":"80 EUR","entity_type":"fee","has_numeric":true,"is_central":true,"confidence":0.9}]}"#.to_vec(),
            &build_id,
        ),
        seo_steps_native_v2_case(
            NormalizeSerpQueries::contract(),
            br#"{"run_id":"grant-run","queries":["spain visa fee"]}"#.to_vec(),
            &build_id,
        ),
        seo_steps_native_v2_case(
            BuildSeoOpportunities::contract(),
            br#"{"serp_patterns":[{"serp_pattern_key":"p1","query_batch_key":"q1","scope_signature":"","query":"spain visa fee","pattern_type":"query_intent","dominant_intent":"informational","reliability_score":0.75,"evidence_ref":"q1","status":"active"}]}"#.to_vec(),
            &build_id,
        ),
        truth_native_v2_case(
            JudgeCompletenessV1::contract(),
            br#"{"section_id":"grant-fee","raw_text":"Fee EUR 25","source_numeric_tokens":["25"],"extracted_numeric_tokens":[],"extracted_rule_keys":[]}"#.to_vec(),
            &build_id,
        ),
        truth_native_v2_case(
            DetectContradictionsV1::contract(),
            br#"{"run_id":"grant-truth","facts":[{"subject_key":"fee","predicate_key":"amount","value_normalized":"80 EUR","source_key":"source-a","confidence":0.96},{"subject_key":"fee","predicate_key":"amount","value_normalized":"120 EUR","source_key":"source-b","confidence":0.94}]}"#.to_vec(),
            &build_id,
        ),
    ];
    for (pure_run, pure_routes) in &mut pure_cases {
        pure_run.plan.scope.tenant_id = tenant.to_owned();
        for external in pure_run.plan.external_artifacts.values_mut() {
            external.scope.tenant_id = tenant.to_owned();
        }
        for admitted in pure_run.admitted.values_mut() {
            admitted.scope.tenant_id = tenant.to_owned();
        }
        pure_run.plan.hash = PlanHash(String::new());
        pure_run.plan = pure_run.plan.clone().seal(pure_routes.catalog()).unwrap();
        pure_routes.validate_run_routes(pure_run).unwrap();
    }
    let seo_pins = seo_steps_owner_schemas().unwrap();
    let truth_pins = truth_pure_operation_owner_schemas().unwrap();
    let mut catalog_entries = vec![serde_json::json!({
        "contract": contract,
        "route": run.pinned_routes[0],
        "parameter_schema": {"type":"object","additionalProperties":false},
        "owner_schema": owner_pin,
    })];
    for (pure_run, pure_routes) in &pure_cases {
        let route = &pure_run.pinned_routes[0];
        let pure_contract = pure_routes.catalog().contract(&route.operation).unwrap();
        let pin = seo_pins.get(&route.operation)
            .or_else(|| truth_pins.get(&route.operation)).unwrap();
        pin.validate(pure_contract).unwrap();
        catalog_entries.push(serde_json::json!({
            "contract": pure_contract,
            "route": route,
            "parameter_schema": {"type":"object","additionalProperties":false},
            "owner_schema": pin,
        }));
    }
    let trusted_catalog = SiteArchitectV2CiRootCatalog::new();
    trusted_catalog.install(&serde_json::json!({
        "schema_revision": 3,
        "entries": catalog_entries,
    }));

    let nonce = Uuid::new_v4().simple().to_string();
    let business_role = format!("grant_business_{nonce}");
    let source_role = format!("grant_source_{nonce}");
    let approver_role = format!("grant_approver_{nonce}");
    let issuer_role = format!("grant_issuer_{nonce}");
    let operator_role = format!("grant_operator_{nonce}");
    let reader_role = format!("grant_reader_{nonce}");
    let status_role = format!("grant_status_{nonce}");
    const PASSWORD: &str = "disposable_native_one_step_grant_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {business_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {source_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {approver_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {issuer_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {operator_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {reader_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {status_role} LOGIN NOINHERIT NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB NOREPLICATION PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {business_role},{source_role},{approver_role},{issuer_role},{operator_role},{reader_role},{status_role};
         GRANT USAGE ON SCHEMA pipeline TO {business_role},{source_role},{approver_role},{issuer_role},{operator_role},{reader_role},{status_role};
         GRANT USAGE ON SCHEMA raw TO {source_role};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business_role};
         GRANT INSERT,UPDATE(revoked_at) ON pipeline.operation_dag_external_source_attestations_v1 TO {source_role};
         GRANT SELECT ON raw.pages TO {source_role};
         GRANT INSERT ON pipeline.operation_dag_one_step_request_approvals_v1 TO {approver_role};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_approvals_v1,
             pipeline.operation_dag_one_step_principal_policies_v1 TO {issuer_role};
         GRANT INSERT ON pipeline.operation_dag_one_step_request_grants_v1 TO {issuer_role};
         GRANT SELECT ON pipeline.operation_dag_business_runs_v1,
             pipeline.operation_dag_external_source_attestations_v1 TO {reader_role};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_grants_v1 TO {operator_role};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_grants_v1 TO {status_role};
         GRANT EXECUTE ON FUNCTION pipeline.one_step_limits_valid_v1(jsonb)
             TO {approver_role},{issuer_role};
         GRANT EXECUTE ON FUNCTION pipeline.one_step_expected_valid_v1(jsonb,text)
             TO {approver_role},{issuer_role};
         GRANT EXECUTE ON FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid)
             TO {operator_role};
         GRANT EXECUTE ON FUNCTION pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid)
             TO {operator_role};
         GRANT EXECUTE ON FUNCTION pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text)
             TO {operator_role};
         REVOKE EXECUTE ON FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid)
             FROM {status_role};
         REVOKE EXECUTE ON FUNCTION pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid)
             FROM {status_role};
         REVOKE EXECUTE ON FUNCTION pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text)
             FROM {status_role};"
    )))
    .execute(&pg.pool).await.expect("DBA provisions separately credentialed grant owners");
    for role in [&business_role, &source_role, &approver_role, &issuer_role, &operator_role, &reader_role, &status_role] {
        sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
            .bind(role).bind(tenant).execute(&pg.pool).await.unwrap();
    }
    // The grant issuer intentionally has NO raw namespace capability. Its
    // privilege probe must use pg_catalog OIDs, not require a new USAGE grant.
    let issuer_raw_usage: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1::name,'raw','USAGE')",
    )
    .bind(&issuer_role)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    let source_raw_usage: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1::name,'raw','USAGE')",
    )
    .bind(&source_role)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert!(!issuer_raw_usage && source_raw_usage,
        "grant issuer must inspect raw DML ACL by catalog OID without raw schema USAGE");
    let (status_select, status_insert, status_update, status_reserve,
        status_unknown, status_confirm): (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege($1::name,
                    'pipeline.operation_dag_one_step_request_grants_v1','SELECT'),
                has_table_privilege($1::name,
                    'pipeline.operation_dag_one_step_request_grants_v1','INSERT'),
                has_table_privilege($1::name,
                    'pipeline.operation_dag_one_step_request_grants_v1','UPDATE'),
                has_function_privilege($1::name,
                    'pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid)',
                    'EXECUTE'),
                has_function_privilege($1::name,
                    'pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid)',
                    'EXECUTE'),
                has_function_privilege($1::name,
                    'pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text)',
                    'EXECUTE')",
    )
    .bind(&status_role)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert!(status_select && !status_insert && !status_update
        && !status_reserve && !status_unknown && !status_confirm,
        "private status login must be SELECT-only and unable to reserve, mutate or confirm Start");
    assert_ne!(status_role, operator_role, "status reader must not reuse the reserver SQL LOGIN");
    let prefix = "postgres://postgres:postgres_password@";
    assert!(pg.database_url.starts_with(prefix));
    let role_dsn = |role: &str| pg.database_url.replacen(
        prefix, &format!("postgres://{role}:{PASSWORD}@"), 1,
    );
    let business_pool = sqlx::PgPool::connect(&role_dsn(&business_role)).await.unwrap();
    let approver_pool = sqlx::PgPool::connect(&role_dsn(&approver_role)).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '15 minutes')",
    )
    .bind(run_uuid).bind(tenant).bind(&context).bind(run.plan.hash.as_str())
    .bind(&parent_id).execute(&business_pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_principal_policies_v1
           (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,$4,$5,$6::jsonb)",
    )
    .bind(&approver_role).bind(&context).bind(tenant)
    .bind("ci-authenticated-user").bind("ci-safe-zero-cost-v1")
    .bind(serde_json::to_string(&maximum_limits).unwrap())
    .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_start_principals_v1
           (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,$4,$5,$6::jsonb)",
    )
    .bind(&operator_role).bind(&context).bind(tenant)
    .bind("ci-authenticated-user").bind("ci-safe-zero-cost-v1")
    .bind(serde_json::to_string(&maximum_limits).unwrap())
    .execute(&pg.pool).await.expect("independent DBA-approved reserver LOGIN principal/ACL budget");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_request_approvals_v1
           (authorization_event_id,request_id,run_id,tenant_id,principal_id,policy_id,
            context_key,operation_id,operation_revision,parent_workflow_id,plan_hash,
            expected_start,limits,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::jsonb,$13::jsonb,
                 clock_timestamp()+interval '5 minutes')",
    )
    .bind(authorization_event_id).bind(request_id).bind(run_uuid).bind(tenant)
    .bind("ci-authenticated-user").bind("ci-safe-zero-cost-v1").bind(&context)
    .bind(contract.reference.id.as_str()).bind(i32::try_from(contract.reference.revision).unwrap())
    .bind(&parent_id).bind(run.plan.hash.as_str())
    .bind(serde_json::to_string(&expected).unwrap())
    .bind(serde_json::to_string(&limits).unwrap())
    .execute(&approver_pool).await.expect("separately credentialed approved user intent");

    let temporal = TemporalHarness::start().await.expect("real disposable Temporal server");
    let _parent = ChildGuard(Some(Command::new(binary("INTEGRATION_EFFECT_DAG_WORKER_BIN"))
        .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE_V2)
        .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &trusted_catalog.0)
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("native V2 generic parent")));
    let _owner = ChildGuard(Some(Command::new(binary("INTEGRATION_ACQUISITION_PURE_OWNER_BIN"))
        .env("ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1", "true")
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("strict independent Acquisition ValidateOnly and owner")));
    let _seo_owner = ChildGuard(Some(Command::new(binary("INTEGRATION_SEO_STEPS_PURE_OWNER_BIN"))
        .env("ALEGRIA_ENABLE_SEO_STEPS_PURE_OWNER_V1", "true")
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("strict independent SEO ValidateOnly and owner")));
    let _truth_owner = ChildGuard(Some(Command::new(binary("INTEGRATION_TRUTH_PURE_OWNER_BIN"))
        .env("ALEGRIA_ENABLE_TRUTH_PURE_OWNER_V1", "true")
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("strict independent Truth ValidateOnly and owner")));
    let client = connect_client(&temporal.temporal_url,
        format!("ci-one-step-grant-{request_id}"), &temporal.namespace).await.unwrap();

    let private = MixedV2PrivateLocatorFiles::new();
    let source_dsn = private.request("source-issuer-dsn", &role_dsn(&source_role));
    let request_dsn = private.request("request-issuer-dsn", &role_dsn(&issuer_role));
    let operator_dsn = private.request("one-use-operator-dsn", &role_dsn(&operator_role));
    let reader_dsn = private.request("one-use-read-only-dsn", &role_dsn(&reader_role));
    let status_dsn = private.request("one-use-status-select-only-dsn", &role_dsn(&status_role));
    let limits_path = private.request("one-use-operator-max-limits", &serde_json::to_string(&maximum_limits).unwrap());
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 240;
    let source_intent = private.request("source-from-real-raw-html", &serde_json::to_string(
        &SourceIssuerIntent { schema_revision: 1, run: &run, context_key: &context,
            source_page_id, expires_at_unix_seconds: deadline as i64 },
    ).unwrap());
    let issue_source = Command::new(binary("INTEGRATION_ACQUISITION_SOURCE_ISSUER_BIN"))
        .arg("--issue-source-v1").arg(&source_intent)
        .env("ALEGRIA_ENABLE_ACQUISITION_SOURCE_ISSUER_V1", "true")
        .env("ACQUISITION_SOURCE_ISSUER_DATABASE_URL_FILE", &source_dsn)
        .env_remove("DATABASE_URL")
        .output().expect("independent immutable source issuer binary");
    assert!(issue_source.status.success(), "source issuance failed: stdout={} stderr={}",
        String::from_utf8_lossy(&issue_source.stdout), String::from_utf8_lossy(&issue_source.stderr));

    let request_intent = private.request("approved-user-request", &serde_json::to_string(
        &RequestIssuerIntent { schema_revision: 1, request_id, authorization_event_id,
            run: &run, expected: &expected, context_key: &context,
            expires_at_unix_seconds: deadline as i64 },
    ).unwrap());
    let issue_request = Command::new(binary("INTEGRATION_ONE_STEP_REQUEST_ISSUER_BIN"))
        .arg("--issue-request-v1").arg(&request_intent)
        .env("ALEGRIA_ENABLE_ONE_STEP_REQUEST_ISSUER_V1", "true")
        .env("ONE_STEP_REQUEST_ISSUER_DATABASE_URL_FILE", &request_dsn)
        .env("ONE_STEP_REQUEST_ISSUER_TRUSTED_CATALOG_FILE", &operator_catalog)
        .env_remove("DATABASE_URL")
        .output().expect("independent request issuer binary");
    assert!(issue_request.status.success(), "request issuance failed: stdout={} stderr={}",
        String::from_utf8_lossy(&issue_request.stdout), String::from_utf8_lossy(&issue_request.stderr));
    let issued: serde_json::Value = serde_json::from_slice(&issue_request.stdout).unwrap();
    assert_eq!(issued["status"], "one_step_request_issued_not_started");
    let (grant_state, actual_issuer, actual_approver): (String, String, String) =
        sqlx::query_as(
            "SELECT g.state, g.issued_by::text, a.approved_by::text
             FROM pipeline.operation_dag_one_step_request_grants_v1 g
             JOIN pipeline.operation_dag_one_step_request_approvals_v1 a
               ON a.authorization_event_id=g.authorization_event_id
             WHERE g.request_id=$1",
        )
        .bind(request_id)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(grant_state, "unclaimed", "issuance is not a Temporal Start");
    assert_eq!(actual_issuer, issuer_role);
    assert_eq!(actual_approver, approver_role);
    assert_ne!(actual_issuer, actual_approver,
        "SQL grant issuance and user approval require different LOGINs");

    // Status is a third, independently bound SELECT-only SQL LOGIN, distinct
    // from both issuer and Start reserver. The opaque UUID cannot reserve or
    // attest execution, and reserved_by is matched to the separately pinned
    // reserver SQL identity, never to the status reader's identity.
    let status_locator = private.request("approved-grant-status-locator", &serde_json::to_string(
        &GrantedStatusLocator { schema_revision: 1, request_id },
    ).unwrap());
    let read_grant_as = |principal: &str| Command::new(binary("INTEGRATION_PG_EFFECT_WORKER_BIN"))
        .arg("--operator-read-granted-single-v1").arg(&status_locator)
        .env("ALEGRIA_ENABLE_SITE_ARCHITECT_GRANTED_V1_STATUS", "true")
        .env("SITE_ARCHITECT_SINGLE_V2_TENANT_ID", tenant)
        .env("SITE_ARCHITECT_GRANTED_V1_STATUS_DATABASE_URL_FILE", &status_dsn)
        .env("SITE_ARCHITECT_GRANTED_V1_RESERVER_LOGIN", &operator_role)
        .env("SITE_ARCHITECT_GRANTED_V1_PRINCIPAL_ID", principal)
        .env("SITE_ARCHITECT_GRANTED_V1_POLICY_ID", "ci-safe-zero-cost-v1")
        .env("SITE_ARCHITECT_GRANTED_V1_CONTEXT_KEY", &context)
        .env_remove("OPERATION_DAG_READ_V2_DATABASE_URL_FILE")
        .env_remove("SITE_ARCHITECT_GRANTED_V1_DATABASE_URL_FILE")
        .env_remove("DATABASE_URL")
        .output().expect("native SQL one-shot grant status reader");
    let before_start = read_grant_as("ci-authenticated-user");
    assert!(before_start.status.success(), "status reader rejected a live unclaimed grant: stdout={} stderr={}",
        String::from_utf8_lossy(&before_start.stdout), String::from_utf8_lossy(&before_start.stderr));
    let before_status: serde_json::Value = serde_json::from_slice(&before_start.stdout).unwrap();
    assert_eq!(before_status["status"], "grant_unclaimed");
    assert_eq!(before_status["grant_state"], "unclaimed");
    assert_eq!(before_status["execution"], "not_confirmed");
    assert_eq!(before_status["owner_admission"], "not_issued_by_status");
    let forged_status = read_grant_as("forged-user");
    assert!(!forged_status.status.success(),
        "a copied opaque status locator cannot enumerate another principal's grant");

    let grant_intent = private.request("approved-one-use-temporal-start", &serde_json::to_string(
        &GrantedStartIntent { schema_revision: 1, request_id, intent: ExactStartIntent {
            schema_revision: 1, run: &run, context_key: &context,
            expected: &expected, expires_at_unix_seconds: deadline,
        } },
    ).unwrap());
    // This is a fixture signer, not a simulated verified ChatGPT OAuth login.
    // It exercises the exact independently keyed Worker->native proof format
    // and the real SQL reservation/Temporal Start without any remote bridge.
    let signer_key = [0xab; 32];
    let signer_key_file = private.request("worker-to-native-hmac-key", &"ab".repeat(32));
    let signed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let signed = operation_protocol::seal_one_step_admission_for_private_fixture_v1(
        &request_id.to_string(),
        "https://auth.example.test", "https://mcp.example.test/mcp",
        "ci-oauth-subject", signed_at, &Uuid::new_v4().to_string(), &signer_key,
    );
    let signed_proof = private.request("signed-oauth-subject-proof", &serde_json::to_string(&signed).unwrap());
    let configure_start = |principal: &str, oauth_subject: &str| {
        let mut command = Command::new(binary("INTEGRATION_PG_EFFECT_WORKER_BIN"));
        command
        .env("ALEGRIA_ENABLE_SITE_ARCHITECT_SINGLE_V2_START", "true")
        .env("ALEGRIA_ENABLE_SITE_ARCHITECT_GRANTED_V1_START", "true")
        .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_KEY_FILE", &signer_key_file)
        .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_ISSUER", "https://auth.example.test")
        .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_RESOURCE", "https://mcp.example.test/mcp")
        .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_SUBJECT", oauth_subject)
        .env("SITE_ARCHITECT_SINGLE_V2_TENANT_ID", tenant)
        .env("SITE_ARCHITECT_SINGLE_V2_DATABASE_URL_FILE", &reader_dsn)
        .env("SITE_ARCHITECT_SINGLE_V2_TEMPORAL_URL", &temporal.temporal_url)
        .env("SITE_ARCHITECT_SINGLE_V2_TEMPORAL_NAMESPACE", &temporal.namespace)
        .env("SITE_ARCHITECT_SINGLE_V2_TRUSTED_CATALOG_FILE", &operator_catalog)
        .env("SITE_ARCHITECT_GRANTED_V1_DATABASE_URL_FILE", &operator_dsn)
        .env("SITE_ARCHITECT_GRANTED_V1_PRINCIPAL_ID", principal)
        .env("SITE_ARCHITECT_GRANTED_V1_POLICY_ID", "ci-safe-zero-cost-v1")
        .env("SITE_ARCHITECT_GRANTED_V1_CONTEXT_KEY", &context)
        .env("SITE_ARCHITECT_GRANTED_V1_MAX_LIMITS_FILE", &limits_path)
        .env_remove("DATABASE_URL");
        command
    };
    // The private ingress selects only this separately provisioned canonical UUID
    // file. Its ancestry is the same CI-only trusted directory as other locators.
    let ingress_dir = private.root.join("signed-ingress-intents");
    fs::create_dir(&ingress_dir).unwrap();
    fs::set_permissions(&ingress_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let ingress_intent = ingress_dir.join(format!("{request_id}.json"));
    fs::copy(&grant_intent, &ingress_intent).unwrap();
    fs::set_permissions(&ingress_intent, fs::Permissions::from_mode(0o600)).unwrap();
    let port_listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let ingress_port = port_listener.local_addr().unwrap().port();
    drop(port_listener);
    let ingress_child = configure_start("ci-authenticated-user", "ci-oauth-subject")
        .arg("--operator-serve-signed-granted-single-v1")
        .env("ALEGRIA_ENABLE_SITE_ARCHITECT_SIGNED_INGRESS_V1", "true")
        .env("SITE_ARCHITECT_SIGNED_INGRESS_V1_PORT", ingress_port.to_string())
        .env("SITE_ARCHITECT_SIGNED_INGRESS_V1_INTENT_DIR", &ingress_dir)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().expect("start private loopback signed ingress");
    let mut ingress_guard = ChildGuard(Some(ingress_child));
    let address = (std::net::Ipv4Addr::LOCALHOST, ingress_port);
    let mut listening = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(address).is_ok() {
            listening = true;
            break;
        }
        assert!(ingress_guard.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "private signed ingress exited before listening");
        sleep(Duration::from_millis(50)).await;
    }
    assert!(listening, "private signed ingress did not listen on loopback");
    let helper = PathBuf::from(env::var("GITHUB_WORKSPACE").unwrap())
        .join("infra/cloudflare/site-architect-mcp/test/loopback-admission.ts");
    let post = |scenario: &str| -> serde_json::Value {
        let output = Command::new("node")
            .arg("--experimental-strip-types").arg(&helper)
            .arg(ingress_port.to_string()).arg(request_id.to_string())
            .arg(&signer_key_file).arg(scenario)
            .output().expect("run Node private ingress fixture (Node >=24)");
        assert!(output.status.success(), "private ingress fixture failed for {scenario}");
        serde_json::from_slice(&output.stdout).expect("private ingress returns a bounded JSON reply")
    };
    for scenario in ["wrong-subject", "bad-signature", "expired"] {
        let denied = post(scenario);
        assert_eq!(denied["status"], "launch_blocked", "{scenario} must be denied");
        assert_eq!(denied["code"], "signed_remote_admission_not_verified");
        let state: String = sqlx::query_scalar(
            "SELECT state FROM pipeline.operation_dag_one_step_request_grants_v1 WHERE request_id=$1",
        ).bind(request_id).fetch_one(&pg.pool).await.unwrap();
        assert_eq!(state, "unclaimed", "{scenario} must not reserve the grant");
    }
    let denied_principal = configure_start("forged-user", "ci-oauth-subject")
        .arg("--operator-start-signed-granted-single-v1")
        .arg(&signed_proof)
        .arg(&grant_intent)
        .output()
        .expect("native SQL one-use grant Temporal Start bridge");
    assert!(!denied_principal.status.success(),
        "even knowing exact private intent and grant UUID does not authorize another principal");
    assert!(client.get_workflow_handle::<UntypedWorkflow>(parent_id.clone())
        .describe(WorkflowDescribeOptions::builder().build()).await.is_err(),
        "denied admissions must not create a Temporal workflow");
    let started_report = post("valid");
    assert_eq!(started_report["status"], "request_reserved");
    assert_eq!(started_report["code"], "one_use_reserved_execution_unverified");
    let duplicate = post("valid");
    assert_eq!(duplicate["status"], "start_outcome_unknown",
        "a repeated admission must not claim another reservation");
    assert_eq!(duplicate["code"], "operator_start_outcome_unconfirmed");

    let parent = client.get_workflow_handle::<UntypedWorkflow>(parent_id);
    let result = timeout(Duration::from_secs(90),
        parent.get_result(WorkflowGetResultOptions::default())).await
        .expect("real one-step user-approved parent timed out")
        .expect("real one-step user-approved parent failed");
    let receipt: OperationDagReceiptV1 = result.to_value(&PayloadConverter::serde_json());
    assert_eq!(receipt.order, vec!["one_step".into()]);
    let classified: PageUtilityClassifierOutput = serde_json::from_slice(
        &receipt.artifacts[&"result".into()].payload).unwrap();
    assert!(!classified.allow_structural_extraction);
    let events = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.unwrap();
    assert_eq!(events.iter().filter(|event| matches!(
        &event.attributes, Some(Attributes::WorkflowExecutionStartedEventAttributes(_))
    )).count(), 1, "one parent execution was started");
    for prefix in ["alegria-owner-validate-v1-", "alegria-owner-operation-v2-"] {
        assert_eq!(events.iter().filter(|event| matches!(
            &event.attributes,
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attributes))
                if attributes.workflow_id.starts_with(prefix)
        )).count(), 1, "exactly one independent native owner child {prefix}");
    }
    let final_state: String = sqlx::query_scalar(
        "SELECT state FROM pipeline.operation_dag_one_step_request_grants_v1 WHERE request_id=$1",
    ).bind(request_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(final_state, "start_outcome_unknown",
        "without an independent History/governance issuer the Start cannot mark itself confirmed");
    let after_start = read_grant_as("ci-authenticated-user");
    assert!(after_start.status.success(), "status reader rejected a consumed grant: stdout={} stderr={}",
        String::from_utf8_lossy(&after_start.stdout), String::from_utf8_lossy(&after_start.stderr));
    let after_status: serde_json::Value = serde_json::from_slice(&after_start.stdout).unwrap();
    assert_eq!(after_status["status"], "start_outcome_unknown");
    assert_eq!(after_status["grant_state"], "start_outcome_unknown");
    assert_eq!(after_status["temporal_start_history_attestation"], "unavailable_or_unverified");
    assert_eq!(after_status["execution"], "not_confirmed",
        "status without independent governance may never promote RPC ACK to execution proof");
    assert_eq!(after_status["owner_admission"], "not_issued_by_status");
    let after_status_state: String = sqlx::query_scalar(
        "SELECT state FROM pipeline.operation_dag_one_step_request_grants_v1 WHERE request_id=$1",
    ).bind(request_id).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(after_status_state, final_state,
        "status reads cannot mutate a consumed one-shot grant or re-open Start");

    // Five separately routable pure operations now cross exactly the same
    // independently authenticated, tenant-bound SQL grant -> one-use reserve
    // -> native Temporal Start path. This is NOT a remote ChatGPT OAuth test,
    // source attestation, or permission for a paid provider effect.
    for (index, (pure_run, pure_routes)) in pure_cases.iter().enumerate() {
        let pure_id = Uuid::parse_str(&pure_run.plan.scope.run_id).unwrap();
        let pure_parent_id = operation_dag_parent_workflow_id_v2(&pure_run.plan.scope.run_id);
        let pure_request_id = Uuid::new_v4();
        let pure_approval_id = Uuid::new_v4();
        let pure_expected = expected_for(pure_run);
        let pure_contract = pure_routes.catalog()
            .contract(&pure_run.pinned_routes[0].operation).unwrap();
        let pure_deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 240;
        sqlx::query(
            "INSERT INTO pipeline.operation_dag_business_runs_v1
             (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
             VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '15 minutes')",
        ).bind(pure_id).bind(tenant).bind(&context)
         .bind(pure_run.plan.hash.as_str()).bind(&pure_parent_id)
         .execute(&business_pool).await.expect("independent business root for pure Start");
        sqlx::query(
            "INSERT INTO pipeline.operation_dag_one_step_request_approvals_v1
             (authorization_event_id,request_id,run_id,tenant_id,principal_id,policy_id,
              context_key,operation_id,operation_revision,parent_workflow_id,plan_hash,
              expected_start,limits,expires_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::jsonb,$13::jsonb,
                     clock_timestamp()+interval '5 minutes')",
        ).bind(pure_approval_id).bind(pure_request_id).bind(pure_id).bind(tenant)
         .bind("ci-authenticated-user").bind("ci-safe-zero-cost-v1").bind(&context)
         .bind(pure_contract.reference.id.as_str())
         .bind(i32::try_from(pure_contract.reference.revision).unwrap())
         .bind(&pure_parent_id).bind(pure_run.plan.hash.as_str())
         .bind(serde_json::to_string(&pure_expected).unwrap())
         .bind(serde_json::to_string(&limits).unwrap())
         .execute(&approver_pool).await.expect("distinct pure-operation user approval");

        let pure_operator_catalog = mixed_v6_governance_issuer_catalog(pure_run, pure_routes);
        let issue_intent = private.request(&format!("pure-user-approved-{index}"),
            &serde_json::to_string(&RequestIssuerIntent {
                schema_revision: 1,
                request_id: pure_request_id,
                authorization_event_id: pure_approval_id,
                run: pure_run,
                expected: &pure_expected,
                context_key: &context,
                expires_at_unix_seconds: pure_deadline as i64,
            }).unwrap());
        let issued = Command::new(binary("INTEGRATION_ONE_STEP_REQUEST_ISSUER_BIN"))
            .arg("--issue-request-v1").arg(&issue_intent)
            .env("ALEGRIA_ENABLE_ONE_STEP_REQUEST_ISSUER_V1", "true")
            .env("ONE_STEP_REQUEST_ISSUER_DATABASE_URL_FILE", &request_dsn)
            .env("ONE_STEP_REQUEST_ISSUER_TRUSTED_CATALOG_FILE", &pure_operator_catalog)
            .env_remove("DATABASE_URL")
            .output().expect("separate pure request issuer");
        assert!(issued.status.success(), "{index}: request issuer: stdout={} stderr={}",
            String::from_utf8_lossy(&issued.stdout),
            String::from_utf8_lossy(&issued.stderr));
        let issued_report: serde_json::Value = serde_json::from_slice(&issued.stdout).unwrap();
        assert_eq!(issued_report["status"], "one_step_request_issued_not_started");

        let pure_intent = private.request(&format!("pure-start-{index}"),
            &serde_json::to_string(&GrantedStartIntent {
                schema_revision: 1,
                request_id: pure_request_id,
                intent: ExactStartIntent {
                    schema_revision: 1,
                    run: pure_run,
                    context_key: &context,
                    expected: &pure_expected,
                    expires_at_unix_seconds: pure_deadline,
                },
            }).unwrap());
        let pure_signed = operation_protocol::seal_one_step_admission_for_private_fixture_v1(
            &pure_request_id.to_string(),
            "https://auth.example.test", "https://mcp.example.test/mcp",
            "ci-oauth-subject", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,
            &Uuid::new_v4().to_string(), &signer_key,
        );
        let pure_proof = private.request(&format!("pure-proof-{index}"),
            &serde_json::to_string(&pure_signed).unwrap());
        let run_pure_start = |principal: &str, subject: &str| {
            Command::new(binary("INTEGRATION_PG_EFFECT_WORKER_BIN"))
                .arg("--operator-start-signed-granted-single-v1")
                .arg(&pure_proof).arg(&pure_intent)
                .env("ALEGRIA_ENABLE_SITE_ARCHITECT_SINGLE_V2_START", "true")
                .env("ALEGRIA_ENABLE_SITE_ARCHITECT_GRANTED_V1_START", "true")
                .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_KEY_FILE", &signer_key_file)
                .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_ISSUER", "https://auth.example.test")
                .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_RESOURCE", "https://mcp.example.test/mcp")
                .env("SITE_ARCHITECT_SIGNED_ADMISSION_V1_SUBJECT", subject)
                .env("SITE_ARCHITECT_SINGLE_V2_TENANT_ID", tenant)
                .env("SITE_ARCHITECT_SINGLE_V2_DATABASE_URL_FILE", &reader_dsn)
                .env("SITE_ARCHITECT_SINGLE_V2_TEMPORAL_URL", &temporal.temporal_url)
                .env("SITE_ARCHITECT_SINGLE_V2_TEMPORAL_NAMESPACE", &temporal.namespace)
                .env("SITE_ARCHITECT_SINGLE_V2_TRUSTED_CATALOG_FILE", &pure_operator_catalog)
                .env("SITE_ARCHITECT_GRANTED_V1_DATABASE_URL_FILE", &operator_dsn)
                .env("SITE_ARCHITECT_GRANTED_V1_PRINCIPAL_ID", principal)
                .env("SITE_ARCHITECT_GRANTED_V1_POLICY_ID", "ci-safe-zero-cost-v1")
                .env("SITE_ARCHITECT_GRANTED_V1_CONTEXT_KEY", &context)
                .env("SITE_ARCHITECT_GRANTED_V1_MAX_LIMITS_FILE", &limits_path)
                .env_remove("DATABASE_URL")
                .output().expect("one-use signed pure Start operator")
        };
        assert!(!run_pure_start("forged-user", "ci-oauth-subject").status.success(),
            "{index}: cross-principal request must not authorize Temporal Start");
        assert!(!run_pure_start("ci-authenticated-user", "forged-subject").status.success(),
            "{index}: copied SQL grant must not authorize another signed OAuth subject");
        assert!(client.get_workflow_handle::<UntypedWorkflow>(pure_parent_id.clone())
            .describe(WorkflowDescribeOptions::builder().build()).await.is_err(),
            "{index}: forged pure Start must have no server workflow");
        let started = run_pure_start("ci-authenticated-user", "ci-oauth-subject");
        assert!(started.status.success(), "{index}: approved pure Start: stdout={} stderr={}",
            String::from_utf8_lossy(&started.stdout),
            String::from_utf8_lossy(&started.stderr));
        let accepted: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
        assert_eq!(accepted["status"],
            "temporal_start_acknowledged_grant_pending_independent_history");
        assert!(!run_pure_start("ci-authenticated-user", "ci-oauth-subject").status.success(),
            "{index}: one approved pure operation may Start exactly once");
        let pure_parent = client.get_workflow_handle::<UntypedWorkflow>(pure_parent_id);
        let completed = timeout(Duration::from_secs(90),
            pure_parent.get_result(WorkflowGetResultOptions::default())).await
            .expect("pure granted Start result timed out")
            .expect("pure granted Start owner Child failed");
        let pure_receipt: OperationDagReceiptV1 = completed.to_value(&PayloadConverter::serde_json());
        assert_eq!(pure_receipt.order, vec!["one_step".into()]);
        assert_eq!(pure_receipt.owner_results.len(), 1);
        assert_eq!(pure_receipt.produced_evidence.len(), 1);
        assert!(serde_json::from_slice::<serde_json::Value>(
            &pure_receipt.artifacts[&"result".into()].payload).unwrap().is_object());
        let history = pure_parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .into_events().await.expect("real pure grant parent History");
        for prefix in ["alegria-owner-validate-v1-", "alegria-owner-operation-v2-"] {
            assert_eq!(history.iter().filter(|event| matches!(
                &event.attributes,
                Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                    if a.workflow_id.starts_with(prefix)
            )).count(), 1, "{index}: one native {prefix} child in server History");
        }
        let grant_state: String = sqlx::query_scalar(
            "SELECT state FROM pipeline.operation_dag_one_step_request_grants_v1 WHERE request_id=$1",
        ).bind(pure_request_id).fetch_one(&pg.pool).await.unwrap();
        assert_eq!(grant_state, "start_outcome_unknown",
            "{index}: native Start cannot self-confirm SQL grant or owner receipt");
    }
}
