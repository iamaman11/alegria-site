# Experimental V3 runtime topology (not production)

`docker-compose.yml` keeps the existing `temporal-worker` as the **only default
Temporal runtime**. Two native Rust executables have independent final image
targets. Three opt-in service configurations run those executables on disjoint
Task Queues: V3 pure and PG under `v3-evaluation`; the V4 admitted-effect
parent under the separately explicit `v4-effect-evaluation` profile. V3 and
V4 orchestrators reuse one physical `temporal_orchestrator` image but register
different Workflows in mutually exclusive runtime modes. This changes no
legacy task-queue polling or publication ownership.

| Owner | Build target / binary | Task Queue | Provider SDKs |
| --- | --- | --- | --- |
| Pure workflow | `pure` / `temporal_orchestrator` | `alegria-pure-dag-v3` | None: no PG/Neo4j/Qdrant |
| V4 admitted effect DAG parent | `pure` / `temporal_orchestrator` | `alegria-effect-dag-v4` | None: no PG/Neo4j/Qdrant; Temporal child dispatch only |
| PostgreSQL effect | `pg` / `temporal_pg_effects` | `alegria-pg-command-v3` + `alegria-pg-effects-v3` | PG only |
| Existing legacy | existing `temporal-worker` | unchanged `alegria-pipeline` | PG/Neo4j/Qdrant; stays default |

Check isolation contracts without any Docker or provider credentials:

```bash
python3 infra/temporal/check_v3_evaluation_topology.py
```

For a disposable evaluation with an already configured Temporal server,
build **two independent source-scoped contexts from the same clean, exact
committed SHA**. The context generator uses the Cargo dependency closure for
each owner: nonowner Rust implementation files are omitted, and stable
manifest/stub skeletons populate the dependency compilation layer. A dirty
checkout is deliberately refused; a release job must build from its accepted
source revision, not an uncommitted working tree. Neither context is a
production release artifact or a replacement for artifact signing.

```bash
export WORKER_BUILD_ID="$(git rev-parse HEAD)"
export V3_PURE_BUILD_CONTEXT="$(mktemp -d)/temporal-pure"
export V3_PG_BUILD_CONTEXT="$(mktemp -d)/temporal-pg"
python3 automation/prepare_worker_build_context.py \
  --source-sha "$WORKER_BUILD_ID" \
  --destination "$V3_PURE_BUILD_CONTEXT" \
  --image-owner temporal-orchestrator-v3
python3 automation/prepare_worker_build_context.py \
  --source-sha "$WORKER_BUILD_ID" \
  --destination "$V3_PG_BUILD_CONTEXT" \
  --image-owner temporal-pg-effects-v3

# Explicit disposable, single-poller evaluation only:
export ALLOW_PURE_DAG_ORCHESTRATOR=true
docker compose --profile v3-evaluation build temporal-orchestrator-v3
docker compose --profile v3-evaluation up -d temporal-orchestrator-v3
```

The PostgreSQL owner additionally requires a separately provisioned
least-privilege role, a secret named `pg_effect_worker_dsn` created *outside*
the repository and Compose `.env`, and an explicit V3 enable flag. The DSN
must target that role, not `postgres`, the legacy `DATABASE_URL`, or a shared
privileged service account. Compose requests UID/GID `65532:65532` and `0400`
for `/run/secrets/pg_effect_worker_dsn`; the binary rejects symlinks,
nonregular files, and group/world-readable mode. **Verify the selected secret
backend actually honors ownership and mode** before evaluation: some
file-backed Compose secret implementations do not apply these attributes,
in which case startup intentionally fails closed. Do not weaken the file
permissions to make an unsupported secret backend work. A Docker or
orchestrator administrator must arrange external secret creation; no
credential value appears in these commands.

```bash
# Only after provisioning the restricted DB role and external secret:
export ALEGRIA_ENABLE_PG_EFFECT_V3=true
docker compose --profile v3-evaluation build temporal-pg-effects-v3
docker compose --profile v3-evaluation up -d temporal-pg-effects-v3
```

The V4 parent uses the **same generated pure-owner build context**, but a
**different Task Queue** and an independently gated Workflow registration.
The Rust mode gates must prevent `AlegriaAdmittedEffectDagV2Workflow` from
polling the V3 pure queue, or the operator's server-history evidence would
disagree with the started Workflows. Start it **only after** the V3 PG child
worker is proven healthy on its experimental queues, no other V4 parent
poller is active, and the V4 operator issuance and acceptance tests pass.

```bash
# Still disposable-only: disabled until the explicit V4 flag is true.
export ALLOW_EFFECT_DAG_ORCHESTRATOR_V4=true
docker compose --profile v4-effect-evaluation build temporal-effect-dag-v4
docker compose --profile v4-effect-evaluation up -d temporal-effect-dag-v4
```

The V4 service pins `ALLOW_PURE_DAG_ORCHESTRATOR=false` in Compose and
requires `WORKER_BUILD_ID` to name the same accepted exact source SHA. The
V3 pure service retains its original dedicated profile, flag and queue. V4
does not load PostgreSQL secrets or assume direct owner Activity authority.

These commands are examples for isolated/disposable testing only, **not
approval to run against a live production namespace**. Ensure no other poller
owns the same experimental Task Queue, and do not start a second Outbox owner.
The PG activity is currently restricted to one typed reconciliation
operation, not arbitrary side effects. Starting any of these workers never
authorizes effects by itself; the PostgreSQL operator admission and receipt
fence still veto every call. No default Compose workload is routed to a V3/V4
Task Queue; no traffic migration, existing-history drain or rollback is
automated here.

## Explicit open criteria before cutover

- Prove at least three real effectful, version-pinned DAGs, durable restart and
  official SDK history replay with typed owner admission before effects.
- Complete raw-writer concurrency fencing, operator governance/tenant RLS,
  approved historical V1/V2 drain and no duplicate Task Queue pollers.
- Measure cold/warm builds per owner, linker units, image size, runtime count,
  startup, CPU/RAM and signed exact-source release; no claim of ≤180 s here.
- The owner-scoped build context now removes unrelated implementation sources
  before `COPY app`, while the Cargo manifest/stub layer can be reused across
  owner-only Rust source edits. This is **not a measured** image rebuild or
  release: the full relevant dependency closure still compiles in each
  independent image build, and cold/warm timings and signed runtime artifacts
  remain open. These preview images deliberately do **not** enter the
  mandatory release or default Compose path.

Until those gates pass, the default legacy Worker remains unchanged and the
new owner-specific images must stay experimental. The canonical acceptance
criteria remain in issue #94.
