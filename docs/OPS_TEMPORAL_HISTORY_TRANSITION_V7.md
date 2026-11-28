# V7: server-history compatibility, tenant quarantine and routing rehearsal

**#94 remains NO-GO.** This expands the [V6 read-only transition protocol](OPS_TEMPORAL_HISTORY_TRANSITION_V6.md).
The V7 tool has no commands that start/cancel/reset/delete a Temporal Workflow,
change Task Queue routing, perform tenant backfill, grant PostgreSQL rights, or
deploy a Worker. Every output has `"cutover_authorized": false`.

## Exact historical Workflow census: 19 known types

Two V7 Qdrant-owned versioned Workflows extend the 17-type census:

| Versioned owner | Workflow Type | Task Queue | Actual offline SDK mode |
| --- | --- | --- | --- |
| Parent / `temporal_orchestrator` | `AlegriaQdrantProjectionDagV1Workflow` | `alegria-qdrant-dag-v1` | `--replay-qdrant-effect-history` |
| Child / `temporal_qdrant_effects` | `QdrantKeywordProjectionV1Workflow` | `alegria-qdrant-command-v1` | `--replay-history` |

The child Activity Queue `alegria-qdrant-effects-v1` is **not** a Workflow
Type or replacement for its separate command Workflow Queue. There are now
11 exact native SDK replay Type/Queue routes covering 10 distinct Workflow
Types. Nine other recognized retired/types are **unreplayable** by these
binary entry points and block if present; unknown closed Types always fail
the whole-namespace `count(all)` equality check. Both new owners are
experimental; their presence in source or in the census is **not** a
production deployment authorization.

## Real Temporal Server History and the SDK JSON envelope

The existing repository real E2E invokes `WorkflowHistory::to_json()` on
histories fetched from the live Temporal Server, then runs the native SDK
replayer (legacy, pure DAG, PG). Those tests create disposable files and
delete them after replay; **there is no retained authentic server History JSON
fixture in the repository** at the time of this V7 change. A native replay
PASS for retained production/staging histories is therefore **not** claimed
here. The available execution environment also has no Cargo, Temporal CLI,
Docker, or precompiled native replay binaries to run a new live capture.

The V6 flat JSON format remains recognized. V7 additionally recognizes the
actual Rust SDK protobuf `HistoryEvent.attributes` **oneof** serialized by
`WorkflowHistory::to_json()`, with the integer
`EventType::WorkflowExecutionStarted` value `1` and nested
`WorkflowExecutionStartedEventAttributes` / snake-case or camel-case
Type/Task Queue fields. Contradictory flat+nested attributes, ambiguous aliases,
unknown first-event kinds, wrong Types/Queues, missing entries and SHA
mismatches remain fail-closed. These parser tests are **synthetic shape
contracts**, not native SDK replay acceptance.

To obtain real staging proof, the authorized operator must preserve each
**complete, server-attested** JSON History from the specific staging Workflow
ID/Run ID, immutable SHA and exact source checkout; run the V6 `capture`
twice, export all retained histories and provide a complete manifest. Then:

```bash
python3 automation/temporal_history_transition.py replay-audit \
  --capture /secure/evidence/staging-second.json \
  --manifest /secure/evidence/staging-all-histories.json \
  --history-root /secure/evidence/exports \
  --legacy-bin app/rust/target/release/temporal_worker \
  --orchestrator-bin app/rust/target/release/temporal_orchestrator \
  --pg-bin app/rust/target/release/temporal_pg_effects \
  --qdrant-bin app/rust/target/release/temporal_qdrant_effects \
  --output /secure/evidence/staging-sdk-replay-candidate.json
```

