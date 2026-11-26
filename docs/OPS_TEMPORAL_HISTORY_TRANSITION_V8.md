# V8: read-only historical replay and rollback-byte rehearsal

This is an extension of [V7](OPS_TEMPORAL_HISTORY_TRANSITION_V7.md), not a
production migration. **Issue #94 and every production cutover remain NO-GO.**
The Python utility cannot deploy, stop a Worker, alter PostgreSQL, change a
Temporal Task Queue, start or cancel a Workflow, or exercise a live rollback.
It deliberately has no command that sets `cutover_authorized=true`.

## What V8 actually proves

The new `rehearse-artifacts` command requires the existing exact-source,
two-snapshot drain check and complete native SDK History replay when any
historical Workflow remains. It additionally verifies **actual bytes** of
both candidate and rollback Worker executables supplied by the operator:

- Every proposed owner/**Task Queue**/Build ID/role has exactly one local executable;
  missing, duplicate, foreign or unregistered routes fail closed.
- Expected SHA-256 and file size must agree with bytes independently read
  from the local artifact directory; symlinks, path escapes, nonexecutable
  files and oversized or changing files are rejected.
- No executable file, hard link/inode or SHA-256 may be reused across
  distinct owner/Task Queue/Build ID/role claims. This deliberately treats
  multi-queue sharing as **unproven physical independence**, even when the
  same executable could legitimately poll multiple queues.
- The rehearsal is bounded to 16 routes and 32 distinct artifacts; each
  must have an owner-executable mode and no group/world write permission.
- The report retains only owner, Task Queue, Build ID, role, SHA-256 and byte count;
  no credentials or executable paths are published.
- The dry-run neither executes the Worker nor calls Temporal/PostgreSQL;
  it never checks or changes real running pollers.

The exact status is `ROLLBACK_ARTIFACT_CANDIDATE_ONLY` with
`cutover_authorized=false`. This is **byte availability evidence only**.
Neither an operator-supplied checksum nor a local executable proves its
signed CI origin or that any live server routes to its Build ID. In
particular, JSON snapshots, routing manifests and both SHA-256 expectations
are **self-asserted inputs**; a malicious operator can manufacture a passing
dry-run from synthetic bytes without accessing GitHub or Temporal. The local
read is not atomic against a hostile concurrent writer replacing an ancestor
directory or rewriting a file of the same size. Use a private immutable
artifact directory and independent signed server/CI attestation later; V8
does not claim either guarantee.

## Native owner SDK replay routes (26.11.2026)

The same read-only `replay-audit` accepts real exported Histories for these
additional **already registered** native queues; the exact pair of Workflow
Type **and** Task Queue determines the replayer, never a generic Type alone:

| Task Queue | Workflow Types | Native CLI |
| --- | --- | --- |
| `alegria-pg-command-v5` | `PgTruthReviewReconcileV5Workflow` | `temporal_pg_effects --replay-v5-history` |
| `alegria-pg-owner-v1` | `AlegriaOwnerOperationV1Workflow`, `AlegriaOwnerValidateV1Workflow` | `temporal_pg_effects --replay-owner-history` |
| `alegria-qdrant-command-v2` | `QdrantKeywordProjectionV2Workflow` | `temporal_qdrant_effects --replay-history-v2` |
| `alegria-qdrant-owner-v1` | `AlegriaOwnerOperationV1Workflow`, `AlegriaOwnerValidateV1Workflow` | `temporal_qdrant_effects --replay-history-generic-v1` |

The new PostgreSQL replay CLI uses the real isolated owner queue and registers
both owner and side-effect-free validation Workflow Types; its historical V3/V4
and V5 queues remain distinct. No Worker is started, and replay neither reads
provider credentials nor performs an Activity. A History claiming the wrong
queue/Type fails before the SDK process is invoked.

