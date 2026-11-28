# V6 historical Temporal replay and state-gated cutover preflight (#94)

**Operational state: NO-GO.** This is a **read-only** staged runbook. It
neither performs a production migration nor grants authority to move Worker
routing. Do not infer launch readiness from a green unit test, `capture`,
`compare`, or even an offline replay `PASS`. The canonical V5 baseline is
[`OPS_TEMPORAL_HISTORY_TRANSITION_V5.md`](OPS_TEMPORAL_HISTORY_TRANSITION_V5.md).

## Why a historical zero-open census is insufficient

Temporal's closed histories can still require an old Worker implementation
when reset, queried for recovery, or retried. Today's code registers fewer
types than the entire historical namespace. The verifier therefore inventories
**all 17 explicitly recognized Workflow Types**, including retired names,
with `temporal workflow count`. The sum of per-type **total** counts must equal
an independent whole-namespace **total** count, not merely the currently open
count. A missing/unknown Workflow Type or moving count fails closed.

An accepted history export must be bound to its **Workflow Type, Task Queue,
Workflow ID, Run ID, SHA-256, source Git SHA/tree, environment and Temporal
namespace**. The export itself is replayed via the real Rust Temporal SDK
`WorkflowReplayer`, not by a JSON parser or simulated success flag. Its
`WorkflowExecutionStarted` event must agree with the declared Type and Queue.

The CLI deliberately **cannot independently prove** a file came from the
specific claimed server Workflow ID/Run ID, that the server export is complete,
that its replay binary was built and signed from the exact source, or that the
old starters/pollers have been fenced. Those remain external gates with
independent evidence. Do not accept self-attested manifest values as server
proof, and do not assign a default tenant to unresolved historical rows.

## Owner versions and offline SDK routes

One Workflow Type can have **multiple owner/Task Queue generations**. Route
on both fields; never start a V5 Worker on the V4 Queue or release a provider
or PostgreSQL credential to an offline replay process.

| Native binary owner | Exact Task Queue | Historical Types SDK-replayable there | CLI argument |
| --- | --- | --- | --- |
| `temporal_worker` (legacy SEO) | `alegria-pipeline` | `SeoSiteBuildCanonicalCutoverWorkflow`, `SeoSiteBuildComposableV2Workflow`, `AlegriaPureOperationDagV1Workflow` | `--replay-history` |
| `temporal_orchestrator` (V3 pure DAG) | `alegria-pure-dag-v3` | `AlegriaPureOperationDagV1Workflow`, `AlegriaCheckedEffectDagV1Workflow` | `--replay-history` |
| `temporal_orchestrator` (V4 effect DAG) | `alegria-effect-dag-v4` | `AlegriaAdmittedEffectDagV2Workflow` | `--replay-effect-history` |
| `temporal_orchestrator` (V5 effect DAG) | `alegria-effect-dag-v5` | `AlegriaAdmittedEffectDagV3Workflow` | `--replay-v5-effect-history` |
| `temporal_pg_effects` (PG command) | `alegria-pg-command-v3` | `PgTruthReviewReconcileV3Workflow`, `PgTruthReviewReconcileV4Workflow` | `--replay-history` |

The other **nine** recognized historical Workflow Types have **no registered
offline replay route** in these binaries and **must block** if the captured
namespace includes any of them. New registered Types require an explicit census
addition and a separately verified SDK replay implementation. Inventing a
route to satisfy the manifest is not allowed.

## Stage 1: independently capture a stable live inventory

Apply reviewed database migrations before attempting an audit. Use a trusted
operator with short-lived DBA read-only `BYPASSRLS` visibility; the normal
tenant-scoped worker account is **not** a suitable audit principal. In
particular, admissions/receipts with missing parent rows, empty `tenant_id`,
unregistered contexts, tenant-crossing source edges, and non-terminal PG
execution runs all block.

On an authorized operator host, from an **exact clean checkout** of the
approved source SHA:

```bash
export ALEGRIA_HISTORY_AUDIT_DSN='...secret-manager supplied...'
SOURCE_SHA="$(git rev-parse HEAD)"
python3 automation/temporal_history_transition.py capture \
  --environment staging \
  --temporal-address temporal-staging.internal:7233 \
  --temporal-namespace alegria-staging \
  --source-sha "$SOURCE_SHA" \
  --output /secure/evidence/staging-first.json
```

For a namespace with *retained* historical Workflows, a captured `BLOCKED`
status containing only `historical SDK replay remains unproved: ...` is
expected; it must not be overridden manually. Any additional blocker stops
the workflow immediately. Perform the old-starter admission fence through a
separately approved operation, then make a **second independent capture**
between 30 seconds and 30 minutes later with the same source and environment.
The most recent capture must be no older than 10 minutes at comparison.

These commands are illustrative only; they have **not** been executed against
staging or production for #94 in this change.

## Stage 2: obtain an independent complete server History export

Under independent operator control, export the **full JSON Event History**
for **every retained Workflow ID + Run ID** in the captured namespace, not
just an arbitrary sample or the first results page. Preserve the bytes
verbatim, record a cryptographic SHA-256 for each file, and independently
attest the server namespace, exported Run identities and export completeness.
Keep the source inventory and manifest in an access-controlled evidence store.

