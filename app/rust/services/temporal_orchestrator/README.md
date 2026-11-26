# Native generic Temporal operation DAG orchestrator

The **default binary** registers only the generic, owner-neutral operation DAG
parents (`AlegriaOperationDagV1Workflow` and
`AlegriaOperationDagV2Workflow`). It depends on `operation_protocol` and the
official Temporal SDK, **not** `seo_application` or `temporal_gateway`.
It holds no provider credentials and registers no Activities. Each owner runs
its typed child Workflow on an independent Task Queue.

Historical PureV3, admitted-effect V4/V5 Workflows and their replay remain
available only when explicitly built with `--features legacy-workflows`.
The frozen Qdrant V1 parent is separately gated by
`--features legacy-qdrant-v1`. Those binary features preserve existing
History/Workflow Types; they do not authorize production cutover.

## Generic operation DAG

`AlegriaOperationDagV1Workflow` on `alegria-operation-dag-v1` accepts one
`OperationDagRunV6` with an immutable `ExecutionPlan`, admitted external
artifacts, versioned generic owner commands, and a pinned route snapshot.
**No** PostgreSQL/Qdrant/Neo4j DTO, SDK, queue, Workflow Type, owner-specific
switch or built-in pure-operation registry participates in this new parent.
The DAG alone defines the operations, checked inputs, Produced bindings and
execution order. V1 is intentionally serial; V2 on the distinct
`alegria-operation-dag-v2` queue performs typed owner-local validation before
starting bounded waves of independent child Workflows.
The canonical business run UUID remains `OperationDagRunV6.plan.scope.run_id`;
the actual parent Temporal Workflow ID must be
`alegria-operation-dag-v1-{business_run_uuid}`. Temporal's separately
server-issued **Run ID** is taken exclusively from the SDK context and is
attested through reciprocal parent/child Histories. It is never substituted
with a business identifier or claimed manifest value.

Every owner — including an owner of a pure transformation — registers the
**same** native Temporal child Workflow Type
`AlegriaOwnerOperationV1Workflow` with the strongly typed SDK boundary
`OwnerInvocationV1 → OwnerResultV1`, but on its **own Task Queue** with its
own credentials, typed Rust decoder, Activities and operator admission for
effects. The generic parent learns the child Task Queue only from a trusted
deployment catalog, never from a caller's manifest. Owner output artifact
port/schema/scope/provenance and checked dispatch identity are validated
before the actual Produced payload becomes input to the next node; each
downstream owner must decode its own concrete Rust input before effects.

`TEMPORAL_OPERATION_DAG_CATALOG_PATH` must name a root-owned, non-group/world
writable, symlink-free **regular file under `/etc/alegria/`**, with root-owned
non-group/world-writable ancestor directories, at most 1 MiB. An operator
installs the versioned JSON file (schema_revision 1, entries of `contract`
and `route`) at deployment time. Its routes are loaded once per process and
every Start History's `pinned_routes` must **exactly** match the registered
catalog subset. A missing, forged or changed route fails closed; no dynamic
unvalidated workflow routing is available. The catalog's asserted `build_id`
does **not** itself instruct Temporal to target a matching Worker Deployment
version; real owner Worker Versioning/routing evidence remains a separate
production requirement.

The provider-free `trusted_owner_catalog` loader is now shared by the Temporal
orchestrator and Site Lab MCP/CLI. Catalog `schema_revision: 2` requires an
additional `parameter_schema` JSON object for each entry, exposed for typed
ChatGPT authoring but **never** treated as runtime admission. Historical v1
catalogs remain valid for pinned Temporal routes without authoring parameters.
The trust path and root-ownership checks are identical; operator-installed
catalog changes require process restart, not a rebuild of the DAG parent.

In a disposable GitHub Actions E2E runner, install the *actual* trusted file
with root privileges and exercise the production trust path. There is no
environment-variable bypass:

```sh
sudo install -d -o root -g root -m 0755 /etc/alegria
sudo install -o root -g root -m 0644 validated-owner-catalog.json \
  /etc/alegria/owner-routes-v1.json
export TEMPORAL_OPERATION_DAG_CATALOG_PATH=/etc/alegria/owner-routes-v1.json
```

The JSON has `schema_revision: 1` and an `entries` array of objects with
`contract` and `route` fields. Generate each contract from its owner-declared
`OperationContract` and derive `route.contract_hash` from `fingerprint()`.
Routes pin the operation, owner, shared Workflow Type, owner-specific Task
Queue and asserted owner build ID. The run's `pinned_routes` must exactly
equal the trusted catalog's sorted snapshot for its operations.