The Qdrant binary argument is optional **only if the live census has zero
Qdrant child Histories**. SDK replay uses protected temporary copies, never
mutable export paths; its subprocess receives no operator PG/Temporal/provider
secrets. The replay executable is likewise copied to a private temporary
directory, executed from the immutable `0500` copy, and reported as
`native_replayers[owner].sha256` together with its exact byte length. This
closes local pathname-swap races between binary verification and SDK replay.
Both verified copies disappear on success or failure. Binary hashing is **not**
a signed CI artifact provenance proof or server History attestation.
Even a successful `OFFLINE_SDK_REPLAY_CANDIDATE_ONLY` must be matched
to an independently signed server export, run-owner/tenant evidence, exact
binary attestation and rollback Worker fleet.

## Ten-table authoritative historical tenant quarantine (migration `0013`)

The read-only PostgreSQL census now depends on
`pipeline.tenant_historical_quarantine_v1()` and requires exactly **ten**
relations with nonnegative `unattributed` and `inconsistent` counts. It
cross-checks their sums and records `quarantine_by_relation` so the DBA can
see **which** historical class prevents the transition. All positives block,
including Qdrant admissions, attempts and receipts, execution runs, raw
pages/sections, candidates, HITL, PG admissions and receipts. Independently,
missing parent pages/sections or a receipt whose run/context disagrees with
its admission now blocks; an `INNER JOIN` cannot hide an orphan.

The migration creates only **prospective** label constraints and the read-only
DBA census; it neither fixes historical NULL tenants nor moves ownership.
The privileged audit principal requires global `BYPASSRLS` visibility and
explicit `EXECUTE` on the restricted census function (typically provisioned
by the DBA to one short-lived SELECT-only audit role). Missing migration,
function privilege, expected relation, or source-owner edge is NO-GO. Calling
`pipeline.assert_no_tenant_quarantine_v1()` alone does not grant cutover.

## Versioned Worker routing: nonmutating rehearsal

After two read-only drain snapshots and any required complete replay-audit,
an operator may check a **hypothetical** reversible Task Queue transition.
No task queue/build ID is changed or contacted. The proposed plan contains
only typed identifiers, matching source, environment and namespace:

```json
{
  "schema_version": 1,
  "source": {"sha": "<40-lowercase-hex>", "tree": "<40-lowercase-hex>"},
  "environment": "staging",
  "namespace": "alegria-staging",
  "routes": [{
    "task_queue": "alegria-effect-dag-v5",
    "owner": "orchestrator",
    "from_build_id": "orchestrator-v5",
    "to_build_id": "orchestrator-v7",
    "rollback_build_id": "orchestrator-v5"
  }]
}
```

```bash
python3 automation/temporal_history_transition.py rehearse \
  --earlier /secure/evidence/staging-first.json \
  --later /secure/evidence/staging-second.json \
  --replay-evidence /secure/evidence/staging-sdk-replay-candidate.json \
  --routing-plan /secure/evidence/operator-hypothetical-routes.json \
  --output /secure/evidence/staging-rehearsal-only.json
```

Omit `--replay-evidence` only if the authoritative namespace census has no
retained histories. The rehearsal rejects changed source/namespace,
nonterminal PG runs, tenant quarantine, missing SDK coverage or native
executable hashes, duplicate Queue owners, incorrect binary owner,
unknown/unregistered Task Queues, non-reversible Build IDs, and any attempt to
change a Queue that might still own retained histories. Conflicting aliases
in the History start-event envelope also fail closed. All these checks are
**model contracts**, not measurements of live routing/poller health.

A successful result is only `ROUTING_REHEARSAL_CANDIDATE_ONLY`, still with
`"cutover_authorized": false`. Independently verify the actual server's
historical run identities and live Worker/build routing, starters frozen,
operator-approved rollback drill, signed exact binaries and production mTLS.
No true production cutover has been performed.

## Local contract verification

```bash
python3 automation/test_temporal_history_transition.py
python3 automation/check_python_scope.py
```

This suite covers the known Workflow registration set, negative replay
manifest/TOCTOU/tenant gaps, nested SDK History envelopes, ten-table census,
and no-network routing rehearsal. It does not replace actual server History
replay or live PostgreSQL isolation E2E.