Supported envelope: a JSON object with `events` (or `history.events`) whose
first event is `EVENT_TYPE_WORKFLOW_EXECUTION_STARTED` and has
`workflowExecutionStartedEventAttributes.workflowType.name` and
`workflowExecutionStartedEventAttributes.taskQueue.name`. Unknown export
formats fail closed; review the actual server/SDK format before changing the
parser. Exports over 16 MiB per file are explicitly refused pending a reviewed
streaming implementation; do not silently truncate them.

Example manifest for **one** V5 orchestrator execution (replace every
placeholder with recorded evidence):

```json
{
  "schema_version": 1,
  "source": {"sha": "<40-lowercase-hex>", "tree": "<40-lowercase-hex>"},
  "environment": "staging",
  "namespace": "alegria-staging",
  "histories": [
    {
      "workflow_type": "AlegriaAdmittedEffectDagV3Workflow",
      "task_queue": "alegria-effect-dag-v5",
      "workflow_id": "<server-attested-workflow-id>",
      "run_id": "<server-attested-run-id>",
      "file": "history-v5.json",
      "sha256": "<64-lowercase-hex>"
    }
  ]
}
```

The manifest is valid **only** if the complete namespace census contains
exactly that one Workflow in the corresponding type, with all other recognized
type totals zero. Exports must be under one directory (no symlink or `..`
traversal), unique by Workflow/Run ID and file hash. Missing, duplicated,
cross-Queue, unsupported or hash-mismatched histories fail **before** calling
any replayer. After verification, the script writes each SHA-identified History
into a private `0700` temporary directory as a read-only `0400` file and
passes **only that protected copy** to the native SDK. It never reopens the
mutable export path for replay, and destroys every temporary copy on both
success and error. This closes the export-file replacement race between hash
verification and SDK consumption. The native child environment inherits only
a short allowlist of non-credential runtime variables, not PostgreSQL,
Temporal or provider secrets. No external effects or Temporal/PG connections
are made by the native replay-only entry points.

## Stage 3: native offline SDK replay, then state comparison

Use verified, executable binaries from the exact reviewed source. The script
does not certify their signatures: preserve a separate signed build
attestation and deterministic provenance. Invoke the same read-only CLI:

```bash
python3 automation/temporal_history_transition.py replay-audit \
  --capture /secure/evidence/staging-second.json \
  --manifest /secure/evidence/complete-manifest.json \
  --history-root /secure/evidence/exports \
  --legacy-bin app/rust/target/release/temporal_worker \
  --orchestrator-bin app/rust/target/release/temporal_orchestrator \
  --pg-bin app/rust/target/release/temporal_pg_effects \
  --output /secure/evidence/staging-replay-audit.json

python3 automation/temporal_history_transition.py compare \
  --earlier /secure/evidence/staging-first.json \
  --later /secure/evidence/staging-second.json \
  --replay-evidence /secure/evidence/staging-replay-audit.json \
  --output /secure/evidence/staging-preconditions.json
```

Every replay must exit successfully **and** emit the exact registered SDK
marker. Missing binaries or markers fail closed. The report enumerates count
by Type, the sorted SHA-256 of every replayed file, and an individual
`replayed_runs` record containing Workflow Type, Task Queue, Workflow ID,
Run ID and SHA-256. `compare` rejects duplicate run identities or hashes,
unsupported Type/Queue combinations and a per-run multiset that disagrees with
the captured namespace census. `compare` checks both
inventory snapshots independently, rejects any new PG/tenant/open-Workflow
blocker, verifies that historical totals did not change, and binds the replay
report to the same source/environment/namespace/counts. It does **not** trust
the report as proof of server origin; independent Temporal Server attestations
must match each `replayed_runs` record before a production change review.

## Allowed diagnostic states (none authorizes cutover)

| Diagnostic status | Meaning | Next external prerequisite |
| --- | --- | --- |
| `BLOCKED` | Counts, permissions, schema, owner relations, unknown Types, replay, or timeline cannot be trusted | Resolve cause; recapture with a new independent audit |
| `DRAIN_CANDIDATE_ONLY` | One empty-history, zero-open, tenant-clean snapshot | Verify starter fence; take second capture |
| `DRAIN_PRECONDITIONS_CAPTURED` | Two fresh stable empty-history snapshots | Independent routing/version/rollback approval |
| `OFFLINE_SDK_REPLAY_CANDIDATE_ONLY` | Every counted retained history had a matching registered native SDK replay | Server-verify each History's Workflow/Run identity and exact build provenance |
| `HISTORY_REPLAY_PRECONDITIONS_CAPTURED` | Two stable snapshots plus the matched offline replay report | Signed independent completeness, route, freeze and rollback attestations |

Every output is machine-readable JSON with `"cutover_authorized": false`.
Do **not** build a release pipeline that treats command exit code zero or
`*_PRECONDITIONS_CAPTURED` as permission to deploy. In particular, a forged
report could claim replay coverage: the only safe production authorization
comes from independent Temporal Server evidence, operator-signed binary,
explicit starter fence, versioned Task Queue ownership, verified old Worker
availability, rollback exercise, production mTLS, and an approved change
control. These are outside this read-only CLI's authority.

## Local regression suite

```bash
python3 automation/test_temporal_history_transition.py
```

This suite checks all recognized types against the Rust Worker registrations,
guards fake/incomplete manifests, queue mismatches, wrong SHA, stale evidence,
wrong owner, missing binaries, tenant mismatches, and forged cutover authority.
It **does not** contain retained production History exports or assert a
successful replay of real server histories. Independent staging replay remains
mandatory before production acceptance.
