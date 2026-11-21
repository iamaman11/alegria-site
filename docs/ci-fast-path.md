# CI for the experimental Alegria app

The canonical `.github/workflows/ci.yml` owns fast PR feedback, full Ready-PR
acceptance, and the single signed Worker build. Draft PRs run only the fast
10/10 feedback gate; a Ready-for-review PR also runs the real Rust/PG/Truth
suite and packages its verified source head. For a manual deep diagnostic,
use GitHub Actions → `ci` → **Run workflow**, or locally run
`bash automation/ci_verify.sh all`.

## Measured result (GitHub Actions, 21 November 2026)

| Exact PR CI | Full wall-clock | Slowest work |
| --- | ---: | --- |
| [#223](https://github.com/iamaman11/alegria-site/actions/runs/44915481077) | ~7m 02s | Truth 416s; Rust contracts 368s; architecture 356s; verify 322s |
| [#224](https://github.com/iamaman11/alegria-site/actions/runs/44917627978) | **3m 28s** | RustSec 199s; architecture 189s |
| [#230](https://github.com/iamaman11/alegria-site/actions/runs/44920111633) | **3m 24s** | Rust architecture 184s; RustSec 4s |
| [#39](https://github.com/iamaman11/alegria-site/actions/runs/44934597524) | **8m 33s** | Cold Rust workspace tests 5m 12s, then E2E feature-graph rebuild 2m 20s |
| [#40, first attempt](https://github.com/iamaman11/alegria-site/actions/runs/44935699114) | **6m 46s** | Cold unit tests of all operational crates still exceeded the PR budget |
| [#40, lightweight gate](https://github.com/iamaman11/alegria-site/actions/runs/44936425556) | **1m 14s** | All ten mandatory jobs passed with Rust model regressions and no full-workspace compilation |

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
graph are checked by the `comprehensive` job before release. The five-minute goal must be
confirmed against exact completed CI runs; it is not a cold-run SLA.

## On each pull request

- Real `cargo test -p runtime_models --locked` for the lightweight model
  regressions and `cargo fmt --all -- --check`. Full workspace Rust tests,
  `integration_harness`, and PostgreSQL E2E run on Ready PRs, not on every
  experimental PR. The PR gate by itself **does not certify full Rust compilation**.
- Python/SQLx/SEO/Truth policy contracts and native Windows operator checks.
- RustSec dependency audit, Cloudflare static build, Temporal/PostgreSQL and
  storage/graph container checks, plus source-level Docker boundary checks.
- A single Rust compilation owner; the static contract jobs no longer compile
  or repeatedly bootstrap Rust/Temporal/PostgreSQL.
- The dependency-impact report is produced on PRs, while the optional scoped
  `cargo check` remains a developer command. A direct Rust-only edit no longer
  starts an unpredictable second cold compile in PR CI.

## Release Worker image (same accepted source, one compilation)

After all mandatory fast and deep checks pass on a Ready PR, the `package`
job of the **same `ci.yml` run** builds the real Temporal Worker once,
verifies its runtime tools, signs the archive and uploads an immutable
source-HEAD-specific artifact. The protected main merge gate checks that its
tree is byte-identical to the successful PR source and that the full acceptance
and package jobs passed. Windows staging checks the main merge and both trees,
then downloads the signed **PR artifact**, verifying its source SHA and
`ci.yml` signer. Neither GitHub nor Windows recompiles the accepted Worker
after merge.

Cold image compilation **can still exceed five minutes**: that packaging is
release work rather than a requirement before each experimental PR review.

## Deep verification at Ready for Review

The `comprehensive` job in `ci.yml` retains all 16 real-worker Truth
certification fixtures, Qdrant REST→gRPC transport, the manual LLM lifecycle,
rebuild/extraction Rust-backed checks, semantic fixtures, protocol checks,
and the remaining PostgreSQL regressions.

Its three ordered `contracts`, `product`, and `integration` phases preserve
the same `ci_verify.sh all` coverage while exposing phase durations.
`product` builds the real Truth Workers; `integration` passes their exact
binary paths to manual LLM E2E and reuses the workspace Cargo target for the
whole-page semantic fixture. No synthetic Worker replaces a real test.

It runs automatically when a PR is marked Ready for Review, on subsequent
Ready-PR updates, or via manual dispatch. Draft feedback stays fast; the full
accepted release is **not** claimed to take under five minutes. The most recent
completed full run, [#41](https://github.com/iamaman11/alegria-site/actions/runs/44943520406),
took about 30m 30s with 1,680 Rust cache misses and 445 cache write errors.
Cache and feature-graph reuse remain measured performance work.

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
