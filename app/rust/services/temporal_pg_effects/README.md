# PostgreSQL Truth review owner — experimental V3

This service is a **single real PostgreSQL business effect**, not a provider
facade: it runs the existing `reconcile_truth_candidate_reviews_in_tx` policy
that opens, reopens and supersedes `site.seo_hitl_tasks` for actual
`extracted.rule_candidates`. It NEVER accepts a human decision, labels a claim
verified or publishes. Legacy verified Truth writes continue to invoke the
same function inside their one original canonical transaction.

**Status: experiment / NO-GO for production.** There is no general
`ExecutionPlan`→PG effect scheduling or automatic production caller.
The isolated native operator command below can issue/revoke admissions, but
there is no production credential provisioning, independent tenant authority,
or deployed cross-queue acceptance yet. Do not start on a production
namespace or grant its worker broad application DB credentials.

## Native Temporal contract

| Boundary | Pinned value |
| --- | --- |
| Operation | `pg.truth.reconcile_candidate_reviews@1` |
| Native owner | `postgres_store` |
| Input schema | `truth.candidate_review_scope@1` |
| Output schema | `truth.review_queue_receipt@1` |
| Invalidates | `truth.review_queue_snapshot` |
| Effect | `pg.truth.review_queue.reconcile` |
| Idempotency | `Idempotent`, keyed by workflow/operation/command/admission |
| Retry | SQL/infra transient only, overall Temporal schedule-to-close 300s; rejection/stale/changed snapshot terminal |
| Workflow Type | `PgTruthReviewReconcileV3Workflow` |
| Command Workflow queue | `alegria-pg-command-v3` |
| Activity owner queue | `alegria-pg-effects-v3` (explicit `ActivityOptions.task_queue`) |
| PG authority | `pipeline.pg_effect_v3_admissions`, issued outside Workflow |
| Atomic receipt | `pipeline.pg_effect_v3_receipts`, unique `admission_id`/`execution_key` |

`ReconcileTruthReviewsInput` has `admission_id`, canonical UUID `run_id`,
`scope: {tenant_id, run_id}`, `context_key`, `operation_instance`, `plan_hash`
(64-hex). Workflow History records the entire input; the Activity obtains the
actual `workflow_id`, `workflow_run_id` and Workflow Type from Temporal `ActivityInfo`, not from
caller JSON. A contract-fingerprint is independently pinned in the admission.

## SQL admission and least-privilege setup

Apply `app/db/schema.sql` on a new database or the explicit upgrade
`app/db/migrations/20261124_0003_pg_truth_review_v3.sql` on an existing one.
This is **schema provisioning only**, not a deployment activation.
Migration 0003 has never been released; if an earlier disposable experimental
copy of it was applied, recreate those two experimental V3 tables (after
preserving any data you need) before running the revised schema with exact
`workflow_execution_run_id`. `CREATE TABLE IF NOT EXISTS` is not a migration
for a previously created table with a different column set.

Configure a separately created PostgreSQL LOGIN role, distinct from the
operator/schema owner. Its grants must be limited to:

```sql
GRANT USAGE ON SCHEMA pipeline, extracted, raw, site, verified, system
    TO alegria_pg_effect_worker;
GRANT SELECT ON pipeline.execution_runs, pipeline.step_executions,
    pipeline.pg_effect_v3_admissions TO alegria_pg_effect_worker;
GRANT SELECT, INSERT ON pipeline.pg_effect_v3_receipts TO alegria_pg_effect_worker;
GRANT SELECT ON extracted.rule_candidates, raw.sections, raw.pages
    TO alegria_pg_effect_worker;
GRANT SELECT, INSERT, UPDATE ON site.seo_hitl_tasks TO alegria_pg_effect_worker;
-- No admission INSERT/UPDATE/DELETE/TRUNCATE; no candidate/verified Truth
-- writes; no system.sync_outbox, no CMS human decisions.
```

