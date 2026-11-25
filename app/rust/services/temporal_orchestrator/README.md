# Provider-free Temporal checked DAG orchestrator (V3/V4 evaluation)

This **standalone Cargo package/binary** is an opt-in physical alternative to
the existing `temporal_worker`. It registers
`AlegriaPureOperationDagV1Workflow` (typed `PureDagRun` → JSON
`PureDagReceipt`) and `AlegriaCheckedEffectDagV1Workflow` (typed
`EffectDagRunV1` → non-retryable failure for every effectful plan).
On the **separately gated V4 Task Queue** it registers only
`AlegriaAdmittedEffectDagV2Workflow`. This new history-visible parent
preflights the entire mixed DAG and starts genuine typed owner child Workflows
for each PostgreSQL reconciliation operation; the children cannot write until
a separate operator verifies both Temporal Histories and commits V4 PG
admission. V1 replay semantics remain unchanged.
Owner operation implementations stay in
`seo_application`/domain crates; this package has no Activities, SQLx,
Neo4j, Qdrant or runtime_composition dependencies.

## Safety and routing

- Pure Task Queue: `alegria-pure-dag-v3`; admitted effect parent Task Queue:
  `alegria-effect-dag-v4`; the owner child always uses
  `alegria-pg-command-v3` (the PG owner Activity is on its separate queue).
- Exactly one start gate must be true:
  `ALLOW_PURE_DAG_ORCHESTRATOR=true` for legacy pure and preflight-only V1,
  or `ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=true` for the new effectful V2 parent.
  Both enabled or neither enabled fails before a connection is made.
- `WORKER_BUILD_ID` is mandatory and must be exactly a nonzero 40-hex source
  commit SHA, including in the unversioned local E2E mode.
- To use native Temporal Worker Deployments, set
  `TEMPORAL_WORKER_VERSIONING=true`; this registers `VersioningBehavior::Pinned`
  with deployment name equal to the selected queue and the exact source build ID.
  With versioning false/absent, no server-side pinning occurs: this is only
  suitable for a controlled, nonproduction proof and is **not** evidence of
  compatible history evolution.
- `TEMPORAL_ORCHESTRATOR_TASK_QUEUE`, if provided, must equal the selected queue.
  `TEMPORAL_URL` defaults to `http://localhost:7233`; namespace defaults to
  `default` unless `TEMPORAL_NAMESPACE` is set.
- Both inputs carry immutable plan hash, versioned contracts and admitted
  artifact envelopes. These declarations are **not authenticated producer
  provenance** or operator consent. The original pure Workflow still rejects
  all effects. The checked-effect Workflow validates the entire heterogeneous
  graph, canonical Truth reconciliation contract, typed owner command,
  declared input bytes, scope and checked dispatch identity **before**
  non-retryably failing every effectful request. It never schedules an
  Activity or child Workflow and cannot return a counterfeit PG receipt.

The V4 effect owner API separately binds the actual Temporal parent Run ID
(`pipeline.execution_runs.workflow_run_id`) and child Run ID to one
`(run_id,plan_hash,operation_instance)`, immutable canonical bytes and
`CheckedDispatch.execution_key`. The privileged operator must independently
confirm reciprocal parent/child Temporal Histories and the current Truth
source before committing admission; a parent-supplied identifier is not
authority. The PostgreSQL effect and its exact V4 receipt are atomic, while
legacy V3 receipt keys remain unchanged. The V4 parent validates the typed
child result before allowing the next graph step.

**Production acceptance remains NO-GO** until real live E2E prove the
server-issued parent/child relationship, controlled creation of a *fresh*
canonical pipeline run already bound to that parent Run ID, operator issuance,
admitted PostgreSQL effect, exact durable receipt, restart/replay and negative
race tests. Never overwrite an existing legacy run's `workflow_run_id`.
Effects accept externally bound canonical Truth review scopes only; a claimed
produced artifact from an untrusted pure operation does not grant PG authority.
The V4 adapter is intentionally not an unrestricted effectful business RPC.

Run from `app/rust`:

```sh
cargo build --locked -p temporal_orchestrator --bin temporal_orchestrator
ALLOW_PURE_DAG_ORCHESTRATOR=true WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/debug/temporal_orchestrator
```

For a **separate, disabled-by-default V4 evaluation deployment**:

```sh
ALLOW_PURE_DAG_ORCHESTRATOR=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=true \
  TEMPORAL_ORCHESTRATOR_TASK_QUEUE=alegria-effect-dag-v4 \
  WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/debug/temporal_orchestrator
```

For offline replay of a **real history from this new Workflow/task queue**:

```sh
target/debug/temporal_orchestrator --replay-history /path/to/actual-temporal-history.json
target/debug/temporal_orchestrator --replay-effect-history /path/to/v4-parent-history.json
```

Successful replay emits `CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS`. Malformed,
missing or nondeterministic history fails nonzero. Legacy `alegria-pipeline`
V1/V2 history must continue to be replayed by `temporal_worker`.

The V4 evaluation profile is opt-in only; there is no production cutover.
