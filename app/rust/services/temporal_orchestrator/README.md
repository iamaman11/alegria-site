# Provider-free Temporal checked DAG orchestrator (V3/V4/V5 evaluation)

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
On the separately gated `alegria-effect-dag-v5` queue, the distinct
`AlegriaAdmittedEffectDagV3Workflow` executes multiple separately attested
Truth reconciliation children in **declared dependency order**. Before any
child starts, V5 rejects reused admission IDs, inconsistent Truth context
keys within the canonical run, and unordered effects writing the same Truth
scope. Its closed typed owner route is an extension boundary, **not** a
generic permission to call arbitrary external APIs.
Owner operation implementations stay in
`seo_application`/domain crates; this package has no Activities, SQLx,
Neo4j, Qdrant or runtime_composition dependencies.

## Safety and routing

- Pure Task Queue: `alegria-pure-dag-v3`; V4/V5 admitted effect parent Task
  Queues: `alegria-effect-dag-v4` / `alegria-effect-dag-v5`; PG children always use
  `alegria-pg-command-v3` (the PG owner Activity is on its separate queue).
- Exactly one start gate must be true:
  `ALLOW_PURE_DAG_ORCHESTRATOR=true` for legacy pure and preflight-only V1,
  `ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=true` for the effectful V2 parent,
  or `ALLOW_EFFECT_DAG_ORCHESTRATOR_V5=true` for the ordered V3 parent.
  Multiple enabled or none enabled fails before a connection is made.
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

The V4 effect owner API (also used separately by each V5 child) binds the actual Temporal parent Run ID
(`pipeline.execution_runs.workflow_run_id`) and child Run ID to one
`(run_id,plan_hash,operation_instance)`, immutable canonical bytes and
`CheckedDispatch.execution_key`. The privileged operator must independently
confirm reciprocal parent/child Temporal Histories and the current Truth
source before committing admission; a parent-supplied identifier is not
authority. The PostgreSQL effect and its exact V4 receipt are atomic, while
legacy V3 receipt keys remain unchanged. The V4 parent validates the typed
child result before allowing the next graph step.

**Production acceptance remains NO-GO** even though live V4 Temporal
parent/child, operator admission, PostgreSQL effect, durable receipt and
negative race tests pass. Trusted production starter, authoritative tenant/RLS
migration, historical drain/replay/rollback, TLS/mTLS and multiple real owner
effect contracts remain outstanding. Never overwrite an existing legacy run's
`workflow_run_id`.
Effects accept externally bound canonical Truth review scopes only; a claimed
produced artifact from an untrusted pure operation does not grant PG authority.
The V4 adapter is intentionally not an unrestricted effectful business RPC.
V5 still supports only the same PostgreSQL Truth owner. Since one canonical
`pipeline.execution_runs` row has exactly one `context_key`, two Truth effects
in the same DAG must be ordered. V5 **does not** claim actual parallel fanout
until other trusted owners with distinct scopes are physically proven.

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

To evaluate the independently gated **V5 ordered multi-child** parent:

```sh
ALLOW_PURE_DAG_ORCHESTRATOR=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V5=true \
  TEMPORAL_ORCHESTRATOR_TASK_QUEUE=alegria-effect-dag-v5 \
  WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/debug/temporal_orchestrator
```

Offline native History replay (one queue per version):

```sh
target/debug/temporal_orchestrator --replay-history /path/to/actual-temporal-history.json
target/debug/temporal_orchestrator --replay-effect-history /path/to/v4-parent-history.json
target/debug/temporal_orchestrator --replay-v5-effect-history /path/to/v5-parent-history.json
```

Successful replay emits `CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS`. Malformed,
missing or nondeterministic history fails nonzero. Legacy `alegria-pipeline`
V1/V2 history must continue to be replayed by `temporal_worker`.

The V4 evaluation profile is opt-in only; there is no production cutover.
