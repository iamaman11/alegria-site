# PostgreSQL Truth review owner — experimental V3

This service is a **single real PostgreSQL business effect**, not a provider
facade: it runs the existing `reconcile_truth_candidate_reviews_in_tx` policy
that opens, reopens and supersedes `site.seo_hitl_tasks` for actual
`extracted.rule_candidates`. It NEVER accepts a human decision, labels a claim
verified or publishes. Legacy verified Truth writes continue to invoke the
same function inside their one original canonical transaction.

**Status: experiment / NO-GO for production.** There is no general
`ExecutionPlan`→PG effect scheduling, no delegated operator-admission issuer,
no automatic caller of this Workflow, no production credential provisioning,
and no deployed cross-queue acceptance yet. Do not start on a production
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
GRANT SELECT ON extracted.rule_candidates, raw.sections TO alegria_pg_effect_worker;
GRANT SELECT, INSERT, UPDATE ON site.seo_hitl_tasks TO alegria_pg_effect_worker;
-- No admission INSERT/UPDATE/DELETE/TRUNCATE; no candidate/verified Truth
-- writes; no system.sync_outbox, no CMS human decisions.
```

The worker fails startup if the *effective* role has admission table or
column-level INSERT/UPDATE, DELETE or TRUNCATE rights; it also refuses write
access to canonical `verified.rule_instances`, `extracted.rule_candidates`,
`system.sync_outbox`, and `site.cms_approval_decisions`. `REVOKE FROM PUBLIC`
in the schema does not revoke owner privileges or inherited membership; a
separate unprivileged user is mandatory.
The `verified` and `system` schema grants provide name resolution solely for
these negative privilege checks; they grant no table read or write rights.

An authorized operator issues each admission through their own audited SQL
process, **not** an Activity, Workflow, or DAG input. Validate canonical
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

The canonical PostgreSQL `truth-context|{context_key}` advisory transaction
lock is acquired **before** reading admission or source rows. Under ordinary
READ COMMITTED, this gives a post-lock fresh snapshot; using SERIALIZABLE
here would incorrectly freeze a pre-lock snapshot. The stale-run predicate
matches legacy verified Truth's creation-order policy: a distinct completed
`verified_truth_write` from a run whose `created_at >= this_run.created_at`
blocks the effect, including ties. This is not an independent commit-time
ordering policy. A later-started run can reconcile after an older run commits
if its currently read source fingerprint matches what the operator approved.

## Opt-in execution

```bash
cd app/rust
cargo build --locked -p temporal_pg_effects --bin temporal_pg_effects
ALEGRIA_ENABLE_PG_EFFECT_V3=true \
  PG_EFFECT_WORKER_DATABASE_URL='postgres://<separate-limited-role>' \
  TEMPORAL_URL='http://localhost:7233' \
  ./target/debug/temporal_pg_effects
```

No deployment definition, default CI worker, or legacy `alegria-pipeline`
worker enables it. The pure DAG (`alegria-pure-dag-v3`) remains effect-free.
The only intended Workflow invocation is an explicitly authorized
`PgTruthReviewReconcileV3Workflow` command on its separate queue. Avoid
sharing that Task Queue with any worker lacking the exact registered Type.

## Acceptance gaps

- Live PostgreSQL role/admission/atomic-receipt and cross-queue Temporal E2E
  must pass on an isolated schema + namespace before production consideration.
- No isolated Temporal History replay or pinned Worker Deployment yet; a
  versioned evolution/restart test is required before any cutover.
- A raw evidence/section writer outside the canonical `truth-context` lock
  could change original bytes between separate READ COMMITTED statements.
  Independent concurrent raw-mutation E2E and a shared lock/trigger policy
  across all evidence writers are required for a complete source fence.
- Operator issuance/revocation governance, tenant isolation/RLS, execution
  admission provenance, role grants, audit and rate limits require review.
- Existing `TruthHitlReconcile` CLI can still invoke canonical PG review
  reconciliation directly under its legacy privilege model. This remains a
  migration consideration, not an alternate V3 effect authorization path.
- No Graph/Qdrant SDK is linked, and no Outbox dispatcher is started here.
