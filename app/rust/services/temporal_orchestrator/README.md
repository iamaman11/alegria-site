# Provider-free Temporal pure DAG orchestrator (V3 experiment)

This **standalone Cargo package/binary** is an opt-in physical alternative to
the existing `temporal_worker`. It registers only
`AlegriaPureOperationDagV1Workflow`, accepts the typed
`seo_application::operation_adapter::PureDagRun` start argument, and returns
the JSON-encoded `PureDagReceipt`. Owner operation implementations stay in
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
- The input carries immutable plan hash, versioned contracts and admitted
  artifacts, not trusted provenance. Effectful operations and
  `allowed_effects` are rejected by the reused pure application adapter.
  Do not schedule stateful owner Activities from this workflow.

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
