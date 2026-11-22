# CI for the experimental Alegria app

The single canonical `.github/workflows/ci.yml` owns affected-crate feedback,
real-service acceptance and the signed Worker artifact. Draft PRs run nine
independent fast owners plus the mandatory `CI` gate; a deep-impact Draft
also tests the Rust runtime model. Ready PRs run the required real Rust/PG,
Temporal and Truth suites **when source impact requires them**.

For an unconditional full diagnostic use GitHub Actions → `ci` → **Run workflow**,
or locally run `bash automation/ci_verify.sh all`.

## Measured result (GitHub Actions, 21–22 November 2026)

| Full PR CI | Wall-clock | Outcome |
| --- | ---: | --- |
| [#44952932021](https://github.com/iamaman11/alegria-site/actions/runs/44952932021) | **12m 46s** | Initial full baseline; 12/12 PASS |
| [#44956626539](https://github.com/iamaman11/alegria-site/actions/runs/44956626539) | **7m 07s** | Independent Rust, integration, Truth and package; 14/14 PASS |
| [#44960887942](https://github.com/iamaman11/alegria-site/actions/runs/44960887942) | **6m 29s** | Full Python Product policy in fast owner; 14/14 PASS |
| [#44962223447](https://github.com/iamaman11/alegria-site/actions/runs/44962223447) | **5m 38s** | Reused workspace-owned Quality/SERP Cargo tests; 14/14 PASS |
| [#44962860262](https://github.com/iamaman11/alegria-site/actions/runs/44962860262) | **5m 13s** | Parallel deep DAG, no unused hosted E2E PostgreSQL; 14/14 PASS |
| [#44963430262](https://github.com/iamaman11/alegria-site/actions/runs/44963430262) | **5m 44s** | Same native code, Truth Workers took 133s instead of 95s; 14/14 PASS |

These are exact completed PR runs, not an established cold-cache SLA. Issue
[#37](https://github.com/iamaman11/alegria-site/issues/37) keeps the **≤5m
full CI + three warm/one cold** acceptance open until measured. Historical
28m52s and 30m+ runs and detailed cache experiments are recorded there.
The critical path varies between the real `integration-e2e` (4m46s in the
first run) and `comprehensive` (5m18s in the second). Even 99.62% sccache
hits did not guarantee a ≤5m full run; the signed image still took only 50s.

## On each pull request

- `cargo fmt --all -- --check` and native Cargo metadata impact on every PR.
  An isolated Rust change runs one `cargo test --locked -p ...` covering the
  changed crate and all its reverse consumers. Core runtime changes still
  require the full real-service suite. Support-only changes compile no Rust.
- Python/SQLx/SEO/Truth policy contracts and native Windows operator checks.
- RustSec dependency audit, Cloudflare static build, Temporal/PostgreSQL and
  storage/graph container checks, plus source-level Docker boundary checks.
- One Cargo *registry* cache writer; independent deep owners compile into their
  own runner targets. The fast Product/contract owners do not invoke Cargo.
- The dependency-impact report selects actual PR Cargo scope; optional scoped
  `cargo check` remains a local developer command. Unknown files conservatively
  expand to full verification, without misclassifying known support-only files.

## Release Worker image (same accepted source, one compilation)

After the mandatory fast `CI` gate passes, the `package` job builds, validates,
signs and uploads the real Temporal Worker image **in parallel** with the deep
jobs. Building early does **not** accept or deploy the artifact: protected-main
provenance requires all 14 mandatory jobs on the exact PR source SHA to pass.
Windows staging checks the merge/source trees, the same signed PR artifact and
its signer before installation. No Worker rebuild occurs after merge.

The package job takes about 50s on the measured Ready PR, not the CI critical
path. A cold Docker image compilation can still vary independently.

## Deep verification at Ready for Review

Ready PRs with deep source impact start all three owners after the lightweight
`rust-contract-smokes` owner has selected the exact-source Cargo impact scope,
in parallel with `architecture-and-rust` and the Docker infrastructure checks.
Every fast owner, **including architecture**, must still pass the required
`CI` gate for packaging and release acceptance. On a failed fast owner, deep
work may be wasted, but all mandatory acceptance remains fail-closed.

- `rust-regressions`: full `cargo test --workspace --locked`, retained
  PostgreSQL/SQLx regressions and Rust-backed contracts. It alone needs a
  hosted disposable fixed-port PostgreSQL for the scoped Truth SQLx test.
- `integration-e2e`: real manual-LLM/Temporal restart, replay, full synthetic
  harness, Qdrant REST→gRPC transport and semantic fixture. Qdrant reuses the
  already compiled `integration_harness` E2E test binary. The harness starts
  isolated PostgreSQL/Temporal/Qdrant containers; Quality/SERP Cargo tests run
  in the mandatory workspace owner, not here again.
- `comprehensive`: all 16 real Temporal/Outbox Truth certification fixtures,
  with both native Workers built in one Cargo target. It reuses the mandatory
  integration sibling's Qdrant transport proof in CI; a standalone local Truth
  gate still runs that real test itself.
  The harness starts its own PostgreSQL, Qdrant, Neo4j and Temporal containers.

The full deep suite remains available through manual dispatch. Support-only
PRs report `NOT_AFFECTED_BY_SOURCE_CHANGE` and skip unnecessary Rust builds.

## Dependency strategy

- Cargo already uses 20 workspace members. Another crate split only helps if
  it actually removes a heavy transitive dependency from a frequently tested
  package; it does not improve a four-way rebuild of the same graph.
- Preserve one owner for Cargo registry cache writes and use `sccache` in the
  Rust owner. Avoid archiving multi-gigabyte `target/` directories.
- RustSec installs the official version-pinned, SHA-256-verified 6-MB
  `cargo-audit` executable, avoiding repeated 196-second tool compilation.
- Disposable Rust integration databases use the same PostgreSQL 18.7 major
  and patch version as the Docker Compose runtime, avoiding version drift.
- Keep reproducible `Cargo.lock` and `--locked` for compilation. Stable
  updates that change the lockfile require a separate resolve/compile pass.
- The bare `rig-core`/`rig-vertexai` re-export adapters were removed: the
  application uses its own reqwest-based Vertex path. These unused dependencies
  pulled duplicate Rig versions and Google Cloud Auth into the build graph.
  The dependency lock was regenerated on GitHub after removing these unused
  adapters; no separate TLS provider migration is assumed.
- `temporalio-sdk-core 0.10.2` requires `temporalio-client 1.1.x`, so the
  Temporal SDK/client/common group stays at compatible `1.1.2` even though a
  newer client exists. The rejected `1.2.0` resolution did not compile.
- Dependency refresh is now manually dispatched; changes to the lockfile must
  pass the same PR CI rather than being silently pushed by an automatic job.
- The `cf` CLI is still in a release-candidate cycle, and its `v1` Build Output
  differs from the current deployment artifact. Do not silently replace the
  pinned beta with an incompatible prerelease during a fast CI optimization.