**Not accepted:** `AlegriaOperationDagV1Workflow` and
`AlegriaOperationDagV2Workflow` parent Histories still require their original
root-owned immutable trusted route catalog. The replay tool deliberately
scrubs operator environment variables and cannot manufacture or guess that
catalog. Those routes remain fail-closed until a protected catalog snapshot
is bound to each actual parent History and its original source/contract set.
The new owner routes expand offline compatibility coverage only; they are
still `OFFLINE_SDK_REPLAY_CANDIDATE_ONLY`, never a genuine staging/production
History replay, drain proof or cutover authorization without independent
Temporal server/export and Worker Build ID attestation.

## Strict input manifest

Prepare an explicit operator-controlled private directory containing the
candidate and rollback executables. Supply a manifest bound to the same
source, environment and namespace as the earlier and later read-only
captures and the routing plan:

```json
{
  "schema_version": 1,
  "source": {"sha": "<full-40-character-lowercase-Git-SHA>", "tree": "<full-40-character-lowercase-Git-tree>"},
  "environment": "staging",
  "namespace": "alegria-staging",
  "artifacts": [
    {
      "owner": "orchestrator",
      "task_queue": "alegria-effect-dag-v5",
      "build_id": "orchestrator-v5",
      "role": "rollback",
      "file": "orchestrator-v5",
      "sha256": "<actual-64-character-lowercase-SHA256>",
      "size_bytes": 12345
    },
    {
      "owner": "orchestrator",
      "task_queue": "alegria-effect-dag-v5",
      "build_id": "orchestrator-v8",
      "role": "candidate",
      "file": "orchestrator-v8",
      "sha256": "<actual-64-character-lowercase-SHA256>",
      "size_bytes": 23456
    }
  ]
}
```

These sizes and digests are **placeholders**; they are not real artifact
attestations. Both files must be regular executable files inside the explicit
`--artifact-root`, not links or paths into other directories. The routing plan
must use the same owner, Task Queue and exact `from_build_id`, `to_build_id` and
`rollback_build_id` identities. A future new queue must already have a native
Workflow registered in the exact source being rehearsed.

After the genuine operator-observed captures, optional complete SDK replay and
local retrieval of the real signed CI binaries:

```bash
python3 automation/temporal_history_transition.py rehearse-artifacts \
  --earlier /secure/evidence/staging-first.json \
  --later /secure/evidence/staging-second.json \
  --replay-evidence /secure/evidence/staging-sdk-replay-candidate.json \
  --routing-plan /secure/evidence/operator-hypothetical-routes.json \
  --artifact-manifest /secure/evidence/rollback-artifacts.json \
  --artifact-root /secure/evidence/immutable-worker-binaries \
  --output /secure/evidence/staging-rollback-bytes-only.json
```

`--replay-evidence` can be omitted **only when the independently captured
whole-namespace historical count is zero**. The command does not accept a
plain test fixture as proof of genuine server History. A dry-run with synthetic
fixtures verifies only parser and control logic.

## Evidence still required before GO

1. Complete independently authenticated Temporal server exports by exact
   Workflow/Run ID for every historical execution, including older retired
   Workflow types; official SDK replay for each real History.
2. Actual tenant ownership census, historical backfill where needed, FORCE
   RLS and verified dedicated production LOGIN privileges.
3. Server-verified admission freeze for old starters; drain of all old Workers;
   no duplicate pollers for any Task Queue.
4. Signed exact-SHA CI provenance for **both** local binaries; Build ID to
   actual live Worker and Task Queue routing attested by Temporal server.
5. An authorized real rollback and recovery drill, independent mTLS and
   operator release authorization.

V8 never performs any of these external actions and cannot infer them from
test counts, checksums or a plan alone. The existing default legacy Worker
remains in production; the V7 pure candidate is explicitly opt-in only.

## Contract verification

```bash
python3 automation/test_temporal_history_transition.py
python3 automation/check_python_scope.py
```

The local suite covers positive and negative rollback binary byte checks,
unknown Build IDs, duplicate or missing entries, symlinks, path escape,
tampering, synthetic nonterminal states, and zero command execution during
dry-run. These tests are not substitute for the external industrial gates.
