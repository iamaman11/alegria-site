# Physical Rust build isolation: A/B decision record

Scope: Issue #94, Draft PR #95. The project GitHub repository remains the
canonical source. This report describes a **compiler experiment**, not an
authorization to switch production processes, Task Queues, or release images.

## Baseline and reproducibility

The local comparison used integrated Git SHA
`8be89d193030fd22c164132eb3fabc24382ba6ba`, tree
`052b95e6dc04a64942c9a2a6d4e4e01a9335aebf`. The integration owner
confirmed this tree matches the Draft PR HEAD `1976bb1`. The PR CI #492
completed successfully but took approximately **386 seconds**, versus the
180-second target. This timing is a reported CI observation, not measured by
the local manifest script.

Reproduce the static comparison with no Rust toolchain:

```bash
python3 automation/compare_physical_build_options.py
python3 automation/check_physical_build_options.py
```

The report labels its conclusion `MANIFEST_ONLY_UNCOMPILED`; it does not imply
that `rustc` executed or that the proposed process exists.

Once a **real**, committed `temporal_orchestrator` Cargo package exists and a
locked Rust toolchain is available, the strict compiler comparison is:

```bash
python3 automation/compare_physical_build_options.py --compiler
python3 automation/compare_physical_build_options.py --compiler-owners
```

`--compiler` exports exact `HEAD` to a temporary source tree, resolves real
`cargo metadata --locked`, and collects compiler-artifact JSON from two real
binary builds. `--compiler-owners` additionally makes four independent,
reversible, inert owner-source changes **only inside the temporary export**:

1. `postgres_store/src/lib.rs`;
2. `graph_neo4j/src/lib.rs`;
3. `vector_qdrant/src/lib.rs`;
4. `temporal_gateway/src/lib.rs` (positive control: both binaries should relink).

Each perturbation records non-fresh `rustc` units, binary-target relinks, mtime,
SHA-256, elapsed time, then restores its source and compiler baseline before
the next owner. A dirty tracked Rust source, missing real B package, missing
Cargo/rustc, inconsistent metadata, incomplete compiler artifacts, missing
owner rebuild, or unexpected binary relink causes **exit 2**, never a synthetic
pass. The experiment does not modify the active worktree or its Cargo target.
To make `fresh: false` meaningful, it disables `RUSTC_WRAPPER`,
`RUSTC_WORKSPACE_WRAPPER`, both Cargo-config environment equivalents, any
`SCCACHE_*` environment, and `CARGO_INCREMENTAL`; it forces the resolved
`rustc` executable and an isolated `CARGO_TARGET_DIR`. Explicit Cargo CLI
configuration also disables wrappers from Cargo home/workspace configuration.
Thus sccache hits cannot be misreported as actual compiler executions. The
study additionally requires matching executable-file mtime changes when a
binary is reported relinked; a different SHA-256 is **not** required because
recompiling identical semantics can reproduce identical binary bytes. Mtime
is filesystem evidence of a write, not an independent trace of the linker
process, and the study does not claim otherwise.
The optional CI label is `physical-ab-probe`, inside the existing workflow;
ordinary PR runs do not pay for this expensive study.

## A/B topology observed from Cargo manifests

The local graph scans all 27 explicit workspace members and their production
path dependencies (including optional and target-specific declarations). It
does not pretend that all manifest edges are compiled for a given target.

| Candidate | Real executable now | Internal production dependency closure | Providers in closure | Physical conclusion |
| --- | --- | ---: | --- | --- |
| A: existing `temporal_worker` | Yes | 19 crates excluding itself | PG, Neo4j, Qdrant, `runtime_composition`, `infrastructure` | Mixed Worker; cannot isolate provider recompiles |
| A′: move Rust modules/bins *inside the same package* | No separate package | Same 19 | Same | Can improve source boundaries, **not** Cargo package build isolation |
| B: separate `temporal_orchestrator` | **No — hypothetical** | 9 crates in proposed provider-free manifest | None | Plausible isolated orchestrator, but uncompiled and undeployed |

The four current source-owner reverse closures, **including the owner itself**:

| Changed owner | Production-path dependent workspace packages | Consumers |
| --- | ---: | --- |
| `postgres_store` | 8 | `cli_tools`, `infrastructure`, `integration_harness`, `outbox_worker`, `reconcile`, `runtime_composition`, `temporal_worker` |
| `graph_neo4j` | 6 | `analytics_svc`, `integration_harness`, `outbox_worker`, `runtime_composition`, `temporal_worker` |
| `vector_qdrant` | 5 | `integration_harness`, `outbox_worker`, `runtime_composition`, `temporal_worker` |
| `temporal_gateway` | 5 | `cli_tools`, `integration_harness`, `reconcile`, `temporal_worker` |