The Worker performs a **fail-closed effective-permission audit before polling
Temporal and again inside every effect invocation**, including inherited roles
and `PUBLIC` privileges. Its entire
allowed application write surface is `INSERT/UPDATE site.seo_hitl_tasks` and
`INSERT pipeline.pg_effect_v3_receipts`. Even on those tables `DELETE` and
`TRUNCATE` are forbidden; receipt `UPDATE` is forbidden. The audit inspects
**all** application relations, including future migration-added tables and
column-level grants; it also rejects write access to application sequences,
`CREATE` on application schemas, role administration and RLS-bypass rights.
The forbidden set therefore includes, but is not limited to,
`pipeline.pg_effect_v3_admissions`, `pipeline.execution_runs`, raw evidence,
canonical Truth, CMS approval, and `system.sync_outbox`.

The real PostgreSQL negative E2E
`pg_v3_rejects_new_app_writer_and_schema_ddl_privileges` grants and revokes
seven distinct excess capabilities (including column-level UPDATE, operator
admission INSERT, allowed-table DELETE/receipt UPDATE, sequence USAGE and
schema CREATE), requiring startup rejection for each, then recovery after
revocation. This adds defence-in-depth, **not** authority to deploy V3: live
production role provisioning, row-level restriction of shared HITL tasks,
historical cutover and general effectful DAG admission are still open.
`REVOKE FROM PUBLIC` in the schema does not revoke owner privileges or
inherited membership; a separate unprivileged user is mandatory. The
`verified` and `system` schema grants are read-free name resolution only.

An authorized operator issues each admission through their own audited SQL
process or explicit native operator CLI below, **not** an Activity, Workflow,
HTTP request or DAG input. Validate canonical
`pipeline.execution_runs` context/workflow/state first. Derive the exact
SHA-256 source fingerprint from `CANDIDATE_SET_HASH_SQL` in
`postgres_store/src/truth_review_effect_v3.rs`: this covers every scoped
candidate JSONB row and its original raw section hash/bytes. Pin the
`contract_fingerprint()` output, operation revision, effect ID, full run
identity, instance, plan hash, exact Temporal Workflow ID, exact server-created
Temporal Workflow execution Run ID and short expiry. Issue the admission only
AFTER starting the Workflow and receiving its server Run ID, then send the
Temporal Signal `operator_admission_ready`. The Workflow waits durably for
that signal *before* scheduling any Activity; no worker-poller timing race
exists between `start_workflow` and privileged admission issuance. The Signal
is solely a wake-up — it never substitutes for the PG admission check. A
forged/early Signal simply produces a terminal permission rejection. Reusing
a Workflow ID with a new Temporal Run ID cannot consume the former approval.
The Activity denies a missing, forged, expired, revoked, mismatched or stale
admission **before** modifying the HITL queue. Committed exact retries return
the same receipt even after expiry/revocation, without a second effect.

Admission authority is **append-only in PostgreSQL**, not merely an operator
runbook convention. The schema/migration installs an invoker trigger that
rejects changes to the original run/scope/workflow/contract/source/expiry and
all DELETE/TRUNCATE attempts. The sole accepted UPDATE is a monotonic
`revoked_at` transition from NULL to a non-NULL value. Before allowing an
UPDATE, the trigger tries the same `pg-v3-admission|{admission_id}` transaction
advisory lock as the Activity. Revocation committed before the Activity's
authorization read is observed and denied. If the Activity already holds the
lock, revocation **fails immediately** with PostgreSQL `55P03` (lock not
available); the authorized operator must retry after the atomic effect/receipt
transaction completes. This fail-fast behavior is necessary: PostgreSQL can
hold a key-row lock merely because the UPDATE's SET list names a key column,
even if its new value equals the old value; blocking on the advisory lock
could deadlock with the Activity's receipt FK KEY SHARE. Forbidden changes
and deletion are rejected independently. Operator revocation tooling must
check that the UPDATE affected exactly one row, retry only `55P03` with a
bounded backoff and escalate repeated BUSY; do not silently treat BUSY as a
completed revocation. Operators must not bypass triggers, disable them or use
table-owner DDL as an ordinary revoke path.

### Isolated native operator commands (opt-in; not production admission)

