# Issue #94 — PG Truth review reconciliation: owner/effect boundary (V3 slice)

> Status: source-linked decision and regression boundary for **one** durable
> PostgreSQL business operation, not acceptance of the other 51+ candidate
> decisions. Canonical review remains Draft PR #95. This document distinguishes
> existing V1 behavior from the new V3 owner contract; it does not claim a
> production rollout, Temporal E2E proof or successful independent Rust build.

## 1. Business identity and owner, not an Activity count

**Decision: KEEP as independently callable PG stateful operation**
`pg.truth.reconcile_candidate_reviews`, revision `1`. This operation ensures
that a context's unresolved canonical Truth candidates have durable human
review tasks, and that obsolete or stale review tasks lose authority.
It does **not** adjudicate, verify or publish Truth. This is a real business
effect (`pg.truth.review_queue.reconcile`), not a synthetic step ledger, a
Temporal Activity, a generic SQL executor or a fifth/sixth phase of a fixed
site-builder recipe.

Evidence in the canonical Rust/SQL implementation:

- [`sqlx_hitl_adapter.rs`](../app/rust/crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs)
  `reconcile_truth_candidate_reviews_in_tx` currently owns four SQL changes in
  one transaction: supersede changed/rechecked candidate reviews, supersede
  orphan reviews after raw re-extraction, reopen changed projection of an
  unresolved candidate, insert missing `needs_hitl` reviews.
- [`sqlx_truth_write_adapter.rs`](../app/rust/crates/infrastructure/src/adapters/sqlx_truth_write_adapter.rs)
  calls that function **in the same transaction** as extracted candidate and
  verified-rule writes. Moving the SQL into `postgres_store` must preserve
  this entry; a separate Activity call is not a replacement for the atomic
  V1 Truth transaction.
- [`schema.sql`](../app/db/schema.sql) defines candidate FK to
  `kb.visa_contexts`, immutable candidate identity, and `site.seo_hitl_tasks`
  task identity/queue-state/audit columns; migration
  [`20261114_0001_truth_candidate_review_hitl.sql`](../app/db/migrations/20261114_0001_truth_candidate_review_hitl.sql)
  makes `truth_candidate_review` a valid queue type.
- [`truth_runtime_tests.rs`](../app/rust/crates/integration_harness/src/tests/truth_runtime_tests.rs)
  exercises one insertion, replay with zero new insertions, recheck
  supersession, exact-source rejection retention, changed-extraction reopen.
  [`outbox_delivery_tests.rs`](../app/rust/crates/integration_harness/src/tests/outbox_delivery_tests.rs)
  rolls back the **entire** Truth transaction after a late failure and
  requires no leaked HITL rows.

The intended physical owner is
[`postgres_store::sqlx_hitl_adapter`](../app/rust/crates/postgres_store/src/sqlx_hitl_adapter.rs)
with the dedicated
`temporal_pg_effects` service and task queue `alegria-pg-effects-v3`. Legacy
`infrastructure::sqlx_hitl_adapter` should be **removed**, and V1 Truth,
CLI and integration harness callers must import the PG owner directly: no
nested compatibility facade and no second SQL implementation. The existing PG
audit and human decision helpers move with their SQL implementation into
`postgres_store`; only reconciliation is registered as the V3 operation in
this slice. The new owner **may** depend on
`postgres_store`, `operation_protocol`, pure domain types and Temporal SDK;
it must not link `runtime_composition`, graph or vector providers. The old
cross-provider Temporal worker cannot serve as proof of independent ownership.

## 2. Typed contract and effect semantics

