// Opt-in experimental second external effect: PostgreSQL role/tenant-bound
// admission -> committed uncertainty fence -> REAL Qdrant upsert -> full REST
// readback -> durable receipt. Temporal child History is not yet attested.

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn admitted_qdrant_keyword_projection_recovers_after_receipt_failure_without_second_rpc() {
    use projection_delivery::admitted_qdrant_projection_v1::{
        execute_admitted_qdrant_projection_v1, QdrantKeywordProjectionCommandV1,
        QdrantProjectionOutcomeV1,
    };

    let pg = PostgresHarness::start().await.expect("real PG with schema");
    let qdrant = QdrantHarness::start().await.expect("real Qdrant HTTP+gRPC");
    let _env = env_overrides::EnvOverrideGuard::apply([
        ("QDRANT_URL".to_string(), qdrant.qdrant_url.clone()),
        ("QDRANT_REST_URL".to_string(), qdrant.qdrant_rest_url.clone()),
        ("QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(), "true".into()),
        ("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1".to_string(), "1".into()),
    ]);
    let admission = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let context_key = format!("qdrant-operator:context:{admission}");
    let keyword_key = format!("keyword-{admission}");
    let tenant = "ci-qdrant-operator-tenant";

    sqlx::query(
        "INSERT INTO kb.visa_contexts
           (context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ES','tourist','BY')",
    )
    .bind(&context_key)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context_key)
    .bind(tenant)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
           (run_id,workflow_run_id,workflow_type,context_key,status,tenant_id)
         VALUES ($1,$2,'seo_site_build',$3,'verifying',$4)",
    )
    .bind(run_id)
    .bind("ci-parent-run")
    .bind(&context_key)
    .bind(tenant)
    .execute(&pg.pool)
    .await
    .unwrap();

    // No bypassing the admission/worker identity split with a public grant.
    sqlx::query("CREATE ROLE qdrant_ci_operator LOGIN PASSWORD 'operator_pw'")
        .execute(&pg.pool).await.unwrap();
    sqlx::query("CREATE ROLE qdrant_ci_executor LOGIN PASSWORD 'executor_pw'")
        .execute(&pg.pool).await.unwrap();
    sqlx::query("CREATE ROLE qdrant_ci_wrong_tenant LOGIN PASSWORD 'wrong_pw'")
        .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('qdrant_ci_operator','ci-qdrant-operator-tenant'),
                ('qdrant_ci_executor','ci-qdrant-operator-tenant'),
                ('qdrant_ci_wrong_tenant','another-tenant')",
    )
    .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT USAGE ON SCHEMA pipeline TO qdrant_ci_operator,qdrant_ci_executor,qdrant_ci_wrong_tenant",
    )
    .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT INSERT ON pipeline.qdrant_projection_v1_admissions TO qdrant_ci_operator",
    )
    .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT SELECT, UPDATE(revoked_at) ON pipeline.qdrant_projection_v1_admissions TO qdrant_ci_operator",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT SELECT ON pipeline.qdrant_projection_v1_admissions TO qdrant_ci_executor,qdrant_ci_wrong_tenant",
    )
    .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT EXECUTE ON FUNCTION pipeline.begin_qdrant_projection_v1(UUID,BYTEA),
             pipeline.confirm_qdrant_projection_v1(UUID,BYTEA)
             TO qdrant_ci_executor,qdrant_ci_wrong_tenant,qdrant_ci_operator",
    )
    .execute(&pg.pool).await.unwrap();
    // Intentional ACL drift in a disposable database. SQL triggers must
    // preserve the provider-attempt fence even when the role is overgranted.
    sqlx::raw_sql(
        "GRANT SELECT, UPDATE, DELETE ON pipeline.qdrant_projection_v1_attempts,
             pipeline.qdrant_projection_v1_receipts TO qdrant_ci_executor;
         GRANT UPDATE ON pipeline.qdrant_projection_v1_admissions
             TO qdrant_ci_operator,qdrant_ci_executor;
         GRANT SELECT ON pipeline.qdrant_projection_v1_admissions TO qdrant_ci_operator",
    )
    .execute(&pg.pool).await.unwrap();

    let operator_url = pg.database_url.replace(
        "postgres:postgres_password", "qdrant_ci_operator:operator_pw",
    );
    let executor_url = pg.database_url.replace(
        "postgres:postgres_password", "qdrant_ci_executor:executor_pw",
    );
    let wrong_tenant_url = pg.database_url.replace(
        "postgres:postgres_password", "qdrant_ci_wrong_tenant:wrong_pw",
    );
    let operator = sqlx::PgPool::connect(&operator_url).await.unwrap();
    let executor = sqlx::PgPool::connect(&executor_url).await.unwrap();
    let wrong_tenant = sqlx::PgPool::connect(&wrong_tenant_url).await.unwrap();
    for table in [
        "pipeline.qdrant_projection_v1_attempts",
        "pipeline.qdrant_projection_v1_receipts",
    ] {
        for privilege in ["SELECT", "UPDATE", "DELETE"] {
            let granted: bool = sqlx::query_scalar(
                "SELECT has_table_privilege(current_user,$1,$2)",
            )
            .bind(table).bind(privilege).fetch_one(&executor).await.unwrap();
            assert!(granted, "test must prove disposable ACL overgrant: {table} {privilege}");
        }
    }
    let command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: admission,
        tenant_id: tenant.into(),
        context_key: context_key.clone(),
        keyword_key: keyword_key.clone(),
        vector: vec![0.123_f32; 1024],
    };
    let bytes = serde_json::to_vec(&command).unwrap();
    sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,
            child_workflow_run_id,child_started_event_id,operation_instance,
            plan_sha256,command_sha256,command_bytes,expires_at)
         VALUES ($1,$2,$3,$4,$5,'ci-parent','ci-parent-run','ci-qdrant-child',
                 'ci-child-run',17,'keyword-projection',repeat('a',64),
                 encode(public.digest($6::bytea,'sha256'),'hex'),$6,
                 clock_timestamp() + interval '10 minutes')",
    )
    .bind(admission)
    .bind(run_id)
    .bind(tenant)
    .bind(&context_key)
    .bind(&keyword_key)
    .bind(&bytes)
    .execute(&operator)
    .await
    .expect("operator PostgreSQL LOGIN owns tenant and admits exact bytes");

    // Incorrect tenant LOGIN sees no admission even if it forges app GUCs.
    sqlx::query("SELECT set_config('app.tenant_id', $1, false)")
        .bind(tenant)
        .execute(&wrong_tenant).await.unwrap();
    assert!(execute_admitted_qdrant_projection_v1(&wrong_tenant, admission).await.is_err());

    // SECURITY DEFINER reads admissions with the SQL function owner's rights;
    // therefore an RLS-hidden SELECT alone does not prove tenant isolation.
    // Hand the wrong-tenant LOGIN the stolen, exact UUID AND command bytes and
    // exercise BOTH privileged SQL entrypoints directly, bypassing Rust.
    for forbidden_pool in [&wrong_tenant, &operator] {
        // The operator has intentionally also been granted EXECUTE for this
        // adversarial test. Issuance and execution by one LOGIN are forbidden
        // even when a DBA accidentally grants both capabilities to it.
        for statement in [
            "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
            "SELECT pipeline.confirm_qdrant_projection_v1($1,$2)",
        ] {
            let denied = sqlx::query_scalar::<_, String>(statement)
                .bind(admission)
                .bind(&bytes)
                .fetch_one(forbidden_pool)
                .await
                .expect_err("SECURITY DEFINER must reject stolen cross-tenant or same-LOGIN authority");
            let code = denied.as_database_error().and_then(|error| error.code());
            assert_eq!(code.as_deref(), Some("AV001"),
                "SQL function must deny at its own tenant/operator boundary: {statement}: {denied}");
        }
    }
    let attempts_before_provider: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    let receipts_before_provider: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_receipts WHERE admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    assert_eq!((attempts_before_provider, receipts_before_provider), (0, 0),
        "denied direct SECURITY DEFINER calls cannot stage unknown RPCs or receipts");

    // Simulate a real PostgreSQL receipt failure *after* the Qdrant RPC.
    // This must leave a durable UNKNOWN barrier and the provider point intact.
    sqlx::query(
        "ALTER TABLE pipeline.qdrant_projection_v1_receipts
         ADD CONSTRAINT qdrant_ci_receipt_fault CHECK (FALSE)",
    )
    .execute(&pg.pool).await.unwrap();
    assert!(execute_admitted_qdrant_projection_v1(&executor, admission).await.is_err());
    let uncertain: String = sqlx::query_scalar(
        "SELECT state FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    assert_eq!(uncertain, "uncertain", "failed receipt cannot erase effect fence");
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_receipts WHERE admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    assert_eq!(receipts, 0);
    for statement in [
        "UPDATE pipeline.qdrant_projection_v1_attempts
             SET state='confirmed',confirmed_at=clock_timestamp() WHERE admission_id=$1",
        "DELETE FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    ] {
        let error = sqlx::query(statement).bind(admission).execute(&executor).await
            .expect_err("overgranted executor cannot reopen the uncertain attempt barrier");
        assert_eq!(error.as_database_error().and_then(|e| e.code()).as_deref(), Some("23514"));
    }

    // Verify Qdrant directly, independently of the SQL receipt and executor
    // result. The provider really accepted the write despite PG rejection.
    let approved_sha: String = sqlx::query_scalar(
        "SELECT command_sha256 FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    let metadata = serde_json::json!({
        "tenant_id": tenant,
        "context_key": context_key,
        "keyword_key": keyword_key,
        "operator_admission_id": admission.to_string(),
        "approved_command_sha256": approved_sha,
    });
    assert!(vector_qdrant::projection_commands::projection_readback_exact(
        "seo_keyword_clusters_4",
        &admission.to_string(),
        "keyword_cluster",
        &format!("tenant/{tenant}/context/{context_key}/keyword/{keyword_key}"),
        &command.vector,
        &metadata,
    ).await.expect("real Qdrant REST point GET after failed SQL receipt"));

    // gRPC independently sees the same point; this rules out a fake HTTP
    // success or a locally synthesized readback response.
    let retrieved = vector_qdrant::search_dense(
        &qdrant.client, "seo_keyword_clusters_4", command.vector.clone(), 10, None,
    ).await.expect("real Qdrant gRPC readback");
    assert!(retrieved.iter().any(|point| {
        vector_qdrant::scored_point_payload_map(point).get("operator_admission_id")
            == Some(&admission.to_string())
    }), "independent gRPC did not observe admitted external business projection");

    // A monotonic issuer revocation cannot pretend the already-started
    // provider RPC never happened. Reconciliation after revocation must keep
    // the immutable uncertainty barrier and recover exact remote success.
    sqlx::query(
        "UPDATE pipeline.qdrant_projection_v1_admissions
         SET revoked_at=clock_timestamp() WHERE admission_id=$1",
    ).bind(admission).execute(&operator).await
        .expect("independently credentialed issuer revokes an already-started Qdrant admission");

    sqlx::query(
        "ALTER TABLE pipeline.qdrant_projection_v1_receipts DROP CONSTRAINT qdrant_ci_receipt_fault",
    )
    .execute(&pg.pool).await.unwrap();
    assert_eq!(
        execute_admitted_qdrant_projection_v1(&executor, admission).await.unwrap(),
        QdrantProjectionOutcomeV1::Reconciled,
        "a second provider RPC is forbidden; exact readback must reconcile"
    );
    assert_eq!(
        execute_admitted_qdrant_projection_v1(&executor, admission).await.unwrap(),
        QdrantProjectionOutcomeV1::AlreadyConfirmed,
    );
    let committed: (String, String, bool) = sqlx::query_as(
        "SELECT CASE WHEN r.admission_id IS NOT NULL THEN 'confirmed' ELSE a.state END,
                a.state,r.exact_provider_readback
         FROM pipeline.qdrant_projection_v1_attempts a
         JOIN pipeline.qdrant_projection_v1_receipts r USING(admission_id)
         WHERE a.admission_id=$1",
    )
    .bind(admission)
    .fetch_one(&pg.pool).await.unwrap();
    assert_eq!(committed, ("confirmed".into(), "uncertain".into(), true),
        "the provider attempt is permanently uncertain; confirmation derives from receipt");
    for statement in [
        "UPDATE pipeline.qdrant_projection_v1_attempts
             SET state='confirmed',confirmed_at=clock_timestamp() WHERE admission_id=$1",
        "DELETE FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
        "UPDATE pipeline.qdrant_projection_v1_receipts
             SET exact_provider_readback=FALSE WHERE admission_id=$1",
        "DELETE FROM pipeline.qdrant_projection_v1_receipts WHERE admission_id=$1",
    ] {
        let error = sqlx::query(statement).bind(admission).execute(&executor).await
            .expect_err("overgranted executor cannot forge or erase immutable provider history");
        assert_eq!(error.as_database_error().and_then(|e| e.code()).as_deref(), Some("23514"));
    }

    // Crash after committing the PG uncertainty barrier but before sending
    // ANY provider RPC. A retry is still forbidden: it must observe the
    // missing readback, preserve UNKNOWN and not manufacture a receipt.
    let missing_admission = Uuid::new_v4();
    let missing_keyword = format!("not-yet-projected-{missing_admission}");
    let missing_command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: missing_admission,
        tenant_id: tenant.into(),
        context_key: context_key.clone(),
        keyword_key: missing_keyword.clone(),
        vector: vec![0.19_f32; 1024],
    };
    let missing_bytes = serde_json::to_vec(&missing_command).unwrap();
    sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,
            child_workflow_run_id,child_started_event_id,operation_instance,
            plan_sha256,command_sha256,command_bytes,expires_at)
         VALUES ($1,$2,$3,$4,$5,'ci-parent','ci-parent-run','ci-missing-child',
                 'ci-missing-run',18,'keyword-missing',repeat('b',64),
                 encode(public.digest($6::bytea,'sha256'),'hex'),$6,
                 clock_timestamp() + interval '1 second')",
    )
    .bind(missing_admission)
    .bind(run_id)
    .bind(tenant)
    .bind(&context_key)
    .bind(&missing_keyword)
    .bind(&missing_bytes)
    .execute(&operator).await.unwrap();
    let mut start_tx = executor.begin().await.unwrap();
    let started: String = sqlx::query_scalar(
        "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
    )
    .bind(missing_admission)
    .bind(&missing_bytes)
    .fetch_one(&mut *start_tx).await.unwrap();
    assert_eq!(started, "started");
    // Effect-first concurrent race: the new attempt transaction holds the
    // canonical run-status advisory fence and FOR SHARE lock. A concurrent
    // terminal writer must fail instead of committing a terminal state while
    // the first provider attempt is being durably admitted.
    let mut status_writer = pg.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='300ms'")
        .execute(&mut *status_writer).await.unwrap();
    let blocked_transition = sqlx::query(
        "UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1",
    ).bind(run_id).execute(&mut *status_writer).await
     .expect_err("terminal writer advanced despite concurrent Qdrant first-start fence");
    assert_eq!(blocked_transition.as_database_error().and_then(|err|err.code()).as_deref(),
        Some("55P03"));
    status_writer.rollback().await.unwrap();
    start_tx.commit().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let expired_reentry: String = sqlx::query_scalar(
        "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
    ).bind(missing_admission).bind(&missing_bytes)
     .fetch_one(&executor).await.unwrap();
    assert_eq!(expired_reentry,"uncertain",
        "expired AFTER initial start may never erase unknown-outcome barrier or permit re-RPC");
    assert!(execute_admitted_qdrant_projection_v1(&executor, missing_admission)
        .await.is_err(), "an uncertain RPC cannot be repeated even if no point exists");
    let missing_state: String = sqlx::query_scalar(
        "SELECT state FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    )
    .bind(missing_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(missing_state, "uncertain");
    assert_eq!(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_receipts WHERE admission_id=$1",
    ).bind(missing_admission).fetch_one(&pg.pool).await.unwrap(), 0);

    // Revoke a not-yet-started effect using its original authenticated
    // operator LOGIN. A tenant-matched executor, even overgranted UPDATE,
    // cannot impersonate the issuer or quietly edit command authority.
    let revoked_admission = Uuid::new_v4();
    let revoked_keyword = format!("revoke-before-begin-{revoked_admission}");
    let revoked_command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: revoked_admission,
        tenant_id: tenant.into(),
        context_key: context_key.clone(),
        keyword_key: revoked_keyword.clone(),
        vector: vec![0.21_f32; 1024],
    };
    let revoked_bytes = serde_json::to_vec(&revoked_command).unwrap();
    sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,
            child_workflow_run_id,child_started_event_id,operation_instance,
            plan_sha256,command_sha256,command_bytes,expires_at)
         VALUES ($1,$2,$3,$4,$5,'ci-parent','ci-parent-run','ci-revoked-child',
                 'ci-revoked-run',19,'keyword-revoked',repeat('c',64),
                 encode(public.digest($6::bytea,'sha256'),'hex'),$6,
                 clock_timestamp() + interval '10 minutes')",
    )
    .bind(revoked_admission).bind(run_id).bind(tenant)
    .bind(&context_key).bind(&revoked_keyword).bind(&revoked_bytes)
    .execute(&operator).await.unwrap();
    let executor_revocation = sqlx::query(
        "UPDATE pipeline.qdrant_projection_v1_admissions
            SET revoked_at=clock_timestamp() WHERE admission_id=$1",
    )
    .bind(revoked_admission).execute(&executor).await
    .expect_err("executor cannot revoke another LOGIN's admission even with UPDATE grant");
    assert_eq!(executor_revocation.as_database_error().and_then(|e| e.code()).as_deref(), Some("AV001"));
    let forged_rewrite = sqlx::query(
        "UPDATE pipeline.qdrant_projection_v1_admissions
            SET operation_instance='changed-after-admission',revoked_at=clock_timestamp()
          WHERE admission_id=$1",
    )
    .bind(revoked_admission).execute(&operator).await
    .expect_err("operator cannot change immutable plan/operation fields while revoking");
    assert_eq!(forged_rewrite.as_database_error().and_then(|e| e.code()).as_deref(), Some("AV001"));
    sqlx::query(
        "UPDATE pipeline.qdrant_projection_v1_admissions
            SET revoked_at='2999-01-01 00:00:00+00' WHERE admission_id=$1",
    )
    .bind(revoked_admission).execute(&operator).await
    .expect("same authenticated operator can monotonically revoke before begin");
    let server_revocation: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL AND revoked_at > issued_at
             AND revoked_at <= clock_timestamp()
         FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    )
    .bind(revoked_admission).fetch_one(&pg.pool).await.unwrap();
    assert!(server_revocation, "DB stores server time, never operator's forged future timestamp");
    let repeated_revoke = sqlx::query(
        "UPDATE pipeline.qdrant_projection_v1_admissions
            SET revoked_at=clock_timestamp() WHERE admission_id=$1",
    )
    .bind(revoked_admission).execute(&operator).await
    .expect_err("already-revoked admission is immutable");
    assert_eq!(repeated_revoke.as_database_error().and_then(|e| e.code()).as_deref(), Some("AV001"));
    let rejected_begin = sqlx::query_scalar::<_, String>(
        "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
    )
    .bind(revoked_admission).bind(&revoked_bytes).fetch_one(&executor).await
    .expect_err("revoke-before-begin must deny provider start");
    assert_eq!(rejected_begin.as_database_error().and_then(|e| e.code()).as_deref(), Some("AV001"));
    assert!(execute_admitted_qdrant_projection_v1(&executor, revoked_admission).await.is_err());
    let revoked_attempt_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    )
    .bind(revoked_admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(revoked_attempt_count, 0, "revocation cannot create a provider attempt");

    // A second, unauthorized or conflicting operator admission for the same
    // business keyword must fail before it can overwrite the Qdrant point.
    let conflict = sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
         (admission_id,run_id,tenant_id,context_key,keyword_key,
          parent_workflow_id,parent_workflow_run_id,child_workflow_id,
          child_workflow_run_id,child_started_event_id,operation_instance,
          plan_sha256,command_sha256,command_bytes,expires_at)
         SELECT gen_random_uuid(),run_id,tenant_id,context_key,keyword_key,
                'other-parent','other-parent-run','other-child','other-child-run',
                18,'other-instance',repeat('b',64),command_sha256,command_bytes,
                clock_timestamp()+ interval '10 minutes'
           FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    )
    .bind(admission)
    .execute(&pg.pool).await;
    assert!(conflict.is_err(), "same business keyword cannot be overwritten by a new admission");
}
