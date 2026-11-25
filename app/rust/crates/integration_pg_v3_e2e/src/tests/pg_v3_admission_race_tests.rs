// Real PostgreSQL concurrency acceptance for append-only V3 operator admission.
// These are deliberately server-side races, not mocked executor decisions.

#[cfg(feature = "e2e")]
async fn pg_v3_wait_for_lock(
    pool: &sqlx::PgPool,
    user: &str,
    event: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_stat_activity
                WHERE usename = $1 AND state = 'active'
                  AND wait_event_type = 'Lock' AND wait_event = $2
            )",
        )
        .bind(user)
        .bind(event)
        .fetch_one(pool)
        .await
        .expect("inspect actual PostgreSQL backend blocking state");
        if waiting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "PostgreSQL backend {user} never reached expected {event} lock wait"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(feature = "e2e")]
async fn pg_v3_wait_until_expired(pool: &sqlx::PgPool, admission_id: Uuid) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let expired: bool = sqlx::query_scalar(
            "SELECT clock_timestamp() > expires_at
             FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1",
        )
        .bind(admission_id)
        .fetch_one(pool)
        .await
        .expect("read operator-issued immutable expiration");
        if expired {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "bounded V3 admission did not reach its PostgreSQL expiry"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(feature = "e2e")]
async fn pg_v3_issue_short_admission(
    pool: &sqlx::PgPool,
    fixture: &PgV3ReviewFixture,
    ttl: &str,
) {
    use postgres_store::truth_review_effect_v3::{
        contract_fingerprint, execution_key, CANDIDATE_SET_HASH_SQL,
    };

    let candidate_hash: String = sqlx::query_scalar(CANDIDATE_SET_HASH_SQL)
        .bind(&fixture.command.context_key)
        .fetch_one(pool)
        .await
        .expect("privileged operator computes exact canonical candidate hash");
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, run_id, tenant_id, context_key, workflow_id,
          workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, v3_expected_execution_key,
          issued_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                 'ci_admission_race_operator', clock_timestamp() + $12::interval)",
    )
    .bind(fixture.command.admission_id)
    .bind(fixture.command.run_id)
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(&fixture.workflow_id)
    .bind(&fixture.workflow_run_id)
    .bind(&fixture.command.operation_instance)
    .bind(&fixture.command.plan_hash)
    .bind(contract_fingerprint().expect("pin typed V3 owner contract"))
    .bind(candidate_hash)
    .bind(execution_key(&fixture.workflow_id, &fixture.workflow_run_id, &fixture.command)
        .expect("pin canonical V3 execution key"))
    .bind(ttl)
    .execute(pool)
    .await
    .expect("issue short-lived immutable admission from operator connection");
}

#[cfg(feature = "e2e")]
async fn pg_v3_seed_newer_verified_truth_step(
    pool: &sqlx::PgPool,
    fixture: &PgV3ReviewFixture,
) -> i64 {
    let newer_run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
         (run_id, workflow_run_id, workflow_type, context_key, status, created_at)
         VALUES ($1, $2, 'seo_site_build', $3, 'verifying',
                 clock_timestamp() + interval '1 second')",
    )
    .bind(newer_run_id)
    .bind(Uuid::new_v4().to_string())
    .bind(&fixture.command.context_key)
    .execute(pool)
    .await
    .expect("seed newer canonical Truth run in same context");
    sqlx::query_scalar(
        "INSERT INTO pipeline.step_executions
         (run_id, step_name, schema_version, input_hash, idempotency_key, status)
         VALUES ($1, 'verified_truth_write', 1, 'source-pin', $2, 'running')
         RETURNING step_execution_id",
    )
    .bind(newer_run_id)
    .bind(Uuid::new_v4().to_string())
    .fetch_one(pool)
    .await
    .expect("seed not-yet-done Truth step whose terminal transition will race")
}

#[cfg(feature = "e2e")]
fn pg_v3_spawn_effect(
    pool: &sqlx::PgPool,
    fixture: &PgV3ReviewFixture,
) -> tokio::task::JoinHandle<
    Result<
        postgres_store::truth_review_effect_v3::ReconcileTruthReviewsReceipt,
        postgres_store::truth_review_effect_v3::ReconcileTruthReviewsError,
    >,