| Contract surface | Exact intended semantics / required evidence |
| --- | --- |
| Operation reference | `pg.truth.reconcile_candidate_reviews`, revision `1`; **PG-owned** (not `pure`). |
| Required input | `ReconcileTruthReviewsInput { admission_id: Uuid, run_id: Uuid, scope: RunScope { tenant_id, run_id }, context_key, operation_instance, plan_hash }`; all fields matched to privileged PG admission, never treated as self-authorizing. Existing SQL accepts only `(context_key: &str, run_id: &str)`, not a candidate-ID/hash filter. |
| Requested effect | Exact `pg.truth.review_queue.reconcile`; contract declaration **and** node permission are necessary plan checks, but neither is an authorization token. |
| Authoritative admission | `pipeline.pg_effect_v3_admissions` must be provisioned by a **separate privileged operator**: exact admission UUID, run UUID, tenant, context, workflow ID, operation instance, plan hash, operation/revision, effect ID and contract fingerprint; not expired/revoked. V3 worker checks PG-observed `pipeline.execution_runs` and hashes the **whole canonical candidate rowset** at effect time; no trust in plan `allowed_effects` or `ExternalArtifact.provenance`. No cross-tenant/context fallback. |
| Stale-run fence | Tie the V3 causal run to `pipeline.execution_runs` (context, workflow type, status) and another run's completed `verified_truth_write` in `pipeline.step_executions`. The current V3 SQL rejects a completed Truth step when **its run's `created_at` is newer/equal** to the admitted run; the predicate executes after the shared PG context advisory lock. This is *run-start chronology*, not a proof of Truth COMMIT-time ordering across an older-created run that finishes later. Candidate-rowset hash also fences the observed canonical projection. |
| Output | `ReconcileTruthReviewsReceipt { receipt_id, admission_id, execution_key, run_id, context_key, candidate_set_hash, created_review_tasks, open_review_tasks }`. `created_review_tasks` counts only newly INSERTed rows; `open_review_tasks` comes from a separate post-reconciliation PG SELECT. Neither reports all reopened/superseded transitions. |
| Error classes | Typed `InvalidCommand`, `AdmissionDenied`, `StaleTruthRun`, `CandidateSetChanged`, `PrivilegedWorkerRole` → `RetryClass::Never`. SQL / infrastructure-unavailable → `RetryWithSameAdmission`. Unknown post-COMMIT ACK → check same admission/`execution_key` receipt before replay. Contract revision/fingerprint mismatch → fail closed. |
| Idempotency | Stable task key `truth_candidate_review@1:<rule_candidate_id>:<source_snapshot_hash>` and `ON CONFLICT DO NOTHING` for insertion; reopening updates existing task with audit. Same task/data replay inserts zero new tasks; a new *canonical* projection may legitimately reopen a task. Run/plan/node/input identity must not become authority to create duplicate review tasks. |
| No epistemic escalation | The effect may write only queue/audit fields of `site.seo_hitl_tasks`. It **must not** `UPDATE/INSERT verified.rule_instances`, change candidate `epistemic_status`, grant human approval, send CMS publish, or emit direct Neo4j/Qdrant writes. Only the independent adjudicator can later verify Truth. |
| Source visibility | `site.seo_hitl_tasks` is the materialized review queue; `extracted.rule_candidates` and source snapshot are read by PG for freshness/identity; audit history append-only for supersession/reopen, not erased to make approval appear fresh. |
| Replay / concurrency | Transactional PG reconciliation; concurrent calls and process crash must converge on canonical task identities. Temporal ACK alone is not durability. If the worker has no durable receipt for ambiguous completion, report uncertainty instead of inventing exactly-once guarantees. |
| Least-privilege worker | On startup `assert_unprivileged_pg_effect_role` refuses a PG role that can INSERT/UPDATE/DELETE/TRUNCATE admissions (including inherited grants/owner privileges). Admission is immutable and external to untrusted plans. The role needs only the narrowly required read/reconcile/receipt grants; audit the deployed grants separately. |

The canonical V1 writer takes
`pg_advisory_xact_lock(hashtextextended('truth-context|' || context_key, 0))`
via [`projection_runtime_store.rs`](../app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_runtime_store.rs)
before candidates/HITL/verified writes. V3 must use the identical lock namespace
and order **before** deciding whether its `pipeline.execution_runs` admission
is stale. An isolated check that observes no newer run *before* this lock can
race with a canonical Truth commit and is not a safety fence.
The late-completion case (a run created earlier but committing verified Truth
after the admitted V3 run began) requires a separate scenario test; do not
declare a stronger wall-clock/COMMIT ordering guarantee from the `created_at`
predicate alone.

Current SQL does a **context-wide** reconciliation: it reads every
`extracted.rule_candidates` row in the given `context_key` in `needs_hitl`
state and reconciles all matching open/reopened/in-review rows in
`site.seo_hitl_tasks`. A caller-supplied candidate or source snapshot hash
must **not** be described as a narrowed query unless the owner actually
enforces a matching PG-side predicate; a hash can at most be an admission
precondition/fence. In particular, `run_id` is currently an audit cause, not
an implicit SQL partition key.

## 3. SQL transition matrix (actual, not invented protocol steps)

| PG-observed candidate/review state | Permitted queue effect | Required preservation |
| --- | --- | --- |
| `needs_hitl`, no task for exact candidate+snapshot | INSERT `open`, owner `truth_reviewer`, blocking severity, evidence/provenance payload | Stable `truth_candidate_review@1` task key; `truth_candidate_needs_review` audit |
| Candidate changed source snapshot or independently rechecked out of `needs_hitl` | Supersede active stale task | `truth_candidate_rechecked` append-only audit; human approval does not verify |
| Raw re-extraction deleted candidate | Supersede orphan active task | `truth_candidate_source_missing` audit; do not resurrect stale source |
| Same candidate/snapshot unresolved but projection fields changed, or task previously superseded and candidate again unresolved | Reopen same task, update evidence, drop old `review` verdict | `truth_candidate_reopened_for_review` audit, no duplicate owner |
| Candidate independently `rejected` with exact negative review still applicable | Preserve negative review for canonical governance | The `reject` veto is not forgotten on recheck; no fabricated `verified` |
| Identical replay/no canonical input change | No new task; no spurious reopen | Legacy inserted count `0`; effect receipt must not mislabel as no task-state work in unrelated changed context |

