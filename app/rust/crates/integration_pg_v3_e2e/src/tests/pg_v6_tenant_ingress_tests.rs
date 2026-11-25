// Real native PostgreSQL LOGINs, distinct from the superuser that provisions
// tenant/context ownership, exercise the actual crawler/extractor/run writers.
// There is no request-header/GUC tenant authority and historical NULL evidence
// is deliberately not backfilled implicitly by this migration.

#[cfg(feature = "e2e")]
fn pg_v6_expect_provenance_rejection(error: sqlx::Error, operation: &str) {
    let sqlstate = error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.into_owned());
    assert_eq!(
        sqlstate.as_deref(),
        Some("AV001"),
        "{operation} should reject as an explicit tenant provenance violation: {error}"
    );
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_tenant_ingress_native_login_and_rls_provenance() {
    use sqlx::postgres::PgPoolOptions;

    let postgres = PostgresHarness::start()
        .await
        .expect("start actual PostgreSQL with ingress policy");
    let fixture = pg_v3_seed_review_fixture(&postgres.pool).await;
    let marker = Uuid::new_v4().simple().to_string();
    // Names only use locally-generated lowercase hex, never external strings.
    let login_a = format!("ci_ingress_a_{marker}");
    let login_b = format!("ci_ingress_b_{marker}");
    let login_unbound = format!("ci_ingress_unbound_{marker}");
    let tenant_b = format!("ci-ingress-b-{marker}");
    let context_b = format!("pg-v6:ingress:{marker}");
    let source_b = format!("https://ci-ingress-{marker}.example.gov/fees");
    let legacy_source = format!("https://ci-legacy-{marker}.example.gov/fees");
    let restricted_password = "ci_provenance_password";

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login_a} LOGIN NOINHERIT PASSWORD '{restricted_password}';
         CREATE ROLE {login_b} LOGIN NOINHERIT PASSWORD '{restricted_password}';
         CREATE ROLE {login_unbound} LOGIN NOINHERIT PASSWORD '{restricted_password}';
         GRANT CONNECT ON DATABASE alegria TO {login_a}, {login_b}, {login_unbound};
         GRANT USAGE ON SCHEMA raw, extracted, pipeline TO {login_a}, {login_b}, {login_unbound};
         GRANT USAGE ON SCHEMA site, kb TO {login_b};
         GRANT TEMPORARY ON DATABASE alegria TO {login_b};
         GRANT SELECT, INSERT, UPDATE ON raw.pages, raw.sections,
             extracted.rule_candidates, pipeline.execution_runs TO {login_a}, {login_b}, {login_unbound};
         GRANT USAGE ON SEQUENCE raw.pages_id_seq, raw.sections_id_seq TO {login_a}, {login_b}, {login_unbound};
         -- Deliberate overgrant in a disposable E2E database: TRUNCATE ignores RLS.
         GRANT TRUNCATE ON ALL TABLES IN SCHEMA
             serp,raw,kb,extracted,verified,site,monitoring,pipeline,system TO {login_b};"
    )))
    .execute(&postgres.pool)
    .await
    .expect("provision two real non-superuser PostgreSQL LOGINs");

    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ($1,'ci-pg-v3-tenant'),($2,$3)",
    )
    .bind(&login_a)
    .bind(&login_b)
    .bind(&tenant_b)
    .execute(&postgres.pool)
    .await
    .expect("DBA immutably binds actual native LOGIN identities");
    sqlx::query(
        "INSERT INTO kb.visa_contexts
         (context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,$2,'tourist','BY')",
    )
    .bind(&context_b)
    .bind(format!("X{}", &marker[..8]))
    .execute(&postgres.pool)
    .await
    .expect("provision second tenant actual canonical context");
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id)
         VALUES ($1,$2)",
    )
    .bind(&context_b)
    .bind(&tenant_b)
    .execute(&postgres.pool)
    .await
    .expect("bind context B immutably to tenant B");
    sqlx::query(
        "INSERT INTO kb.sources(source_key,source_type,source_label,trust_level)
         VALUES ($1,'government','Tenant B direct-source evidence',5)",
    )
    .bind(&source_b)
    .execute(&postgres.pool)
    .await
    .expect("provision B government source");

    let legacy_page: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages(url,domain,dtype)
         VALUES ($1,'ci-legacy.example.gov','government') RETURNING id",
    )
    .bind(&legacy_source)
    .fetch_one(&postgres.pool)
    .await
    .expect("create unattributed historical page with privileged legacy writer");
    let legacy_tenant: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM raw.pages WHERE id=$1")
            .bind(legacy_page)
            .fetch_one(&postgres.pool)
            .await
            .expect("inspect genuinely unowned historical row");
    assert_eq!(legacy_tenant, None, "legacy evidence must stay unclaimed");

    let url_b = postgres
        .database_url
        .replace("postgres:postgres_password@", &format!("{login_b}:{restricted_password}@"));
    let url_a = postgres
        .database_url
        .replace("postgres:postgres_password@", &format!("{login_a}:{restricted_password}@"));
    let url_unbound = postgres.database_url.replace(
        "postgres:postgres_password@",
        &format!("{login_unbound}:{restricted_password}@"),
    );
    let actor_b = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_b)
        .await
        .expect("connect as actual tenant B LOGIN");
    let actor_a = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_a)
        .await
        .expect("connect as actual tenant A LOGIN");
    let actor_unbound = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_unbound)
        .await
        .expect("connect as actual unprovisioned LOGIN");
    let unprovisioned_tenant: Option<String> =
        sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
            .fetch_one(&actor_unbound)
            .await
            .expect("unprovisioned native LOGIN has no tenant authority");
    assert_eq!(unprovisioned_tenant, None);
    let authenticated_tenant: String =
        sqlx::query_scalar("SELECT pipeline.current_session_tenant_v1()")
            .fetch_one(&actor_b)
            .await
            .expect("derive B's actual authenticated tenant");
    assert_eq!(authenticated_tenant, tenant_b);
    sqlx::query("SELECT set_config('alegria.tenant_id','ci-pg-v3-tenant',false)")
        .execute(&actor_b)
        .await
        .expect("untrusted GUC can be set but must not authorize another tenant");

    // Positive authentic ingress without any application-supplied tenant_id.
    let page_b: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages(url,domain,dtype)
         VALUES ($1,'ci-ingress.example.gov','government') RETURNING id",
    )
    .bind(&source_b)
    .fetch_one(&actor_b)
    .await
    .expect("crawler page B is stamped from immutable LOGIN ledger");
    let section_b: i64 = sqlx::query_scalar(
        "INSERT INTO raw.sections(page_id,content_md,content_hash)
         VALUES ($1,'Consular fee is 40 EUR','ci-ingress-source-v1') RETURNING id",
    )
    .bind(page_b)
    .fetch_one(&actor_b)
    .await
    .expect("extracted section inherits its parent page tenant");
    let owner_b: (String, String) = sqlx::query_as(
        "SELECT p.tenant_id,s.tenant_id FROM raw.pages p
         JOIN raw.sections s ON s.page_id=p.id WHERE s.id=$1",
    )
    .bind(section_b)
    .fetch_one(&actor_b)
    .await
    .expect("tenant B sees its own real crawler/extractor result under RLS");
    assert_eq!(owner_b, (tenant_b.clone(), tenant_b.clone()));

    let candidate_b = format!("ci_v6_candidate_{marker}");
    sqlx::query(
        "INSERT INTO extracted.rule_candidates
         (rule_candidate_id,context_key,raw_section_id,role,concept_canonical_key,
          raw_mention,confidence,evidence_section_id,evidence_quote,span_start,
          span_end,source_key,source_snapshot_hash,llm_provider,llm_model,prompt_version)
         VALUES ($1,$2,$3,'FEE_ITEM','consular_fee','40 EUR',0.98,
                 $3,'Consular fee is 40 EUR',0,21,$4,'ci-ingress-source-v1',
                 'chat_manual','gpt-6','ci_ingress@1')",
    )
    .bind(&candidate_b)
    .bind(&context_b)
    .bind(section_b)
    .bind(&source_b)
    .execute(&actor_b)
    .await
    .expect("actual extractor candidate inherits context plus source/evidence tenant");
    let run_b = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
         (run_id,workflow_run_id,workflow_type,context_key,status)
         VALUES ($1,$2,'seo_site_build',$3,'verifying')",
    )
    .bind(run_b)
    .bind(Uuid::new_v4().to_string())
    .bind(&context_b)
    .execute(&actor_b)
    .await
    .expect("actual canonical run derives tenant from DBA-provisioned context");
    let owned_candidates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM extracted.rule_candidates
         WHERE context_key=$1 AND tenant_id=$2",
    )
    .bind(&context_b)
    .bind(&tenant_b)
    .fetch_one(&actor_b)
    .await
    .expect("B candidate is visible with RLS");
    assert_eq!(owned_candidates, 1);
    let owned_runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1 AND tenant_id=$2",
    )
    .bind(run_b)
    .bind(&tenant_b)
    .fetch_one(&actor_b)
    .await
    .expect("B canonical run is visible with RLS");
    assert_eq!(owned_runs, 1);

    // V8: old 0007 explicitly granted this global SECURITY DEFINER readiness
    // oracle to PUBLIC. A real unrelated LOGIN could probe B's known keys and
    // learn a bit about B's otherwise RLS-hidden Truth state. The forward-only
    // migration must remove that privilege before production cutover.
    for (intruder, reason) in [
        (&actor_a, "tenant A cross-tenant readiness probe"),
        (&actor_b, "tenant B ingress readiness probe"),
        (&actor_unbound, "unbound readiness probe"),
    ] {
        let denied = sqlx::query_scalar::<_, bool>(
            "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
        )
        .bind(&tenant_b)
        .bind(&context_b)
        .bind(run_b)
        .fetch_one(intruder)
        .await
        .expect_err("unvetted LOGIN cannot query global owner-definer Truth readiness");
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("42501"),
            "{reason} must fail on EXECUTE ACL before disclosing a bit"
        );
    }

    // Test the PostgreSQL trigger trampoline, not just its ACL flag. An
    // attacker may CREATE a temporary relation and has USAGE on the original
    // trigger-function schema. CREATE TRIGGER must fail with 42501 before the
    // attacker can choose NEW fields for a privileged SECURITY DEFINER body.
    let mut temp_probe = actor_b.acquire().await.expect("pin attacker TEMP session");
    sqlx::query(
        "CREATE TEMP TABLE ci_v8_trigger_probe
         (task_type TEXT, scope_signature TEXT, tenant_id TEXT, run_id UUID, context_key TEXT)",
    )
    .execute(&mut *temp_probe)
    .await
    .expect("real untrusted LOGIN creates its own TEMP table");
    for (index, trigger_function) in [
        "pipeline.guard_execution_run_truth_context",
        "pipeline.guard_verified_truth_step_context",
        "site.guard_truth_review_hitl_context",
        "pipeline.enforce_pg_effect_admission_tenant_v1",
        "site.bind_new_truth_review_tenant_v1",
        "pipeline.enforce_pg_effect_receipt_tenant_v1",
        "kb.guard_truth_source_authority_writer_v1",
        "raw.stamp_ingress_page_tenant_v1",
        "raw.stamp_ingress_section_tenant_v1",
        "pipeline.stamp_ingress_run_tenant_v1",
        "extracted.stamp_ingress_candidate_tenant_v1",
        "pipeline.guard_qdrant_projection_admission_v1",
    ]
    .into_iter()
    .enumerate()
    {
        let denied = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TRIGGER ci_v8_probe_{index} BEFORE INSERT ON ci_v8_trigger_probe \
             FOR EACH ROW EXECUTE FUNCTION {trigger_function}()"
        )))
        .execute(&mut *temp_probe)
        .await
        .expect_err("unvetted LOGIN cannot attach privileged trigger to attacker-created table");
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("42501"),
            "{trigger_function} must refuse CREATE TRIGGER despite attacker USAGE/TEMP"
        );
    }
    drop(temp_probe);

    // A reviewed, separate operator LOGIN is deliberately bound to tenant B
    // before receiving the narrow function EXECUTE grant. Even with that grant
    // it must return FALSE for A's otherwise-ready canonical context/run.
    let operator_login_b = format!("ci_v8_operator_b_{marker}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {operator_login_b} LOGIN NOINHERIT PASSWORD '{restricted_password}';
         GRANT CONNECT ON DATABASE alegria TO {operator_login_b};
         GRANT USAGE ON SCHEMA pipeline TO {operator_login_b};
         INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('{operator_login_b}','{tenant_b}');"
    )))
    .execute(&postgres.pool)
    .await
    .expect("DBA provisions separate LOGIN with immutable tenant B binding");
    let vetted: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT r.rolcanlogin, r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                OR r.rolcreatedb OR r.rolreplication,
                EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid),
                EXISTS(SELECT 1 FROM pipeline.tenant_role_ownership b
                       WHERE b.role_name=r.rolname AND b.tenant_id=$2)
         FROM pg_catalog.pg_roles r WHERE r.rolname=$1",
    )
    .bind(&operator_login_b)
    .bind(&tenant_b)
    .fetch_one(&postgres.pool)
    .await
    .expect("DBA verifies actual new operator role and exact tenant ledger");
    assert_eq!(vetted, (true, false, false, true));
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) \
         TO {operator_login_b}"
    )))
    .execute(&postgres.pool)
    .await
    .expect("DBA explicitly grants scoped definer helper to vetted operator LOGIN only");
    let operator_url = postgres.database_url.replace(
        "postgres:postgres_password@",
        &format!("{operator_login_b}:{restricted_password}@"),
    );
    let operator_b = PgPoolOptions::new()
        .max_connections(1)
        .connect(&operator_url)
        .await
        .expect("connect as reviewed, separate operator tenant B LOGIN");
    let own_truth_ready: bool = sqlx::query_scalar(
        "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
    )
    .bind(&tenant_b)
    .bind(&context_b)
    .bind(run_b)
    .fetch_one(&operator_b)
    .await
    .expect("granted operator reads its own canonical Truth readiness");
    assert!(own_truth_ready, "tenant B's actual candidate/run must be ready");
    let admin_a_ready: bool = sqlx::query_scalar(
        "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
    )
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(fixture.command.run_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("independent DBA sees canonical tenant A Truth evidence");
    assert!(admin_a_ready, "negative test requires otherwise-ready tenant A Truth");
    let other_truth_ready: bool = sqlx::query_scalar(
        "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
    )
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(fixture.command.run_id)
    .fetch_one(&operator_b)
    .await
    .expect("vetted operator must get a deterministic cross-tenant FALSE, never leak");
    assert!(!other_truth_ready, "even an EXECUTE-granted operator cannot probe tenant A");

    // RLS does not apply to TRUNCATE. Even an explicitly overgranted native
    // tenant LOGIN must receive a typed statement-trigger denial, including
    // when CASCADE would otherwise erase records owned by another tenant.
    for table in [
        "extracted.rule_candidates",
        "pipeline.execution_runs",
        "pipeline.pg_effect_v3_receipts",
        "pipeline.qdrant_projection_v1_admissions",
        "pipeline.qdrant_projection_v1_attempts",
        "pipeline.qdrant_projection_v1_receipts",
    ] {
        let deliberately_overgranted: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,'TRUNCATE')")
                .bind(table)
                .fetch_one(&actor_b)
                .await
                .expect("inspect authentic B privilege after intentional overgrant");
        assert!(
            deliberately_overgranted,
            "test must prove trigger protection even when {table} TRUNCATE is overgranted"
        );
        let installed_guard: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_trigger
             WHERE tgrelid=$1::regclass
               AND tgname='deny_tenant_scoped_truncate_v1' AND NOT tgisinternal",
        )
        .bind(table)
        .fetch_one(&postgres.pool)
        .await
        .expect("inspect production target TRUNCATE guard");
        assert_eq!(installed_guard, 1, "{table} must have its own fail-closed guard");
        let denied = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "TRUNCATE TABLE {table} CASCADE"
        )))
        .execute(&actor_b)
        .await
        .expect_err("overgranted native LOGIN must not TRUNCATE RLS-protected ledger");
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("23514"),
            "{table} should fail from a BEFORE TRUNCATE guard, not merely from ACL or FK checks"
        );
    }
    let protected_run_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1")
            .bind(run_b)
            .fetch_one(&actor_b)
            .await
            .expect("canonical run survives blocked CASCADE TRUNCATE");
    assert_eq!(protected_run_count, 1);
    let protected_candidate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM extracted.rule_candidates WHERE rule_candidate_id=$1",
    )
    .bind(&candidate_b)
    .fetch_one(&actor_b)
    .await
    .expect("Truth source candidate survives blocked CASCADE TRUNCATE");
    assert_eq!(protected_candidate_count, 1);

    // RLS hides evidence, candidate and run rows from the other native LOGIN.
    for table_count in [
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM raw.pages WHERE id=$1")
            .bind(page_b)
            .fetch_one(&actor_a)
            .await
            .expect("A reads B page through RLS"),
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM raw.sections WHERE id=$1")
            .bind(section_b)
            .fetch_one(&actor_a)
            .await
            .expect("A reads B section through RLS"),
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM extracted.rule_candidates WHERE rule_candidate_id=$1",
        )
            .bind(&candidate_b)
            .fetch_one(&actor_a)
            .await
            .expect("A reads B candidate through RLS"),
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1")
            .bind(run_b)
            .fetch_one(&actor_a)
            .await
            .expect("A reads B run through RLS"),
    ] {
        assert_eq!(table_count, 0, "RLS must hide all B-owned ingress artifacts from A");
    }

    let a_page: i64 = sqlx::query_scalar(
        "SELECT s.page_id FROM raw.sections s
         JOIN extracted.rule_candidates c ON c.raw_section_id=s.id
         WHERE c.rule_candidate_id=$1",
    )
    .bind(&fixture.candidate_id)
    .fetch_one(&actor_a)
    .await
    .expect("authentic A can see its own seeded evidence under RLS");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM raw.pages WHERE id=$1")
            .bind(a_page)
            .fetch_one(&actor_b)
            .await
            .expect("B reads A raw page under RLS"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM raw.pages WHERE id=$1")
            .bind(legacy_page)
            .fetch_one(&actor_b)
            .await
            .expect("B reads unowned historical raw page under RLS"),
        0
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "INSERT INTO raw.pages(url,domain,dtype,tenant_id)
             VALUES ($1,'ci-unbound.example.gov','government',$2)",
        )
        .bind(format!("https://ci-unbound-{marker}.example.gov"))
        .bind(&tenant_b)
        .execute(&actor_unbound)
        .await
        .expect_err("unprovisioned native LOGIN cannot self-issue tenant authority"),
        "unprovisioned tenant claim",
    );
    let unbound_insert = sqlx::query(
        "INSERT INTO raw.pages(url,domain,dtype)
         VALUES ($1,'ci-unbound.example.gov','government')",
    )
    .bind(format!("https://ci-unbound-null-{marker}.example.gov"))
    .execute(&actor_unbound)
    .await
    .expect_err("unprovisioned native LOGIN cannot bypass RLS by using NULL");
    assert_eq!(
        unbound_insert
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501"),
        "ordinary unbound LOGIN must be blocked by tenant RLS"
    );

    // Every forged direct writer attempt returns our typed AV001 denial from
    // the real before-write trigger, prior to policy WITH CHECK.
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "INSERT INTO raw.pages(url,domain,dtype,tenant_id)
             VALUES ($1,'ci-spoof.example.gov','government','ci-pg-v3-tenant')",
        )
        .bind(format!("https://ci-spoof-{marker}.example.gov"))
        .execute(&actor_b)
        .await
        .expect_err("forged page tenant must never enter database"),
        "forged page tenant",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "INSERT INTO raw.sections(page_id,content_md) VALUES ($1,'forged cross-tenant section')",
        )
            .bind(a_page)
            .execute(&actor_b)
            .await
            .expect_err("B cannot attach section to A source"),
        "cross-tenant section",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("INSERT INTO raw.sections(page_id,content_md) VALUES ($1,'unowned section')")
            .bind(legacy_page)
            .execute(&actor_b)
            .await
            .expect_err("B cannot claim historical NULL source"),
        "historical page appropriation",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "INSERT INTO pipeline.execution_runs
             (run_id,workflow_run_id,workflow_type,context_key,status)
             VALUES ($1,$2,'seo_site_build',$3,'verifying')",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4().to_string())
        .bind(&fixture.command.context_key)
        .execute(&actor_b)
        .await
        .expect_err("tenant B cannot forge canonical run A"),
        "cross-tenant canonical run",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "INSERT INTO extracted.rule_candidates
             (rule_candidate_id,context_key,raw_section_id,role,concept_canonical_key,
              raw_mention,confidence,evidence_section_id,evidence_quote,span_start,
              span_end,source_key,source_snapshot_hash,llm_provider,llm_model,prompt_version)
             VALUES ($1,$2,$3,'FEE_ITEM','cross_tenant_fee','40 EUR',0.95,
                     $3,'Consular fee is 40 EUR',0,21,$4,'ci-ingress-source-v1',
                     'chat_manual','gpt-6','ci_ingress@1')",
        )
        .bind(format!("ci_v6_forged_candidate_{marker}"))
        .bind(&fixture.command.context_key)
        .bind(section_b)
        .bind(&source_b)
        .execute(&actor_b)
        .await
        .expect_err("tenant B candidate cannot enter A Truth context"),
        "cross-tenant candidate evidence",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("UPDATE raw.pages SET tenant_id='ci-pg-v3-tenant' WHERE id=$1")
            .bind(page_b)
            .execute(&actor_b)
            .await
            .expect_err("B cannot reassign its persisted page to A"),
        "raw page reassignment",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("UPDATE raw.sections SET page_id=$1 WHERE id=$2")
            .bind(a_page)
            .bind(section_b)
            .execute(&actor_b)
            .await
            .expect_err("B cannot reparent its persisted section under A"),
        "section reparenting",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query(
            "UPDATE extracted.rule_candidates SET context_key=$1 WHERE rule_candidate_id=$2",
        )
            .bind(&fixture.command.context_key)
            .bind(&candidate_b)
            .execute(&actor_b)
            .await
            .expect_err("B cannot rebind a candidate to A Truth context"),
        "candidate rebind",
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("UPDATE pipeline.execution_runs SET context_key=$1 WHERE run_id=$2")
            .bind(&fixture.command.context_key)
            .bind(run_b)
            .execute(&actor_b)
            .await
            .expect_err("B cannot rebind a canonical run to A Truth context"),
        "canonical run rebind",
    );
    let unowned_still_null: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM raw.pages WHERE id=$1")
            .bind(legacy_page)
            .fetch_one(&postgres.pool)
            .await
            .expect("historical NULL source still exists");
    assert_eq!(unowned_still_null, None, "legacy rows are quarantined, not silently transferred");

    // V7 preflight is a read-only DBA diagnostic, not an authorization
    // shortcut for native ingestion LOGINs and not an implicit backfill.
    let preflight_a: bool = sqlx::query_scalar(
        "SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)",
    )
    .bind(&login_a)
    .bind("ci-pg-v3-tenant")
    .fetch_one(&postgres.pool)
    .await
    .expect("restricted native LOGIN A satisfies immutable tenant-bound ingress preflight");
    assert!(preflight_a);
    pg_v6_expect_provenance_rejection(
        sqlx::query("SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)")
            .bind(&login_b)
            .bind(&tenant_b)
            .execute(&postgres.pool)
            .await
            .expect_err("B's explicit TRUNCATE overgrant blocks role cutover"),
        "overprivileged tenant LOGIN preflight",
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE TRUNCATE ON ALL TABLES IN SCHEMA
         serp,raw,kb,extracted,verified,site,monitoring,pipeline,system FROM {login_b};"
    )))
    .execute(&postgres.pool)
    .await
    .expect("DBA corrects the disposable role's overly broad TRUNCATE grants");
    let preflight_b: bool = sqlx::query_scalar(
        "SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)",
    )
    .bind(&login_b)
    .bind(&tenant_b)
    .fetch_one(&postgres.pool)
    .await
    .expect("B native tenant LOGIN can pass after the unsafe privilege is revoked");
    assert!(preflight_b);
    // A role can pass ordinary RLS and the existing destructive-privilege
    // checks while still being able to mint tenant context ownership or
    // promote an untrusted registry entry to authoritative Truth evidence.
    // These administrator/curator capabilities are not ingress privileges.
    for (privilege, table) in [
        ("UPDATE", "kb.sources"),
        ("UPDATE", "kb.visa_contexts"),
        ("INSERT", "pipeline.tenant_context_ownership"),
        ("INSERT", "pipeline.tenant_role_ownership"),
        ("INSERT", "site.seo_hitl_tasks"),
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT {privilege} ON {table} TO {login_b}"
        )))
        .execute(&postgres.pool)
        .await
        .expect("DBA introduces an isolated test-only curator/authority overgrant");
        pg_v6_expect_provenance_rejection(
            sqlx::query("SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)")
                .bind(&login_b)
                .bind(&tenant_b)
                .execute(&postgres.pool)
                .await
                .expect_err("native tenant ingress cannot possess curator/DBA authority"),
            table,
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE {privilege} ON {table} FROM {login_b}"
        )))
        .execute(&postgres.pool)
        .await
        .expect("DBA removes the test-only curator/authority overgrant");
    }
    let privilege_recovery: bool = sqlx::query_scalar(
        "SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)",
    )
    .bind(&login_b)
    .bind(&tenant_b)
    .fetch_one(&postgres.pool)
    .await
    .expect("cutover preflight recovers after all forbidden authority grants are removed");
    assert!(privilege_recovery);
    pg_v6_expect_provenance_rejection(
        sqlx::query("SELECT pipeline.assert_tenant_ingress_login_v1($1::name,$2::text)")
            .bind(&login_unbound)
            .bind(&tenant_b)
            .execute(&postgres.pool)
            .await
            .expect_err("unprovisioned LOGIN cannot pass prospective tenant preflight"),
        "unbound tenant LOGIN preflight",
    );
    let forbidden_census = sqlx::query("SELECT * FROM pipeline.tenant_historical_quarantine_v1()")
        .execute(&actor_a)
        .await
        .expect_err("raw historical census is DBA-only and must not expose other tenants");
    assert_eq!(
        forbidden_census
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501"),
        "tenant LOGIN must lack privileged census EXECUTE"
    );
    let census: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT relation_name,unattributed,inconsistent
         FROM pipeline.tenant_historical_quarantine_v1() ORDER BY relation_name",
    )
    .fetch_all(&postgres.pool)
    .await
    .expect("DBA reads all ten real tenant-bearing owner relation counts");
    assert_eq!(census.len(), 10, "every tenant-bearing RLS table is inventoried");
    let legacy_page_census = census
        .iter()
        .find(|(relation, _, _)| relation == "raw.pages")
        .expect("raw page history must be included in migration census");
    assert_eq!(legacy_page_census.1, 1, "historical unowned page remains quarantined");
    assert_eq!(
        census.iter().map(|(_, _, inconsistent)| inconsistent).sum::<i64>(),
        0,
        "both valid native tenants have consistent actual source/context ownership"
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("SELECT pipeline.assert_no_tenant_quarantine_v1()")
            .execute(&postgres.pool)
            .await
            .expect_err("unattributed historical evidence must block attested cutover preflight"),
        "historical NULL quarantine preflight",
    );
    let preflight_denied = sqlx::query_scalar::<_, bool>(
        "SELECT pipeline.assert_tenant_force_rls_preflight_v1()",
    )
    .fetch_one(&actor_b)
    .await
    .expect_err("tenant operator/ingress LOGIN cannot invoke DBA-only FORCE RLS preflight");
    assert_eq!(
        preflight_denied
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501"),
        "ordinary tenant login must not enumerate global FORCE RLS readiness"
    );
    pg_v6_expect_provenance_rejection(
        sqlx::query("SELECT pipeline.assert_tenant_force_rls_preflight_v1()")
            .execute(&postgres.pool)
            .await
            .expect_err("production cutover cannot pass without real FORCE policies/history"),
        "pre-cutover production FORCE RLS NO-GO",
    );
    let forced_in_test: bool = sqlx::query_scalar(
        "SELECT relforcerowsecurity FROM pg_catalog.pg_class
         WHERE oid='raw.pages'::regclass",
    )
    .fetch_one(&postgres.pool)
    .await
    .expect("independently verify read-only preflight did not FORCE RLS");
    assert!(!forced_in_test, "V8 preflight must not secretly enable FORCE RLS");
    let malformed_label = sqlx::query(
        "INSERT INTO raw.pages(url,domain,dtype,tenant_id)
         VALUES ($1,'ci-malformed.example.gov','government',' ci-pg-v3-tenant ')",
    )
    .bind(format!("https://ci-malformed-{marker}.example.gov"))
    .execute(&postgres.pool)
    .await
    .expect_err("new tenant labels must be canonical despite historical NOT VALID check");
    assert_eq!(
        malformed_label
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("23514"),
        "forward-only NOT VALID check still rejects new malformed tenant labels"
    );
    // An authorized DBA can plan historical repair, but unsafe parent
    // reassignment of an already-ingested page must be visible, not masked.
    sqlx::query("UPDATE raw.pages SET tenant_id='ci-pg-v3-tenant' WHERE id=$1")
        .bind(page_b)
        .execute(&postgres.pool)
        .await
        .expect("simulate audited historical ownership mismatch in disposable fixture");
    let mismatched_edges: Vec<(String, i64)> = sqlx::query_as(
        "SELECT relation_name,inconsistent
         FROM pipeline.tenant_historical_quarantine_v1()
         WHERE inconsistent > 0 ORDER BY relation_name",
    )
    .fetch_all(&postgres.pool)
    .await
    .expect("DBA receives read-only mismatch inventory");
    assert!(
        mismatched_edges.iter().any(|(relation, count)|
            relation == "raw.sections" && *count == 1
        ),
        "census must flag the section whose parent was reassigned to another tenant"
    );
    assert!(
        mismatched_edges.iter().any(|(relation, count)|
            relation == "extracted.rule_candidates" && *count == 1
        ),
        "census must flag canonical candidate source provenance after parent reassignment"
    );
    let no_implicit_claim: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM raw.pages WHERE id=$1")
            .bind(legacy_page)
            .fetch_one(&postgres.pool)
            .await
            .expect("read-only preflight has not changed historical unowned source");
    assert_eq!(no_implicit_claim, None, "preflight must never mint tenant ownership");

    eprintln!("CI_PG_V6_NATIVE_TENANT_INGRESS_AND_RLS_PROVENANCE=PASS");
    eprintln!("CI_PG_V7_TENANT_HISTORICAL_QUARANTINE_AND_ROLE_PREFLIGHT=PASS");
    eprintln!("CI_PG_V8_TENANT_FORCE_RLS_READINESS_AND_DEFINER_ACL_DENIALS=PASS");
}
