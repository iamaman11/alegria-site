# Experimental V3 runtime topology (not production)

`docker-compose.yml` keeps the existing `temporal-worker` as the **only default
production runtime**. The two existing Rust V3 executables are isolated in
independent final image targets and can be started only through the explicit
`v3-evaluation` Compose profile. This changes no legacy task-queue polling or
publication ownership.

| Owner | Build target / binary | Task Queue | Provider SDKs |
| --- | --- | --- | --- |
| Pure workflow | `pure` / `temporal_orchestrator` | `alegria-pure-dag-v3` | None: no PG/Neo4j/Qdrant |
| PostgreSQL effect | `pg` / `temporal_pg_effects` | `alegria-pg-command-v3` + `alegria-pg-effects-v3` | PG only |
| Existing legacy | existing `temporal-worker` | unchanged `alegria-pipeline` | PG/Neo4j/Qdrant; stays default |

Check isolation contracts without any Docker or provider credentials:

```bash
python3 infra/temporal/check_v3_evaluation_topology.py
```

For a disposable evaluation with an already configured Temporal server,
an operator must explicitly supply the *exact* 40-hex source SHA and permit
the pure worker. The PostgreSQL owner additionally needs a separately
provisioned least-privilege role and an explicit V3 enable flag. Leave these
flags unset on ordinary staging/production deployments; the Rust executables
fail closed rather than silently using the legacy admin database URL.

```bash
export WORKER_BUILD_ID='<verified 40-hex source SHA>'
export ALLOW_PURE_DAG_ORCHESTRATOR=true
docker compose --profile v3-evaluation build temporal-orchestrator-v3
docker compose --profile v3-evaluation up -d temporal-orchestrator-v3

# Only after following temporal_pg_effects/README.md for a disposable DB role:
export ALEGRIA_ENABLE_PG_EFFECT_V3=true
export PG_EFFECT_WORKER_DATABASE_URL='<limited-role PostgreSQL connection string>'
docker compose --profile v3-evaluation build temporal-pg-effects-v3
docker compose --profile v3-evaluation up -d temporal-pg-effects-v3
```

These commands are examples for isolated/disposable testing only, **not
approval to run against a live production namespace**. Ensure no other poller
owns the same experimental Task Queue and do not start a second Outbox owner.
No production DAG dispatch to the PG effect exists yet. Starting the V3
workers never authorizes effects by itself; the PostgreSQL operator admission
and receipt fence still veto every call.

## Explicit open criteria before cutover

- Prove at least three real effectful, version-pinned DAGs, durable restart and
  official SDK history replay with typed owner admission before effects.
- Complete raw-writer concurrency fencing, operator governance/tenant RLS,
  approved historical V1/V2 drain and no duplicate Task Queue pollers.
- Measure cold/warm builds per owner, linker units, image size, runtime count,
  startup, CPU/RAM and signed exact-source release; no claim of ≤180 s here.
- `COPY app` still invalidates both Docker builders on unrelated source edits.
  These preview images deliberately do **not** enter the mandatory release or
  default Compose path. They demonstrate binary/runtime shape, **not** final
  independent Docker rebuilds. Use verified precompiled artifacts in a future
  accepted release image path to avoid redundant Rust compilation.

Until those gates pass, the default legacy Worker remains unchanged and the
new owner-specific images must stay experimental. The canonical acceptance
criteria remain in issue #94.