Create a dedicated PostgreSQL operator LOGIN with **no role memberships,
SUPERUSER, BYPASSRLS, CREATEDB, CREATEROLE, schema CREATE, sequence mutation
or application writer grants**. Its only allowed application mutation is
`INSERT/UPDATE pipeline.pg_effect_v3_admissions`; it needs SELECT on that
table, `pipeline.execution_runs`, `pipeline.pg_effect_v3_receipts`,
`extracted.rule_candidates`, `raw.sections`, `raw.pages`, and schema USAGE for
those reads. The operator runtime validates effective privileges before
every command, requires `session_user = current_user`, and rejects a
privileged LOGIN using `SET ROLE` to impersonate the restricted operator (it
could otherwise `RESET ROLE` to regain privilege). The role must not be the
Activity Worker or the schema owner.

First start the exact V3 Workflow and obtain the server-issued Temporal
execution Run ID. Keep the Workflow waiting for `operator_admission_ready`.
Prepare a UTF-8 JSON file with only the fields below (example UUIDs/digest):

```json
{
  "command": {
    "admission_id": "11111111-1111-4111-8111-111111111111",
    "run_id": "22222222-2222-4222-8222-222222222222",
    "scope": {
      "tenant_id": "operator-tenant",
      "run_id": "22222222-2222-4222-8222-222222222222"
    },
    "context_key": "approved-truth-context",
    "operation_instance": "review-reconcile",
    "plan_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "workflow_id": "pg-review-workflow-id",
  "workflow_run_id": "33333333-3333-4333-8333-333333333333",
  "expires_in_seconds": 120
}
```

```bash
ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V3=true \
  PG_EFFECT_OPERATOR_DATABASE_URL='postgres://<dedicated-operator-role>' \
  PG_EFFECT_OPERATOR_TENANT_ID='operator-tenant' \
  ./target/debug/temporal_pg_effects --operator-issue /path/to/request.json

ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V3=true \
  PG_EFFECT_OPERATOR_DATABASE_URL='postgres://<dedicated-operator-role>' \
  PG_EFFECT_OPERATOR_TENANT_ID='operator-tenant' \
  ./target/debug/temporal_pg_effects --operator-revoke 11111111-1111-4111-8111-111111111111
```

These commands do not start a Worker, poll a Task Queue or send a Temporal
Signal. On issue, a PostgreSQL transaction takes the canonical context lock,
requires a live matching `pipeline.execution_runs` record, pins the exact
source and operation fingerprints, uses database `current_user` as
`issued_by`, and limits expiry to 1–300 seconds. No caller-supplied
`candidate_set_hash`, fingerprint, issuer or expiry date is accepted.
The operator must independently confirm the Workflow Type/Run ID from
Temporal and send the Signal after successful issue; the CLI checks the
canonical PostgreSQL `workflow_run_id` but does **not** attest Temporal
server identity itself. This is an explicit remaining cutover gate.
The issuer and Activity both lock the full raw source snapshot (extraction and
evidence sections plus both parent pages) in stable key order, then verify
that page/section associations did not move while those locks were acquired.

Revocation is tenant-policy scoped, monotonic and idempotent. It acquires the
same per-admission transaction lock as the Activity **before** checking for
an already committed receipt, and rechecks absence of a receipt in the
conditional UPDATE. The PostgreSQL trigger also serializes
against the effect, retries **only** SQLSTATE `55P03` at most three times
with short bounded backoff, and returns nonzero if admission was missing,
effect already committed, or revocation remained BUSY. The JSON result
reports the actual operator DB role and outcome, never secrets.

The mandatory `integration_pg_v3_e2e --features e2e` shard runs the **actual
native operator binary** against a disposable, migrated PostgreSQL container.
It provisions a distinct restricted LOGIN, checks GRANT denials, wrong
tenant/run/Temporal Run ID, valid issue/revoke/idempotent re-revoke and
the after-commit revocation veto. A separate privileged-session `SET ROLE`
impersonation negative is available in the extra isolated service test.
The mandatory test's synthetic receipt is inserted solely by disposable
fixture-admin rights; it proves the revocation guard, not that an effect ran.
To run the optional deeper service-only test, use a **disposable migrated
database** with explicit CREATE ROLE-capable credentials (append-only
admissions prevent fixture deletion):

```bash
PG_EFFECT_OPERATOR_E2E_ADMIN_DATABASE_URL='postgres://<ephemeral-pg-admin>' \
  cargo test -p temporal_pg_effects --bin temporal_pg_effects \
  live_pg_operator_role_issuance_revocation_and_denials -- --ignored
```