SQL source predicates are the authority; the matrix is not a license for a
Workflow to choose a transition and write `site.seo_hitl_tasks` itself.

## 4. Capability and security matrix

| Initiator / route | Reads | Writes | Admission/routing rule |
| --- | --- | --- | --- |
| V3 pure DAG (`temporal_orchestrator`, `alegria-pure-dag-v3`) | Pinned owner contract / external binding metadata | **No PG effects** | May schedule only a checked owner handoff; cannot itself execute or infer a PG effect from a plan JSON flag. |
| V3 PG owner (`temporal_pg_effects`, `alegria-pg-effects-v3`) | PG context, canonical extracted candidates, current Truth review queue | **Only** Truth review task reconciliation and a durable execution receipt, if implemented | Typed operation+revision, exact effect, verified scope/run/admission; deny before DB mutation on mismatch. |
| V1 canonical Truth write | Candidate/source state inside own transaction | Candidates, verified Truth, HITL and existing Truth/Outbox journal | Keep reconciliation **in the owning Truth transaction**. No new nested workflow or cross-transaction facade. |
| Operator `truth-hitl-reconcile` | PG current candidates | PG HITL queue directly | **Still enabled** in [`truth_hitl.rs`](../app/rust/services/cli_tools/src/cli/truth_hitl.rs). It is an explicitly tracked direct PG route, **not** a V3 owner-routed proof. Cutover requires its own admission/reachability decision. |
| Operator `truth-hitl-list` / `show` | PG HITL queue and evidence | None | Read-only review inspection. |
| Operator `truth-hitl-decide` | Exact task+source+candidate/review | PG human review/audit | Separate human decision, scoped/fenced in [`sqlx_hitl_adapter.rs`](../app/rust/crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs); **not** this reconciliation operation. |
| Operator `seo-run`, `crawl-pending-sources` | None in the disabled command arms | None | [`main_entry.rs`](../app/rust/services/cli_tools/src/cli/main_entry.rs) returns `CLI_DIRECT_SITE_COMMAND_DISABLED` before invoking the legacy composed implementation. They are discoverable CLI syntax, **not** V3 runnable operations. |
| Neo4j/Qdrant/HTTP/browser runtime owners | Provider-specific | Provider-specific | **Not reachable** from this V3 PG owner. Existing Outbox, backfill and advisory paths require independent decisions; no transitive provider facade. |

The remaining direct operator paths (`BuildStaticSite`, `RawReextractEmpty`,
CMS decisions, `OntologyBackfillPlan --apply-*`, `search_whole_page_prototypes`
implicit seed, analytics GDS cache) retain their separately audited status in
[`ISSUE94_OPERATION_BOUNDARY_DECISIONS.md`](ISSUE94_OPERATION_BOUNDARY_DECISIONS.md).
Disabling only `seo-run` and `crawl-pending-sources` does not quarantine those.

## 5. Physical acceptance / red lines

The automated [`check_v3_effect_owner_boundary.py`](../automation/check_v3_effect_owner_boundary.py)
checks several **source reachability/owner** invariants with mutation tests:
the direct CLI disable arms, the V1 in-transaction call, the allowed
reconciliation table, and the V3 crate's transitive lack of provider/mixed
composition dependencies when the service exists. It does **not** prove SQL
atomicity, durable receipts, actual Temporal registration, deployed Task Queue
isolation, PG authorization, or every source path at runtime.

Acceptance still requires independently reported evidence:

1. `cargo check -p postgres_store -p temporal_pg_effects` and owner-specific
   tests; package discovery in a standalone workspace/closure, not merely
   successful build of the old monolith.
2. PG transaction tests for empty/other-context denial, exact snapshot,
   concurrent retries, task reopen, source deletion, negative review, and
   ambiguous post-COMMIT ACK/receipt recovery. The existing V1 atomic
   rollback test must remain green.
3. Temporal opt-in and queue proof: worker **off** by default, a V3 unknown
   operation/effect is not scheduled, no V1 task consumers accidentally poll
   `alegria-pg-effects-v3` or vice versa, audit identifies the run/contract.
4. A/D tests across genuinely different subgraphs with this PG effect
   positioned under different valid causal orders, and a negative unauthorized
   plan; old fixed SEO stage order is not accepted as DAG proof.
5. Explicit routing/admission decision for still-active operator
   `truth-hitl-reconcile`; preserve V1 semantics while transitioning.

No source inventory count is promoted to a normative operation count here.
The 73 Temporal Activities, 22 legacy phase keys, 52 candidate decisions and
five pure registrations in [`ISSUE94_OPERATION_DECISION_MATRIX.md`](ISSUE94_OPERATION_DECISION_MATRIX.md)
are discovery evidence, not completion percentages for Issue #94.
