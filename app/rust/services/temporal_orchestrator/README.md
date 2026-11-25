# Provider-free Temporal checked DAG orchestrator (V3/V1 experiment)

This **standalone Cargo package/binary** is an opt-in physical alternative to
the existing `temporal_worker`. It registers
`AlegriaPureOperationDagV1Workflow` (typed `PureDagRun` → JSON
`PureDagReceipt`) and `AlegriaCheckedEffectDagV1Workflow` (typed
`EffectDagRunV1` → non-retryable failure for every effectful plan).
Owner operation implementations stay in
`seo_application`/domain crates; this package has no Activities, SQLx,
Neo4j, Qdrant or runtime_composition dependencies.

## Safety and routing

- Task Queue: `alegria-pure-dag-v3` (immutable separate queue, not `alegria-pipeline`).
- Explicit start gate: `ALLOW_PURE_DAG_ORCHESTRATOR=true`; absent or any other
  value fails before a connection is made. No deployment config enables it.
- `WORKER_BUILD_ID` is mandatory and must be exactly a nonzero 40-hex source
  commit SHA, including in the unversioned local E2E mode.
- To use native Temporal Worker Deployments, set
  `TEMPORAL_WORKER_VERSIONING=true`; this registers `VersioningBehavior::Pinned`
  with deployment name `alegria-pure-dag-v3` and the exact source build ID.
  With versioning false/absent, no server-side pinning occurs: this is only
  suitable for a controlled, nonproduction proof and is **not** evidence of
  compatible history evolution.
- `TEMPORAL_ORCHESTRATOR_TASK_QUEUE`, if provided, must equal the fixed queue.
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

The effectful graph intentionally remains **NO-GO for execution**. The
existing PostgreSQL V3 owner admission binds one canonical
`pipeline.execution_runs.workflow_run_id` to the actual server-issued
Temporal Run ID. A mixed parent Workflow and multiple child effect
Workflows necessarily have distinct server Run IDs. Reusing the parent
row, overwriting its Run ID or accepting a manifest hash as authorization
would break PG provenance. Production support needs a **new versioned,
operator-issued per-operation-instance admission** that independently
binds parent/child Run IDs, immutable scope/command/input digest, canonical
producer snapshot and durable owner receipt, plus real server-start/replay
and adversarial concurrency tests. This is not provided by the V1 preflight.

Run from `app/rust`:

```sh
cargo build --locked -p temporal_orchestrator --bin temporal_orchestrator
ALLOW_PURE_DAG_ORCHESTRATOR=true WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/debug/temporal_orchestrator
```

For offline replay of a **real history from this new Workflow/task queue**:

```sh
target/debug/temporal_orchestrator --replay-history /path/to/actual-temporal-history.json
```

Successful replay emits `CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS`. Malformed,
missing or nondeterministic history fails nonzero. Legacy `alegria-pipeline`
V1/V2 history must continue to be replayed by `temporal_worker`.

The V3 experiment is not wired into Docker/production routing. Measure build
isolation and a real server run/replay before considering deployment.