> {
    use postgres_store::truth_review_effect_v3::execute_admitted_reconcile;

    let pool = pool.clone();
    let workflow_id = fixture.workflow_id.clone();
    let workflow_run_id = fixture.workflow_run_id.clone();
    let command = fixture.command.clone();
    tokio::spawn(async move {
        execute_admitted_reconcile(&pool, &workflow_id, &workflow_run_id, &command).await
    })
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_admission_expiry_and_revocation_serialize_with_effect_commit() {
    use postgres_store::truth_review_effect_v3::{
        execute_admitted_reconcile, ReconcileTruthReviewsError,
    };

    let postgres = PostgresHarness::start()
        .await
        .expect("start disposable real PostgreSQL for admission races");
    let effect_pool = pg_v3_unprivileged_effect_pool(&postgres).await;
    let effect_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&effect_pool)
        .await
        .expect("identify isolated least-privileged Activity role");

    // Hold the same context lock as canonical verified Truth while an Activity
    // starts its DB transaction. The operator issues a short admission AFTER
    // the Activity begins waiting, so transaction-start now() would be a
    // false-positive approval once the actual expiration passes.
    let expired = pg_v3_seed_review_fixture(&postgres.pool).await;
    let mut context_tx = postgres.pool.begin().await.expect("hold Truth context lock");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("truth-context|{}", expired.command.context_key))
        .execute(&mut *context_tx)
        .await
        .expect("hold canonical Truth context lock");
    let waiting_effect = pg_v3_spawn_effect(&effect_pool, &expired);
    pg_v3_wait_for_lock(&postgres.pool, &effect_role, "advisory").await;
    pg_v3_issue_short_admission(&postgres.pool, &expired, "250 milliseconds").await;
    pg_v3_wait_until_expired(&postgres.pool, expired.command.admission_id).await;
    context_tx.commit().await.expect("release context lock after expiry");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), waiting_effect)
            .await
            .expect("blocked Activity completes after context unlock")
            .expect("Activity task did not panic"),
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &expired).await;
    eprintln!("CI_PG_V3_EXPIRED_WHILE_LOCKED=PASS");

    // A real committed receipt survives natural expiry without changing the
    // authority row. A replay must not issue another HITL mutation.
    let replay = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_short_admission(&postgres.pool, &replay, "5 seconds").await;
    let first = execute_admitted_reconcile(
        &effect_pool,
        &replay.workflow_id,
        &replay.workflow_run_id,
        &replay.command,
    )
    .await
    .expect("execute once within immutable short operator window");
    pg_v3_wait_until_expired(&postgres.pool, replay.command.admission_id).await;
    let again = execute_admitted_reconcile(
        &effect_pool,
        &replay.workflow_id,
        &replay.workflow_run_id,
        &replay.command,
    )
    .await
    .expect("return committed receipt after natural admission expiry");
    assert_eq!(first, again);
    eprintln!("CI_PG_V3_REPLAY_AFTER_NATURAL_EXPIRY=PASS");

    // Revocation wins the admission lock first. The waiting Activity cannot
    // observe the old authorized value after revocation commits.
    let deny = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &deny).await;
    let mut revoke_tx = postgres.pool.begin().await.expect("start operator revoke tx");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("pg-v3-admission|{}", deny.command.admission_id))
        .execute(&mut *revoke_tx)
        .await
        .expect("operator holds admission lock first");
    sqlx::query(
        "UPDATE pipeline.pg_effect_v3_admissions SET revoked_at = clock_timestamp()
         WHERE admission_id = $1",
    )
    .bind(deny.command.admission_id)
    .execute(&mut *revoke_tx)
    .await
    .expect("trigger serializes operator revocation");
    let denied_effect = pg_v3_spawn_effect(&effect_pool, &deny);
    pg_v3_wait_for_lock(&postgres.pool, &effect_role, "advisory").await;
    revoke_tx.commit().await.expect("commit operator revocation first");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), denied_effect)
            .await
            .expect("Activity finishes after operator revoke")
            .expect("Activity task did not panic"),
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &deny).await;
    eprintln!("CI_PG_V3_REVOKE_FIRST=PASS");

    // Activity wins first. A third session blocks only its actual HITL SQL,
    // giving the operator time to attempt a concurrent revoke. PostgreSQL
    // must keep the revoke waiting on the admission advisory lock until the
    // effect+receipt transaction commits. No fabricated race in Rust mocks.
    let allow = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &allow).await;
    let mut blocker_tx = postgres.pool.begin().await.expect("start HITL blocker");
    sqlx::query("LOCK TABLE site.seo_hitl_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker_tx)
        .await
        .expect("hold real review task table lock");
    let admitted_effect = pg_v3_spawn_effect(&effect_pool, &allow);
    pg_v3_wait_for_lock(&postgres.pool, &effect_role, "relation").await;

    // A prohibited PRIMARY KEY rewrite would obtain an incompatible row
    // FOR UPDATE lock. It MUST fail immediately before attempting the
    // advisory lock, or the Activity's eventual receipt FK KEY SHARE could
    // form a real deadlock cycle with this operator transaction.
    let bad_key_change = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query(
            "UPDATE pipeline.pg_effect_v3_admissions
             SET admission_id = gen_random_uuid() WHERE admission_id = $1",
        )
        .bind(allow.command.admission_id)
        .execute(&postgres.pool),
    )
    .await
    .expect("forbidden key mutation blocked instead of failing immediately")
    .expect_err("PostgreSQL accepted a mutable admission primary key");
    assert_eq!(
        bad_key_change
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514"),
        "expected explicit admission immutability failure"
    );
    // PostgreSQL chooses FOR UPDATE even if the key is assigned to itself.
    // A blocking trigger could deadlock with the Activity receipt FK. The
    // advisory try-lock must fail fast with 55P03 on this actual SQL shape.
    let noop_key_change = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query(
            "UPDATE pipeline.pg_effect_v3_admissions
             SET admission_id = admission_id WHERE admission_id = $1",
        )
        .bind(allow.command.admission_id)
        .execute(&postgres.pool),
    )
    .await
    .expect("no-op key UPDATE hung on advisory instead of refusing busy")
    .expect_err("no-op key UPDATE bypassed busy fail-closed fence");
    assert_eq!(
        noop_key_change
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03"),
        "expected typed busy denial for key-row lock inversion attempt"
    );

    let operator_pool = postgres.pool.clone();
    let admission_id = allow.command.admission_id;
    let revoke_after_effect = tokio::spawn(async move {
        sqlx::query(
            "UPDATE pipeline.pg_effect_v3_admissions /* pg_v3_revocation_race */
             SET revoked_at = clock_timestamp() WHERE admission_id = $1",
        )
        .bind(admission_id)
        .execute(&operator_pool)
        .await
    });
    let busy_revoke = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        revoke_after_effect,
    )
    .await
    .expect("concurrent operator revocation waited instead of refusing busy")
    .expect("operator revocation task did not panic")
    .expect_err("operator revocation ignored an in-flight Activity advisory lock");
    assert_eq!(
        busy_revoke
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03"),
        "in-flight admission revocation must return typed 55P03"
    );
    blocker_tx.commit().await.expect("release HITL SQL blocker");
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(10), admitted_effect)
        .await
        .expect("first Activity commits effect+receipt after unblock")
        .expect("Activity task did not panic")
        .expect("admission won serialization before operator revoke");
    let changed = sqlx::query(
        "UPDATE pipeline.pg_effect_v3_admissions SET revoked_at = clock_timestamp()
         WHERE admission_id = $1",
    )
    .bind(allow.command.admission_id)
    .execute(&postgres.pool)
    .await
    .expect("operator retries busy revocation after Activity receipt COMMIT");
    assert_eq!(changed.rows_affected(), 1);
    let repeat = execute_admitted_reconcile(
        &effect_pool,
        &allow.workflow_id,
        &allow.workflow_run_id,
        &allow.command,
    )
    .await
    .expect("receipt replay after serialized revocation");
    assert_eq!(receipt, repeat);
    eprintln!("CI_PG_V3_EFFECT_FIRST=PASS");

    // Operator cannot rewrite original identity/contract/expiry, erase
    // already committed revocation, delete authority, or TRUNCATE the ledger.
    for mutation in [
        "UPDATE pipeline.pg_effect_v3_admissions SET plan_hash = repeat('b', 64)
         WHERE issued_by = 'ci_authorized_operator'",
        "UPDATE pipeline.pg_effect_v3_admissions SET expires_at = clock_timestamp()
         WHERE issued_by = 'ci_authorized_operator'",
        "UPDATE pipeline.pg_effect_v3_admissions SET revoked_at = NULL
         WHERE issued_by = 'ci_authorized_operator' AND revoked_at IS NOT NULL",
        "TRUNCATE pipeline.pg_effect_v3_admissions CASCADE",
    ] {
        assert!(
            sqlx::query(mutation).execute(&postgres.pool).await.is_err(),
            "PostgreSQL accepted forbidden admission authority mutation: {mutation}"
        );
    }
    // No receipt or referencing row exists for this admission: DELETE would
    // succeed without the mutation trigger, so this asserts the DB fence,
    // not an incidental foreign-key failure on a completed effect.
    assert!(
        sqlx::query("DELETE FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1")
            .bind(deny.command.admission_id)
            .execute(&postgres.pool)
            .await
            .is_err(),
        "an unconsumed operator admission was deletable"
    );
    eprintln!("CI_PG_V3_ADMISSION_IMMUTABLE=PASS");
}

