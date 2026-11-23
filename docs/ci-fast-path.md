# CI for the experimental Alegria app

The pull-request gate is intentionally a **fast feedback** gate; deep runtime
certification runs on demand through GitHub Actions → `ci` →
**Run workflow** (`comprehensive` job), or locally with
`bash automation/ci_verify.sh all`. All CI jobs are defined in
`.github/workflows/ci.yml`; release-image packaging remains independent.

## Measured result (GitHub Actions, 21 November 2026)

| Exact PR CI | Full wall-clock | Slowest work |
| --- | ---: | --- |
| [#223](https://github.com/iamaman11/alegria-site/actions/runs/44915481077) | ~7m 02s | Truth 416s; Rust contracts 368s; architecture 356s; verify 322s |
| [#224](https://github.com/iamaman11/alegria-site/actions/runs/44917627978) | **3m 28s** | RustSec 199s; architecture 189s |
| [#230](https://github.com/iamaman11/alegria-site/actions/runs/44920111633) | **3m 24s** | Rust architecture 184s; RustSec 4s |
| [#39](https://github.com/iamaman11/alegria-site/actions/runs/44934597524) | **8m 33s** | Cold Rust workspace tests 5m 12s, then E2E feature-graph rebuild 2m 20s |
| [#40, first attempt](https://github.com/iamaman11/alegria-site/actions/runs/44935699114) | **6m 46s** | Cold unit tests of all operational crates still exceeded the PR budget |
| [#40, lightweight gate](https://github.com/iamaman11/alegria-site/actions/runs/44936425556) | **1m 14s** | All ten mandatory jobs passed with Rust model regressions and no full-workspace compilation |
| [#62, new PR canary](https://github.com/iamaman11/alegria-site/actions/runs/45238064039) | **9m 13s** | Scoped integration harness, 2 real CMS E2E and 2 Temporal E2E tests; 65.47% compiler cache hit rate |

The three earlier runs passed every then-required job. After the version refresh, a cold
Docker rebuild revealed a separate release-compilation bottleneck, so Docker
image packaging has been removed from the PR critical path. These timings are
**warm-cache** observations, not a guarantee for a cold Rust rebuild. Run #230
retained full workspace tests and one real PostgreSQL e2e regression while
the `sccache` compiler cache reported 100% hits.

Run #39 exposed the opposite case: `sccache` had no Rust compiler hits, and
the second E2E feature graph made the cold PR gate exceed five minutes.
Removing the E2E rebuild was insufficient (#40). The regular prototype PR
gate now exercises lightweight Rust model regressions, source/format/contract
checks and operational container checks; the full workspace and E2E feature
graph are available in the manual `comprehensive` job before release.
The current impact-selected PR Rust path can invoke real PostgreSQL and
Temporal tests; #62 exceeded the five-minute target. Do not treat earlier
lightweight-PR runs as cold-run proof.

## On each pull request

- Real Rust model regressions and `cargo fmt --all -- --check`.
  The impact-selected Rust owner checks dependent crates and selected
  PostgreSQL/Temporal E2E; whole-workspace tests and *all* fixtures remain
  manual. The PR gate does not certify every unselected feature combination.
- Python/SQLx/SEO/Truth policy contracts and native Windows operator checks.
- RustSec dependency audit, Cloudflare static build, Temporal/PostgreSQL and
  storage/graph container checks, plus source-level Docker boundary checks.
- A single Rust compilation owner; the static contract jobs no longer compile
  or repeatedly bootstrap Rust/Temporal/PostgreSQL.
- The dependency-impact report is produced on PRs and used by the single Rust
  compilation owner. No other PR job compiles the same broad Cargo graph.

## Release Worker image (independent of PR feedback)

On an accepted push to `main`, `worker-image.yml` separately builds the real
Temporal Worker Docker image, runs its native runtime smoke, signs the archive,
and publishes the source-specific artifact. A manual dispatch can also package
the exact HEAD of the selected GitHub branch. Windows staging requires both a
successful `ci` run and a successful `worker-image` run for the requested
commit; it downloads and verifies the signed archive without compiling on Windows.

Cold image compilation **can still exceed five minutes**: that packaging is
release work rather than a requirement before each experimental PR review.

## Deep verification on demand

The manually dispatched `comprehensive` job in `ci.yml` retains all 16 real-worker Truth
certification fixtures, Qdrant REST→gRPC transport, the manual LLM lifecycle,
rebuild/extraction Rust-backed checks, semantic fixtures, protocol checks,
and the remaining PostgreSQL regressions.

Run it before a release, or after changes to Temporal workflow behavior,
truth admission, publishing, or SQL schema. These checks were **moved out of
the PR critical path**, not deleted or represented as having run on PR.

## Trusted compiler-cache warm-up

The `rust-cache-prime` job also lives in `ci.yml`. Only successful,
provenance-checked main pushes may run it, and it performs heavy work only if
Rust compilation inputs changed. Changes limited to CI files or the
`integration_harness/src/tests/` regression sources do not launch another
long compiler build. Mixed changes including production Rust still prime.
It prepares the three Rust CI profiles but
does not execute tests. PRs read the shared compiler cache without uploading
PR-local compiler objects. This is *not* proof that a cold PR will finish
within five minutes; exact new-PR timings and cache errors must be measured.

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