This is a nonproduction pilot. Even a valid DAG has **no** independent
permission to execute external effects: each owner operator must attest
the actual parent/child Temporal Histories, dispatch, scopes and durable
receipt before enabling Activity execution. Owners lacking that new generic
bridge/operator proof must remain **unregistered**, not fall back to older
provider-specific Workflows. V4/V5/Qdrant V1 Histories are frozen.

Both generic modes are disabled by default. Set exactly one of
`ALLOW_OPERATION_DAG_V1=true` (serial) or
`ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION=true` (validated fanout), an exact
source `WORKER_BUILD_ID` and the corresponding queue. Offline replay uses
`--replay-operation-dag-v1-history` or
`--replay-operation-dag-v2-history` with the same trusted catalog path.
Neither mode authorizes cutover.

## Safety and routing

- Exactly one start gate must be true:
  `ALLOW_OPERATION_DAG_V1=true` or
  `ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION=true` in the default binary;
  additional historical opt-ins are accepted only with the relevant binary
  feature (`ALLOW_PURE_DAG_ORCHESTRATOR`,
  `ALLOW_EFFECT_DAG_ORCHESTRATOR_V4`,
  `ALLOW_EFFECT_DAG_ORCHESTRATOR_V5`, `ALLOW_QDRANT_DAG_ORCHESTRATOR_V1`).
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
- Historical site inputs carry immutable plan hash, versioned contracts and admitted
  artifact envelopes. These declarations are **not authenticated producer
  provenance** or operator consent. The original pure Workflow still rejects
  all effects. The checked-effect Workflow validates the entire heterogeneous
  graph, canonical Truth reconciliation contract, typed owner command,
  declared input bytes, scope and checked dispatch identity **before**
  non-retryably failing every effectful request. It never schedules an
  Activity or child Workflow and cannot return a counterfeit PG receipt.

### Frozen site-specific history compatibility

With `legacy-workflows`, Pure V3 runs pure operations without external writes;
the original checked-effect V1 Workflow preflights then rejects every effect.
V4 starts individually admitted PostgreSQL reconciliation children. V5 runs
those independently attested children in dependency order; it rejects
conflicting canonical Truth contexts, reused admissions and unordered effects
in the same Truth scope. Their queues remain
`alegria-pure-dag-v3`, `alegria-effect-dag-v4`, `alegria-effect-dag-v5`;
PG children use `alegria-pg-command-v3`. They must never be polled by generic
workers or silently upgraded to generic Workflow Types.

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

Build and run the **default generic worker** from `app/rust` (after installing
the trusted catalog described above):

```sh
cargo build --locked -p temporal_orchestrator --bin temporal_orchestrator
ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION=true \
  TEMPORAL_ORCHESTRATOR_TASK_QUEUE=alegria-operation-dag-v2 \
  WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_OPERATION_DAG_CATALOG_PATH=/etc/alegria/owner-routes-v1.json \
  target/debug/temporal_orchestrator
```

For historical V3/V4/V5 evaluations, build to a *separate target directory*
so the generic executable is not overwritten:

```sh
cargo build --locked --target-dir target/legacy-workflows \
  -p temporal_orchestrator --bin temporal_orchestrator \
  --features legacy-workflows
ALLOW_PURE_DAG_ORCHESTRATOR=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=true \
  TEMPORAL_ORCHESTRATOR_TASK_QUEUE=alegria-effect-dag-v4 \
  WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/legacy-workflows/debug/temporal_orchestrator
```

To evaluate the independently gated **V5 ordered multi-child** parent:

```sh
ALLOW_PURE_DAG_ORCHESTRATOR=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=false \
  ALLOW_EFFECT_DAG_ORCHESTRATOR_V5=true \
  TEMPORAL_ORCHESTRATOR_TASK_QUEUE=alegria-effect-dag-v5 \
  WORKER_BUILD_ID="$(git rev-parse HEAD)" \
  TEMPORAL_URL=http://localhost:7233 \
  target/legacy-workflows/debug/temporal_orchestrator
```

Offline native History replay (use the binary matching the History queue):

```sh
target/legacy-workflows/debug/temporal_orchestrator --replay-history /path/to/actual-temporal-history.json
target/legacy-workflows/debug/temporal_orchestrator --replay-effect-history /path/to/v4-parent-history.json
target/legacy-workflows/debug/temporal_orchestrator --replay-v5-effect-history /path/to/v5-parent-history.json
target/debug/temporal_orchestrator --replay-operation-dag-v1-history /path/to/generic-parent-history.json
target/debug/temporal_orchestrator --replay-operation-dag-v2-history /path/to/generic-v2-parent-history.json
```

Successful replay emits `CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS`. Malformed,
missing or nondeterministic history fails nonzero. Legacy `alegria-pipeline`
V1/V2 history must continue to be replayed by `temporal_worker`.

The V4 evaluation profile is opt-in only; there is no production cutover.
