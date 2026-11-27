// Physical, separately credentialed PostgreSQL admission for an immutable
// tenant raw snapshot. No Temporal server is simulated and the issuing role
// cannot grant itself the independent business-run authority.

#[tokio::test]
#[serial]
async fn external_source_attestation_pg_issuer_reader_roles_and_revocation() {
    let pg = PostgresHarness::start().await.expect("disposable PostgreSQL");
    for (label, migration) in [
        ("Qdrant mixed owner", include_str!("../../../../../db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql")),
        ("PG generic owner", include_str!("../../../../../db/migrations/20261125_0018_pg_generic_owner_v1.sql")),
        ("Qdrant generic owner", include_str!("../../../../../db/migrations/20261125_0019_qdrant_generic_owner_v3.sql")),
        ("DAG V1 governance", include_str!("../../../../../db/migrations/20261125_0020_operation_dag_run_governance_v1.sql")),
        ("PG vector projection", include_str!("../../../../../db/migrations/20261125_0021_pg_vector_projection_v1.sql")),
        ("independent V2 business root", include_str!("../../../../../db/migrations/20261125_0022_pg_owner_v2_admission.sql")),
        ("trusted external source", include_str!("../../../../../db/migrations/20261127_0002_operation_dag_external_source_attestation_v1.sql")),
    ] {
        sqlx::raw_sql(migration)
            .execute(&pg.pool)
            .await
            .unwrap_or_else(|error| panic!("{label} migration failed: {error}"));
    }

    let nonce = Uuid::new_v4().simple().to_string();
    let ingress_name = format!("source_ingress_{nonce}");
    let business_name = format!("source_business_{nonce}");
    let issuer_name = format!("source_issuer_{nonce}");
    let inherited_writer_name = format!("source_writer_{nonce}");
    let reader_name = format!("source_reader_{nonce}");
    let foreign_name = format!("source_foreign_{nonce}");
    let tenant = format!("source-tenant-{nonce}");
    let foreign_tenant = format!("foreign-tenant-{nonce}");
    const PASSWORD: &str = "disposable_external_source_pg_test_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {ingress_name} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {business_name} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {issuer_name} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {reader_name} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {foreign_name} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {ingress_name},{business_name},{issuer_name},{reader_name},{foreign_name};
         GRANT USAGE ON SCHEMA pipeline TO {business_name},{issuer_name},{reader_name},{foreign_name};
         GRANT USAGE ON SCHEMA raw TO {ingress_name},{issuer_name};
         GRANT INSERT, SELECT ON raw.pages TO {ingress_name};
         GRANT UPDATE(content_hash,title,processed) ON raw.pages TO {ingress_name};
         GRANT SELECT(id,tenant_id,url,final_url,content_type,status_code,
                      raw_html,raw_html_bytes,content_hash)
             ON raw.pages TO {issuer_name};
         GRANT USAGE ON SEQUENCE raw.pages_id_seq TO {ingress_name};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business_name};"
    )))
    .execute(&pg.pool)
    .await
    .expect("create five separate least-privileged test roles");

    for role in [&ingress_name, &business_name, &issuer_name, &reader_name] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
             VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(&tenant)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ($1::name,$2)",
    )
    .bind(&foreign_name)
    .bind(&foreign_tenant)
    .execute(&pg.pool)
    .await
    .unwrap();

    let prefix = "postgres://postgres:postgres_password@";
    assert!(pg.database_url.starts_with(prefix));
    let as_role = |role: &str| {
        pg.database_url.replacen(
            prefix,
            &format!("postgres://{role}:{PASSWORD}@"),
            1,
        )
    };
    let ingress = sqlx::PgPool::connect(&as_role(&ingress_name)).await.unwrap();
    let business = sqlx::PgPool::connect(&as_role(&business_name)).await.unwrap();
    let issuer = sqlx::PgPool::connect(&as_role(&issuer_name)).await.unwrap();
    let reader = sqlx::PgPool::connect(&as_role(&reader_name)).await.unwrap();
    let foreign = sqlx::PgPool::connect(&as_role(&foreign_name)).await.unwrap();

    let target = "pipeline.operation_dag_external_source_attestations_v1";
    for role in [&business, &issuer, &reader, &foreign] {
        let permitted: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(session_user,
                'pipeline.operation_dag_external_source_attestations_v1','INSERT')",
        )
        .fetch_one(role)
        .await
        .unwrap();
        assert!(!permitted, "no source authority exists without explicit DBA grant");
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT INSERT, UPDATE ON {target} TO {issuer_name};
         GRANT SELECT(record_id) ON {target} TO {issuer_name};
         GRANT SELECT ON {target} TO {reader_name},{foreign_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    for forbidden in ["pipeline.operation_dag_business_runs_v1", "raw.pages"] {
        let right: bool = sqlx::query_scalar("SELECT has_table_privilege(session_user,$1,'INSERT')")
            .bind(forbidden)
            .fetch_one(&issuer)
            .await
            .unwrap();
        assert!(!right, "source issuer must not mint business roots or raw pages");
    }

    let run_id = Uuid::new_v4();
    let record_id = Uuid::new_v4();
    let context_key = format!("source-physical-{run_id}");
    let plan_hash = "a".repeat(64);
    let page_hash = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let payload_hash = "b".repeat(64);
    sqlx::query(
        "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ZZ','tourist','BY')",
    )
    .bind(&context_key)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context_key)
    .bind(&tenant)
    .execute(&pg.pool)
    .await
    .unwrap();
    let page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages
            (url,domain,dtype,raw_html,raw_html_bytes,content_hash)
         VALUES ('https://example.test/fixture','example.test','html','',0,$1)
         RETURNING id",
    )
    .bind(page_hash)
    .fetch_one(&ingress)
    .await
    .expect("separate authenticated ingestion writes immutable snapshot");
    let page_tenant: String =
        sqlx::query_scalar("SELECT tenant_id FROM raw.pages WHERE id=$1")
            .bind(page_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
    assert_eq!(page_tenant, tenant);
    sqlx::query("UPDATE raw.pages SET title='observed-before-attestation' WHERE id=$1")
        .bind(page_id)
        .execute(&ingress)
        .await
        .expect("tenant-owned raw metadata may change before source attestation");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
            (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '8 minutes')",
    )
    .bind(run_id)
    .bind(&tenant)
    .bind(&context_key)
    .bind(&plan_hash)
    .bind(format!("alegria-operation-dag-v2-{run_id}"))
    .execute(&business)
    .await
    .expect("separately issued business-run authority");

    let insert =
        "INSERT INTO pipeline.operation_dag_external_source_attestations_v1
         (record_id,run_id,tenant_id,context_key,plan_hash,artifact_id,
          artifact_schema,payload_hash,source_page_id,source_page_content_hash,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7::jsonb,$8,$9,$10,
                 clock_timestamp()+interval '4 minutes')";
    let schema = r#"{"id":"acquisition.classify_source_page.input","revision":1}"#;
    let issued = sqlx::query(insert)
        .bind(record_id)
        .bind(run_id)
        .bind(&tenant)
        .bind(&context_key)
        .bind(&plan_hash)
        .bind("source")
        .bind(&schema)
        .bind(&payload_hash)
        .bind(page_id)
        .bind(page_hash)
        .execute(&issuer)
        .await;
    issued.expect("source issuer may attest only a matching tenant-owned raw snapshot");
    let raw_sha256: String = sqlx::query_scalar(
        "SELECT source_page_raw_sha256
         FROM pipeline.operation_dag_external_source_attestations_v1
         WHERE record_id=$1",
    )
    .bind(record_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(
        raw_sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "the SQL issuer must bind the SHA256 of actual empty raw HTML"
    );

    // Give the actual ingress writer UPDATE permission so rejection must come
    // from the immutable attested-snapshot trigger, not from absent grants.
    assert!(sqlx::query(
        "UPDATE raw.pages SET title='forged-after-attestation' WHERE id=$1",
    )
    .bind(page_id)
    .execute(&ingress)
    .await
    .is_err(), "post-attestation title mutation must be rejected by snapshot trigger");
    assert!(sqlx::query("UPDATE raw.pages SET content_hash=$1 WHERE id=$2")
        .bind("c".repeat(64))
        .bind(page_id)
        .execute(&ingress)
        .await
        .is_err(), "attested raw bytes may not change even for their tenant ingress writer");
    sqlx::query("UPDATE raw.pages SET processed=true WHERE id=$1")
        .bind(page_id)
        .execute(&ingress)
        .await
        .expect("non-content processed progress remains mutable");

    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.operation_dag_external_source_attestations_v1
         WHERE record_id=$1 AND payload_hash=$2",
    )
    .bind(record_id)
    .bind(&payload_hash)
    .fetch_one(&reader)
    .await
    .unwrap();
    assert_eq!(visible, 1, "independent Start reader may read exact grant");
    let hidden: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.operation_dag_external_source_attestations_v1
         WHERE record_id=$1",
    )
    .bind(record_id)
    .fetch_one(&foreign)
    .await
    .unwrap();
    assert_eq!(hidden, 0, "cross-tenant source grant must be invisible");
    // Even accidental DBA provisioning of raw UPDATE makes this LOGIN an
    // ineligible independent issuer. The negative uses a distinct artifact:
    // UNIQUE(run_id,artifact_id) must not be the reason for rejection.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE ON raw.pages TO {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_misprovisioned_writer")
        .bind(&schema).bind(&payload_hash).bind(page_id).bind(page_hash)
        .execute(&issuer).await.is_err(),
        "raw UPDATE authority must disqualify a source-issuer LOGIN");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE ON raw.pages FROM {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    // A NOINHERIT issuer is still able to SET ROLE into any granted writer
    // membership. Reject source minting for such a login, even though direct
    // `has_table_privilege(session_user, raw.pages, UPDATE)` is false.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {inherited_writer_name} NOLOGIN;
         GRANT UPDATE ON raw.pages TO {inherited_writer_name};
         GRANT {inherited_writer_name} TO {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_noinherit_writer_membership")
        .bind(&schema).bind(&payload_hash).bind(page_id).bind(page_hash)
        .execute(&issuer).await.is_err(),
        "SET ROLE membership into raw writer must disqualify source issuer");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE {inherited_writer_name} FROM {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    assert!(sqlx::query(
        "INSERT INTO pipeline.operation_dag_external_source_attestations_v1
          (record_id,run_id,tenant_id,context_key,plan_hash,artifact_id,
           artifact_schema,payload_hash,source_page_id,source_page_content_hash,
           source_page_raw_sha256,expires_at)
         VALUES ($1,$2,$3,$4,$5,'source_forged_raw_digest',$6::jsonb,$7,$8,$9,$10,
                 clock_timestamp()+interval '4 minutes')"
    )
    .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
    .bind(&context_key).bind(&plan_hash).bind(&schema)
    .bind(&payload_hash).bind(page_id).bind(page_hash).bind("f".repeat(64))
    .execute(&issuer).await.is_err(),
    "caller cannot assert a raw HTML digest distinct from actual SQL bytes");
    let malformed_page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages
          (url,domain,dtype,raw_html,raw_html_bytes,content_hash)
         VALUES ('https://example.test/inconsistent-bytes','example.test',
                 'html','nonempty',0,$1) RETURNING id",
    )
    .bind("a".repeat(64))
    .fetch_one(&ingress)
    .await
    .unwrap();
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_bad_bytecount")
        .bind(&schema).bind(&payload_hash).bind(malformed_page_id).bind("a".repeat(64))
        .execute(&issuer).await.is_err(),
        "a source page with mismatched raw_html_bytes must not be admissible");
    let missing_html_page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages
          (url,domain,dtype,raw_html,raw_html_bytes,content_hash)
         VALUES ('https://example.test/missing-html','example.test',
                 'html',NULL,0,$1) RETURNING id",
    )
    .bind(page_hash)
    .fetch_one(&ingress)
    .await
    .unwrap();
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_without_html")
        .bind(&schema).bind(&payload_hash).bind(missing_html_page_id).bind(page_hash)
        .execute(&issuer).await.is_err(),
        "an asserted content hash without captured raw_html is not a source");
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&foreign_tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_tenant_spoof").bind(&schema)
        .bind(&payload_hash).bind(page_id).bind(page_hash)
        .execute(&issuer).await.is_err(), "tenant spoofing must fail before issuance");
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_bad_hash").bind(&schema)
        .bind(&payload_hash).bind(page_id).bind("c".repeat(64))
        .execute(&issuer).await.is_err(), "snapshot digest mismatch denied");
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind("d".repeat(64)).bind("source_bad_plan").bind(&schema)
        .bind(&payload_hash).bind(page_id).bind(page_hash)
        .execute(&issuer).await.is_err(), "immutable business plan mismatch denied");
    assert!(sqlx::query(
        "INSERT INTO pipeline.operation_dag_external_source_attestations_v1
          (record_id,run_id,tenant_id,context_key,plan_hash,artifact_id,
           artifact_schema,payload_hash,source_page_id,source_page_content_hash,expires_at)
         VALUES ($1,$2,$3,$4,$5,'source_expired',$6::jsonb,$7,$8,$9,
                 clock_timestamp()-interval '1 second')",
    )
    .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
    .bind(&context_key).bind(&plan_hash).bind(&schema)
    .bind(&payload_hash).bind(page_id).bind(page_hash)
    .execute(&issuer).await.is_err(), "already expired issuance denied");
    assert!(sqlx::query(
        "UPDATE pipeline.operation_dag_external_source_attestations_v1
         SET payload_hash=$1 WHERE record_id=$2",
    )
    .bind("e".repeat(64))
    .bind(record_id)
    .execute(&issuer)
    .await
    .is_err(), "issued source payload cannot be rewritten");
    assert!(sqlx::query("DELETE FROM pipeline.operation_dag_external_source_attestations_v1 WHERE record_id=$1")
        .bind(record_id).execute(&issuer).await.is_err(), "source evidence cannot be deleted");
    assert!(sqlx::query("INSERT INTO pipeline.operation_dag_external_source_attestations_v1 (record_id) VALUES ($1)")
        .bind(Uuid::new_v4()).execute(&reader).await.is_err(), "Start reader must be SELECT-only");
    assert!(sqlx::query(
        "UPDATE pipeline.operation_dag_external_source_attestations_v1
         SET revoked_at=clock_timestamp() WHERE record_id=$1",
    )
    .bind(record_id).execute(&reader).await.is_err(), "Start reader must not revoke grants");

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE SELECT(id,tenant_id,url,final_url,content_type,status_code,
                       raw_html,raw_html_bytes,content_hash)
             ON raw.pages FROM {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();
    assert!(sqlx::query(insert)
        .bind(Uuid::new_v4()).bind(run_id).bind(&tenant)
        .bind(&context_key).bind(&plan_hash).bind("source_issuer_without_raw_read")
        .bind(&schema).bind(&payload_hash).bind(page_id).bind(page_hash)
        .execute(&issuer).await.is_err(),
        "source issuer must have direct authenticated raw-html read capability");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT(id,tenant_id,url,final_url,content_type,status_code,
                      raw_html,raw_html_bytes,content_hash)
             ON raw.pages TO {issuer_name};"
    )))
    .execute(&pg.pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE pipeline.operation_dag_external_source_attestations_v1
         SET revoked_at=clock_timestamp() WHERE record_id=$1",
    )
    .bind(record_id)
    .execute(&issuer)
    .await
    .expect("only original issuer may perform monotonic revocation");
    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL
         FROM pipeline.operation_dag_external_source_attestations_v1 WHERE record_id=$1",
    )
    .bind(record_id)
    .fetch_one(&reader)
    .await
    .unwrap();
    assert!(revoked);
    assert!(sqlx::query(
        "UPDATE pipeline.operation_dag_external_source_attestations_v1
         SET revoked_at=NULL WHERE record_id=$1",
    )
    .bind(record_id)
    .execute(&issuer)
    .await
    .is_err(), "revocation can never be undone");
}
