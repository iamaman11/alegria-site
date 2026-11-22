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
| [#44964187468](https://github.com/iamaman11/alegria-site/actions/runs/44964187468) | **5m 15s** | Qdrant transport reused existing integration test target; 14/14 PASS |
| [#44964851388](https://github.com/iamaman11/alegria-site/actions/runs/44964851388) | **4m 32s** | Deep jobs start after 10s Rust contracts; 14/14 PASS |
| [#44965308197](https://github.com/iamaman11/alegria-site/actions/runs/44965308197) | **5m 19s** | Identical Rust code; compiler-cache misses regress; 14/14 PASS |
| [#44967454786](https://github.com/iamaman11/alegria-site/actions/runs/44967454786) | **4m 48s** | Cargo-native compile timings recorded, full integration 4m33s; 14/14 PASS |
| [#44968309924](https://github.com/iamaman11/alegria-site/actions/runs/44968309924) | **5m 04s** | Parallel real test services speed up readiness; Rust cache writes still fail; 14/14 PASS |
| [#44968816771](https://github.com/iamaman11/alegria-site/actions/runs/44968816771) | **4m 56s** | One-run GHA cache failure diagnosis; 409 and 429 confirmed, 14/14 PASS |

These are exact completed PR runs, not an established cold-cache SLA. Issue
[#37](https://github.com/iamaman11/alegria-site/issues/37) keeps the **≤5m
full CI + three warm/one cold** acceptance open until measured. Historical
28m52s and 30m+ runs and detailed cache experiments are recorded there.
The critical path varies between the real `integration-e2e` (4m46s in the
first run) and `comprehensive` (5m18s in the second). Even 99.62% sccache
hits did not guarantee a ≤5m full run; the signed image still took only 50s.
The latest diagnostic run also met five minutes without removing tests, but
repeatable cold-cache performance still requires the full Issue #37 sample.

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
  isolated PostgreSQL/Temporal/Qdrant/Neo4j containers concurrently because
  their readiness and data setup are independent; Quality/SERP Cargo tests run
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
- The GitHub Actions `sccache` backend has reproducible write losses: the
  5m19s run logged 127–180 misses per deep Rust owner and almost as many write
  errors. In the 4m48s run, **every cache write** in the three deep jobs failed
  (52/52, 76/76, 78/78), despite 92–95% cache hits. GitHub limits uploads
  to 200/min; the
  upstream [retry fix](https://github.com/mozilla/sccache/pull/2911) addresses
  transient rate limiting.
  In the next 5m04s run, `rust-regressions` succeeded in only **1 of 220**
  writes; integration had 102/102 write errors. The real Temporal/Neo4j/Qdrant/
  PostgreSQL startup improved 22.4s→18.0s, and manual Temporal restart test
  56.1s→52.8s. One disposable, runner-local diagnostic in the next full
  [run #44968816771](https://github.com/iamaman11/alegria-site/actions/runs/44968816771)
  confirmed **89 HTTP 409 conflicting writes and 35 HTTP 429 rate limits** in
  integration (124/124 failed writes), without uploading the raw debug log.
  The permanent native fix assigns one GHA compiler-cache writer to the full
  `rust-regressions` workspace owner; `integration-e2e` and `comprehensive`
  retain the same real tests and run in parallel, but read the shared cache
  without competing writes. The temporary debug instrumentation was removed.
  That fix was merged after the latest stable sccache v0.19.0 release
  (24 October 2026); do not assume a version-only upgrade repairs this.
  Cargo `--timings` in the deep evidence
  independently measured `aws-lc-sys` C build scripts at **45s (Truth)** and
  **60s (integration)**. Moving the application from AWS-LC to `ring` could
  remove that work but changes the runtime TLS provider and is not part of a
  CI-only optimization.
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