The service-only test is ignored by default. The mandatory container test
does not independently attest a live Temporal Workflow: cross-queue E2E
and historical replay are separate acceptance requirements.

**Tenant limitation:** `PG_EFFECT_OPERATOR_TENANT_ID` is a fail-closed
operator configuration guard, **not row-level security**. Existing
`pipeline.execution_runs`, `raw.sections`, and `site.seo_hitl_tasks` have
no independently authoritative tenant ownership. The #94 architecture must
first decide single-tenant versus multi-tenant data and bind the canonical
run/source/HITL rows to that identity before implementing database RLS.

The canonical PostgreSQL `truth-context|{context_key}` advisory transaction
lock is acquired **before** reading admission or source rows. Under ordinary
READ COMMITTED, this gives a post-lock fresh snapshot; using SERIALIZABLE
here would incorrectly freeze a pre-lock snapshot. The stale-run predicate
matches legacy verified Truth's creation-order policy: a distinct completed
`verified_truth_write` from a run whose `created_at >= this_run.created_at`
blocks the effect, including ties. This is not an independent commit-time
ordering policy. A later-started run can reconcile after an older run commits
if its currently read source fingerprint matches what the operator approved.
Expiry is checked using `clock_timestamp()` at the authorization SELECT
**after both advisory locks**, not transaction-start `now()`: waiting for the
context lock cannot extend an already-expired admission. This establishes
authorization at the post-lock checkpoint, not a promise that the window
remains open through COMMIT; exact committed retries still read the receipt
first and cannot produce a second effect.

## Opt-in execution

```bash
cd app/rust
cargo build --locked -p temporal_pg_effects --bin temporal_pg_effects
ALEGRIA_ENABLE_PG_EFFECT_V3=true \
  PG_EFFECT_WORKER_DATABASE_URL='postgres://<separate-limited-role>' \
  TEMPORAL_URL='http://localhost:7233' \
  ./target/debug/temporal_pg_effects
```

For an audited **real server-generated** JSON Workflow History, replay the
registered V3 Workflow deterministically with the native Temporal SDK:

```bash
./target/debug/temporal_pg_effects --replay-history /path/to/pg-v3-history.json
```

This replay mode is offline, does not require the V3 enable flag, DB
credentials or an active Temporal server, and never connects to an Activity
worker or executes the PostgreSQL effect. The original Activity outcome must
be recorded in History; replay success is **not** a production cutover.

No deployment definition, default CI worker, or legacy `alegria-pipeline`
worker enables it. The pure DAG (`alegria-pure-dag-v3`) remains effect-free.
The only intended Workflow invocation is an explicitly authorized
`PgTruthReviewReconcileV3Workflow` command on its separate queue. Avoid
sharing that Task Queue with any worker lacking the exact registered Type.

## Acceptance gaps

- Live PostgreSQL role/admission/atomic-receipt and cross-queue Temporal E2E
  must pass on an isolated schema + namespace before production consideration.
- Same-version offline SDK History replay is implemented and exercised by the
  new isolated E2E, but **cross-version historical replay** and pinned Worker
  Deployment remain untested; both are required before any cutover.
- A raw evidence/section writer outside the canonical `truth-context` lock
  could change original bytes between separate READ COMMITTED statements.
  Independent concurrent raw-mutation E2E and a shared lock/trigger policy
  across all evidence writers are required for a complete source fence.
- Opt-in local native issuance/revocation commands are available with a
  mandatory disposable PostgreSQL CLI E2E, but production role provisioning,
  Temporal server Describe
  verification, tenant ownership/RLS, audit and rate limits require review.
  One canonical `pipeline.execution_runs` stores only one Temporal execution
  Run ID. Multiple independent effect child Workflows require a new per-node
  parent/child provenance and admission schema, **not** repeated overwrite of
  that single field or a broadening of the existing V3 Activity queue.
- Existing `TruthHitlReconcile` CLI can still invoke canonical PG review
  reconciliation directly under its legacy privilege model. This remains a
  migration consideration, not an alternate V3 effect authorization path.
- No Graph/Qdrant SDK is linked, and no Outbox dispatcher is started here.