/// Two actual database connections prove that direct raw source writers and
/// an admitted Activity cannot race between source-hash validation and the
/// final HITL + receipt COMMIT. The Activity keeps SELECT-only raw rights.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_raw_source_concurrent_update_is_fenced() {
    use postgres_store::truth_review_effect_v3::{
        execute_admitted_reconcile, ReconcileTruthReviewsError,
    };

    let postgres = PostgresHarness::start()
        .await
        .expect("disposable PostgreSQL raw writer race");
    let (effect_pool, _) = pg_v3_unprivileged_effect_identity(&postgres).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&effect_pool)
        .await
        .expect("effect role");

    // Writer commits before the effect can acquire its source row key.
    // The Activity waits, hashes the new committed bytes, and refuses the
    // earlier operator admission without creating a task or receipt.
    let stale = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &stale).await;
    let section: i64 = sqlx::query_scalar(
        "SELECT raw_section_id FROM extracted.rule_candidates WHERE rule_candidate_id = $1",
    )
    .bind(&stale.candidate_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("section id");
    let mut writer = postgres.pool.begin().await.expect("raw editor tx");
    sqlx::query(
        "UPDATE raw.sections SET content_md = 'source changed during admission' WHERE id = $1",
    )
    .bind(section)
    .execute(&mut *writer)
    .await
    .expect("writer locks source first");
    let pending = pg_v3_spawn_effect(&effect_pool, &stale);
    pg_v3_wait_for_lock(&postgres.pool, &role, "advisory").await;
    writer.commit().await.expect("writer commits first");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), pending)
            .await
            .expect("Activity unlocks")
            .expect("Activity did not panic"),
        Err(ReconcileTruthReviewsError::CandidateSetChanged)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &stale).await;
    eprintln!("CI_PG_V3_RAW_SOURCE_WRITER_FIRST=PASS");

    // The page bytes are also included in the operator's snapshot, not only
    // the candidate and section data. A pre-effect page rewrite is stale.
    let page_stale = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &page_stale).await;
    let page: i64 = sqlx::query_scalar(
        "SELECT s.page_id FROM raw.sections s
         JOIN extracted.rule_candidates c ON c.raw_section_id = s.id
         WHERE c.rule_candidate_id = $1",
    )
    .bind(&page_stale.candidate_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("page id");
    sqlx::query("UPDATE raw.pages SET raw_html = '<p>changed</p>' WHERE id = $1")
        .bind(page)
        .execute(&postgres.pool)
        .await
        .expect("direct raw page writer");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &page_stale.workflow_id,
            &page_stale.workflow_run_id,
            &page_stale.command
        )
        .await,
        Err(ReconcileTruthReviewsError::CandidateSetChanged)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &page_stale).await;
    eprintln!("CI_PG_V3_RAW_PAGE_FINGERPRINT=PASS");

    // Structured crawler content and source observation are independent of
    // raw_html; mutations of either must invalidate the operator's hash.
    for (source_update, case_name) in [
        (
            "UPDATE raw.pages SET content = '{\"markdown\":\"changed without HTML rewrite\"}'::jsonb WHERE id = $1",
            "structured page content",
        ),
        (
            "UPDATE raw.pages SET source_observation = '{\"robots\":\"changed\"}'::jsonb WHERE id = $1",
            "source observation",
        ),
        (
            "UPDATE raw.pages SET final_url = 'https://example.org/other-final' WHERE id = $1",
            "final source URL",
        ),
    ] {
        let changed = pg_v3_seed_review_fixture(&postgres.pool).await;
        pg_v3_issue_operator_admission(&postgres.pool, &changed).await;
        let page_id: i64 = sqlx::query_scalar(
            "SELECT s.page_id FROM raw.sections s
             JOIN extracted.rule_candidates c ON c.raw_section_id = s.id
             WHERE c.rule_candidate_id = $1",
        )
        .bind(&changed.candidate_id)
        .fetch_one(&postgres.pool)
        .await
        .expect("find separate original source page");
        sqlx::query(source_update)
            .bind(page_id)
            .execute(&postgres.pool)
            .await
            .expect("writer changes only one provenance field");
        assert!(
            matches!(
                execute_admitted_reconcile(
                    &effect_pool,
                    &changed.workflow_id,
                    &changed.workflow_run_id,
                    &changed.command,
                )
                .await,
                Err(ReconcileTruthReviewsError::CandidateSetChanged)
            ),
            "{case_name} was omitted from admitted source fingerprint"
        );
        pg_v3_assert_no_effect(&postgres.pool, &changed).await;
    }
    eprintln!("CI_PG_V3_JSONB_OBSERVATION_FINAL_URL_PINNED=PASS");

    // A distinct `evidence_section_id` can legitimately point to a section
    // other than `raw_section_id`. Both sections and their parent pages must
    // be pinned; changing evidence-only markdown is a stale admission.
    let evidence_only = pg_v3_seed_review_fixture(&postgres.pool).await;
    let separate_page: i64 = sqlx::query_scalar(
        "SELECT s.page_id FROM raw.sections s
         JOIN extracted.rule_candidates c ON c.raw_section_id = s.id
         WHERE c.rule_candidate_id = $1",
    )
    .bind(&evidence_only.candidate_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("original evidence page");
    let separate_evidence_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.sections (page_id, content_md, content_hash)
         VALUES ($1, 'separate evidence', 'evidence-original') RETURNING id",
    )
    .bind(separate_page)
    .fetch_one(&postgres.pool)
    .await
    .expect("persist independent evidence section");
    sqlx::query(
        "UPDATE extracted.rule_candidates SET evidence_section_id = $1
         WHERE rule_candidate_id = $2",
    )
    .bind(separate_evidence_id)
    .bind(&evidence_only.candidate_id)
    .execute(&postgres.pool)
    .await
    .expect("bind separate evidence before admission");
    pg_v3_issue_operator_admission(&postgres.pool, &evidence_only).await;
    sqlx::query("UPDATE raw.sections SET content_md = 'forged evidence' WHERE id = $1")
        .bind(separate_evidence_id)
        .execute(&postgres.pool)
        .await
        .expect("modify only evidence section after admission");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &evidence_only.workflow_id,
            &evidence_only.workflow_run_id,
            &evidence_only.command,
        )
        .await,
        Err(ReconcileTruthReviewsError::CandidateSetChanged)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &evidence_only).await;
    eprintln!("CI_PG_V3_INDEPENDENT_EVIDENCE_SECTION_PINNED=PASS");

    // Two real connections expose the READ COMMITTED parent-relink race:
    // while V3 reads (section, old_page) and waits on old_page's writer key,
    // the writer commits section.page_id -> new_page. V3 must *not* hash the
    // unlocked new page and create a review task based on stale admission.
    let reparent = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &reparent).await;
    let (reparent_section, original_page): (i64, i64) = sqlx::query_as(
        "SELECT s.id, s.page_id FROM raw.sections s
         JOIN extracted.rule_candidates c ON c.raw_section_id = s.id
         WHERE c.rule_candidate_id = $1",
    )
    .bind(&reparent.candidate_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("original section parent");
    let new_page: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages (url, domain, dtype, content)
         VALUES ('https://example.org/reparent', 'example.org', 'html', '{}'::jsonb)
         RETURNING id",
    )
    .fetch_one(&postgres.pool)
    .await
    .expect("new page has a distinct source key");
    let mut reparent_writer = postgres.pool.begin().await.expect("reparent writer tx");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("raw-evidence|pages|{original_page}"))
        .execute(&mut *reparent_writer)
        .await
        .expect("hold initial parent key while racing V3 snapshot");
    sqlx::query("UPDATE raw.sections SET page_id = $1 WHERE id = $2")
        .bind(new_page)
        .bind(reparent_section)
        .execute(&mut *reparent_writer)
        .await
        .expect("uncommitted reparent invisible to V3 initial SELECT");
    let reparent_effect = pg_v3_spawn_effect(&effect_pool, &reparent);
    pg_v3_wait_for_lock(&postgres.pool, &role, "advisory").await;
    reparent_writer
        .commit()
        .await
        .expect("commit reparent while V3 waits on old page");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), reparent_effect)
            .await
            .expect("V3 handles reparent unlock")
            .expect("V3 reparent task did not panic"),
        Err(ReconcileTruthReviewsError::CandidateSetChanged)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &reparent).await;
    eprintln!("CI_PG_V3_CONCURRENT_SECTION_REPARENT=PASS");

    // Admission does not exist *yet*. The terminal writer has already
    // acquired its execution_runs row lock and the truth-run-status key.
    // Operator issuance takes the SAME key before reading canonical status;
    // it must wait for the committed terminal state instead of approving the
    // stale pre-update value under READ COMMITTED.
    let pre_admission = pg_v3_seed_review_fixture(&postgres.pool).await;
    let mut early_writer = postgres.pool.begin().await.expect("early status writer tx");
    sqlx::query("UPDATE pipeline.execution_runs SET status = 'failed' WHERE run_id = $1")
        .bind(pre_admission.command.run_id)
        .execute(&mut *early_writer)
        .await
        .expect("status writer acquires run-key without an existing admission");
    let early_operator_pool = postgres.pool.clone();
    let early_context = pre_admission.command.context_key.clone();
    let early_run_id = pre_admission.command.run_id;
    let waiting_operator = tokio::spawn(async move {
        let mut tx = early_operator_pool.begin().await.expect("operator issue tx");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("truth-context|{early_context}"))
            .execute(&mut *tx)
            .await
            .expect("operator takes context first");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("truth-run-status|{early_run_id}"))
            .execute(&mut *tx)
            .await
            .expect("operator waits on exactly the status-writer run-key");
        let status: String = sqlx::query_scalar(
            "SELECT status FROM pipeline.execution_runs WHERE run_id = $1",
        )
        .bind(early_run_id)
        .fetch_one(&mut *tx)
        .await
        .expect("authoritative status read AFTER run-key lock");
        tx.commit().await.expect("release admission preflight locks");
        status
    });
    pg_v3_wait_for_lock(&postgres.pool, "postgres", "advisory").await;
    early_writer
        .commit()
        .await
        .expect("terminal writer commits before operator status preflight");
    let issue_status = tokio::time::timeout(
        std::time::Duration::from_secs(10), waiting_operator,
    )
    .await
    .expect("operator preflight resumes")
    .expect("operator issue task did not panic");
    assert_eq!(issue_status, "failed", "operator must deny stale terminal run");
    pg_v3_assert_no_effect(&postgres.pool, &pre_admission).await;
    eprintln!("CI_PG_V3_PRE_ADMISSION_TERMINAL_WRITER_FIRST=PASS");

    // Operator wins that same admission preflight key first. Even without a
    // committed admission row, a terminal update must fail fast and release
    // its row lock (otherwise the subsequent operator FK could deadlock).
    let pre_admission_operator_first = pg_v3_seed_review_fixture(&postgres.pool).await;
    let mut issuing = postgres.pool.begin().await.expect("operator-first issue tx");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!(
            "truth-context|{}",
            pre_admission_operator_first.command.context_key
        ))
        .execute(&mut *issuing)
        .await
        .expect("hold operator's Truth context lock");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!(
            "truth-run-status|{}",
            pre_admission_operator_first.command.run_id
        ))
        .execute(&mut *issuing)
        .await
        .expect("hold operator's exact run-key");
    let rejected_before_admission = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("UPDATE pipeline.execution_runs SET status = 'failed' WHERE run_id = $1")
            .bind(pre_admission_operator_first.command.run_id)
            .execute(&postgres.pool),
    )
    .await
    .expect("status update waited on operator-first run-key")
    .expect_err("status writer bypassed uncommitted issuance run-key");
    assert_eq!(
        rejected_before_admission
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    issuing.commit().await.expect("release operator-first preflight");
    pg_v3_assert_no_effect(&postgres.pool, &pre_admission_operator_first).await;
    eprintln!("CI_PG_V3_PRE_ADMISSION_OPERATOR_FIRST=PASS");

    // Canonical run status is authoritative admission state. A terminal
    // writer that acquires truth-context first must make the waiting effect
    // reject, even when the unchanged candidate snapshot still matches.
    let terminal_first = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &terminal_first).await;
    let mut terminal_writer = postgres.pool.begin().await.expect("terminal writer tx");
    sqlx::query("UPDATE pipeline.execution_runs SET status = 'failed' WHERE run_id = $1")
        .bind(terminal_first.command.run_id)
        .execute(&mut *terminal_writer)
        .await
        .expect("terminal transition holds Truth context key");
    let terminal_denied = pg_v3_spawn_effect(&effect_pool, &terminal_first);
    pg_v3_wait_for_lock(&postgres.pool, &role, "advisory").await;
    terminal_writer
        .commit()
        .await
        .expect("terminal state commits before Activity readback");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), terminal_denied)
            .await
            .expect("Activity wakes after terminal transition")
            .expect("Activity terminal-race task did not panic"),
        Err(ReconcileTruthReviewsError::StaleTruthRun)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &terminal_first).await;
    eprintln!("CI_PG_V3_TERMINAL_STATUS_WRITER_FIRST=PASS");

    // In the opposite order, an admitted Activity holds truth-context while
    // its real HITL write is blocked. A conflicting terminal update has an
    // execution_runs row lock before its BEFORE UPDATE trigger, so the DB
    // must fail fast with 55P03 rather than risk receipt-FK deadlock.
    let terminal_effect_first = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &terminal_effect_first).await;
    let mut terminal_hitl_blocker = postgres.pool.begin().await.expect("terminal HITL blocker");
    sqlx::query("LOCK TABLE site.seo_hitl_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *terminal_hitl_blocker)
        .await
        .expect("block terminal race Activity after status check");
    let terminal_allowed = pg_v3_spawn_effect(&effect_pool, &terminal_effect_first);
    pg_v3_wait_for_lock(&postgres.pool, &role, "relation").await;
    let refused_terminal = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("UPDATE pipeline.execution_runs SET status = 'done' WHERE run_id = $1")
            .bind(terminal_effect_first.command.run_id)
            .execute(&postgres.pool),
    )
    .await
    .expect("terminal status writer hung on an in-flight admitted effect")
    .expect_err("terminal writer bypassed admitted Truth run-status lock");
    assert_eq!(
        refused_terminal
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    // Operational timestamp updates carry no business state changes and
    // should not be blocked by the narrow execution-run status fence.
    let timestamp_progress = sqlx::query(
        "UPDATE pipeline.execution_runs SET updated_at = clock_timestamp() WHERE run_id = $1",
    )
    .bind(terminal_effect_first.command.run_id)
    .execute(&postgres.pool)
    .await
    .expect("non-identity timestamp progress stays legal");
    assert_eq!(timestamp_progress.rows_affected(), 1);
    terminal_hitl_blocker
        .commit()
        .await
        .expect("release terminal race HITL blocker");
    let terminal_receipt = tokio::time::timeout(
        std::time::Duration::from_secs(10), terminal_allowed,
    )
    .await
    .expect("Activity completes after rejected terminal mutation")
    .expect("Activity terminal-race task did not panic")
    .expect("admitted Truth effect commits with stable canonical run");
    assert_eq!(terminal_receipt.run_id, terminal_effect_first.command.run_id);
    let terminal_status: String = sqlx::query_scalar(
        "SELECT status FROM pipeline.execution_runs WHERE run_id = $1",
    )
    .bind(terminal_effect_first.command.run_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("status remained nonterminal through receipt");
    assert_eq!(terminal_status, "verifying");
    eprintln!("CI_PG_V3_TERMINAL_STATUS_EFFECT_FIRST=PASS");

    // Chronology TOCTOU independent of execution_runs.status: a newer run's
    // `verified_truth_write` may flip from running to done after V3 checked
    // chronology, unless the STEP writer uses the same truth-context key.
    let step_writer_first = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &step_writer_first).await;
    let newer_step = pg_v3_seed_newer_verified_truth_step(
        &postgres.pool, &step_writer_first,
    ).await;
    let mut step_writer = postgres.pool.begin().await.expect("newer Truth step writer tx");
    sqlx::query("UPDATE pipeline.step_executions SET status = 'done' WHERE step_execution_id = $1")
        .bind(newer_step)
        .execute(&mut *step_writer)
        .await
        .expect("newer completed step locks Truth context before effect");
    let step_denied = pg_v3_spawn_effect(&effect_pool, &step_writer_first);
    pg_v3_wait_for_lock(&postgres.pool, &role, "advisory").await;
    step_writer.commit().await.expect("commit newer verified Truth first");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), step_denied)
            .await
            .expect("Activity wakes after completed newer step")
            .expect("Activity task did not panic"),
        Err(ReconcileTruthReviewsError::StaleTruthRun)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &step_writer_first).await;
    eprintln!("CI_PG_V3_VERIFIED_STEP_WRITER_FIRST=PASS");

    let step_effect_first = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &step_effect_first).await;
    let competing_step = pg_v3_seed_newer_verified_truth_step(
        &postgres.pool, &step_effect_first,
    ).await;
    let mut step_hitl_blocker = postgres.pool.begin().await.expect("step HITL blocker");
    sqlx::query("LOCK TABLE site.seo_hitl_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *step_hitl_blocker)
        .await
        .expect("block in-flight effect after verified-step chronology read");
    let step_admitted = pg_v3_spawn_effect(&effect_pool, &step_effect_first);
    pg_v3_wait_for_lock(&postgres.pool, &role, "relation").await;
    let step_mutation_err = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("UPDATE pipeline.step_executions SET status = 'done' WHERE step_execution_id = $1")
            .bind(competing_step)
            .execute(&postgres.pool),
    )
    .await
    .expect("step writer waited instead of fail-fast context denial")
    .expect_err("newer verified Truth changed while admitted effect was committing");
    assert_eq!(
        step_mutation_err
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    step_hitl_blocker.commit().await.expect("release step HITL blocker");
    tokio::time::timeout(std::time::Duration::from_secs(10), step_admitted)
        .await
        .expect("effect proceeds with stable newer-step chronology")
        .expect("effect task did not panic")
        .expect("effect safely committed before competing verified step");
    let still_running: String = sqlx::query_scalar(
        "SELECT status FROM pipeline.step_executions WHERE step_execution_id = $1",
    )
    .bind(competing_step)
    .fetch_one(&postgres.pool)
    .await
    .expect("rejected newer step remains running");
    assert_eq!(still_running, "running");
    eprintln!("CI_PG_V3_VERIFIED_STEP_EFFECT_FIRST=PASS");

    // Activity commits first: pause only the actual HITL table so its source
    // locks are known to be held, then attempt a conflicting raw page update
    // from another connection. The trigger must fail fast with 55P03.
    let accepted = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &accepted).await;
    let accepted_page: i64 = sqlx::query_scalar(
        "SELECT s.page_id FROM raw.sections s
         JOIN extracted.rule_candidates c ON c.raw_section_id = s.id
         WHERE c.rule_candidate_id = $1",
    )
    .bind(&accepted.candidate_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("accepted page id");
    let mut blocker = postgres.pool.begin().await.expect("HITL blocker");
    sqlx::query("LOCK TABLE site.seo_hitl_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("block real HITL write after source validation");
    let allowed = pg_v3_spawn_effect(&effect_pool, &accepted);
    pg_v3_wait_for_lock(&postgres.pool, &role, "relation").await;
    // Crawler bookkeeping is intentionally not Truth source evidence. A
    // processed-only UPDATE must remain available during source admission.
    let operational_progress = sqlx::query(
        "UPDATE raw.pages SET processed = true WHERE id = $1",
    )
    .bind(accepted_page)
    .execute(&postgres.pool)
    .await
    .expect("processed-only page progress is not source mutation");
    assert_eq!(operational_progress.rows_affected(), 1);
    let rejected = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("UPDATE raw.pages SET raw_html = '<p>forged</p>' WHERE id = $1")
            .bind(accepted_page)
            .execute(&postgres.pool),
    )
    .await
    .expect("raw update blocked rather than immediate failure")
    .expect_err("raw source rewrite slipped past admitted Activity");
    assert_eq!(
        rejected
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    let rejected_delete = sqlx::query("DELETE FROM raw.sections WHERE page_id = $1")
        .bind(accepted_page)
        .execute(&postgres.pool)
        .await
        .expect_err("raw evidence section deleted during admitted Activity");
    assert_eq!(
        rejected_delete
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    blocker.commit().await.expect("release HITL blocker");
    tokio::time::timeout(std::time::Duration::from_secs(10), allowed)
        .await
        .expect("Activity completes")
        .expect("Activity did not panic")
        .expect("admitted effect commits once");
    let stored: Option<String> =
        sqlx::query_scalar("SELECT raw_html FROM raw.pages WHERE id = $1")
            .bind(accepted_page)
            .fetch_one(&postgres.pool)
            .await
            .expect("read source bytes after receipt");
    assert!(stored.is_none(), "forged HTML did not remain uncommitted");
    eprintln!("CI_PG_V3_ADMITTED_SOURCE_IMMUTABLE=PASS");
}
