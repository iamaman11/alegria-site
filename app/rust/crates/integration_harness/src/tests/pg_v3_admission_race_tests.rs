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
        contract_fingerprint, CANDIDATE_SET_HASH_SQL,
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
          contract_fingerprint, candidate_set_hash, issued_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
                 'ci_admission_race_operator', clock_timestamp() + $11::interval)",
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
    .bind(ttl)
    .execute(pool)
    .await
    .expect("issue short-lived immutable admission from operator connection");
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