These are potential Cargo rebuild edges, **not** observed relink counts.
The older monolithic packages continue to exist after merely adding B, so
the global reverse closures and `cargo test --workspace` remain broad.

## Why the runtime split is not a manifest-only change

Current `services/temporal/src/main.rs` connects PostgreSQL, recovers
interrupted static publications, starts a navigation listener, and registers
`AlegriaActivities` alongside Workflows on `alegria-pipeline`. The package also
owns provider-heavy `temporal_starter` and `whole_page_semantic_report` bins.
Its Dockerfile compiles `temporal_worker`, `temporal_starter`, and `cli_tools`
together. A new bin in that same `Cargo.toml` would still share its provider
dependencies and Docker invalidation boundary.

Every helper in `workflows/runtime.rs` currently creates `ActivityOptions`
without `task_queue`, so Activity tasks inherit their Workflow queue. Starting
two workers with *different registered Activity sets* on `alegria-pipeline`
would be unsafe: a task may be picked up by a worker that cannot execute it.
Re-routing existing V1/V2 Activity commands could also break replay of
durable Temporal histories.

For a meaningful B vertical slice, use a **new opt-in V3 Workflow queue**
`alegria-orchestration-v3`, explicitly routed owner Activity queues, and a
real, scoped PostgreSQL Activity returning a verified receipt. The SDK pinned
at Temporal 1.1.2 supports cross-queue `UntypedActivity` + `RawValue` and
`ActivityOptions.task_queue`. The V1/V2 workers, old queue, publication
recovery, history replay, and canonical starter remain untouched until drain.
Task Queue routing is not effect authority: owner admission must check plan
hash, registered contract, run scope, idempotency/fence, and actual PG state.

`runtime_composition` currently joins PostgreSQL + Graph + Vector. The legacy
`projection_sync_impl` Activity itself claims leased Outbox rows and dispatches
Graph/Vector effects, in addition to `outbox_worker` doing delivery. Full
provider-owned execution therefore requires moving V3 projection delivery to
target-scoped workers and making V3 Workflow wait for durable projection ACKs.
Preserve the verified-rule Qdrant uncertainty marker and PostgreSQL advisory
fence across external RPC; never retry unknown outcomes automatically. The
existing unscoped Outbox worker must be drained/stopped before target-scoped
executors compete for the same leases.

## Trade-offs and acceptance gate

| Option | Build benefit | Operational cost | Decision now |
| --- | --- | --- | --- |
| A / A′ modular monolith | No proven provider relink reduction; one Worker deployment | Lowest migration/replay risk | Maintain existing production |
| B new orchestrator only | Its own binary can avoid PG/Graph/Vector builds | New Task Queue, worker image, service and metrics; old monolith still built | **Do not count as 180s CI fix** |
| B + provider-owned Activities/Outbox | Potential independent compilers and releases by owner | Queues, separate secrets, idempotency fences, failure/retry/HITL, old-history drain | Target architecture after measured V3 E2E |

Before enabling V3 beyond Draft, require:

- Real `cargo metadata` and `--compiler-owners` output at one exact committed
  SHA, including owner `rustc` units and binary relink identities; three
  provider-only changes must *not* relink the provider-free orchestrator, while
  the shared Temporal gateway change must relink it.
- Interpret the uncached, wrapper-free owner probe as a graph/relink isolation
  experiment, **not** a representative production CI wall-time benchmark.
  The normal CI may still use sccache; record its hits/misses separately.
- Cold and warm `cargo build --locked --timings --message-format=json` and
  independent Docker layer measurements. Account for Cargo target cache,
  sccache hits, registry sources and dependency skeleton invalidation.
- Three fully green, exact-SHA required CI runs with wall clock at or below
  **180 seconds**, not merely lower static Cargo closure counts.
- Old V1/V2 history replay, V3 cross-queue completion, worker outage/restart,
  timed-out/ambiguous Activities, PG fencing, Outbox concurrent claims,
  Qdrant uncertain-result quarantine, Graph/Vector readback and HITL E2E.
- A deployment/runbook rollback preserving legacy Task Queue polling and
  native Cloudflare publication recovery, with a no-side-effect failure mode
  if any V3 owner executor or required provider is unavailable.

**Current verdict: evidence suffices to reject cosmetic in-package splitting,
but does not yet justify implementing a new production V3 runtime.** Creating
V3 solely to turn a hypothetical 19→9 into a green manifest check would add
process and CI cost without proven functionality or SLA improvement.
