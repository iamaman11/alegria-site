# CI for the experimental Alegria app

The pull-request gate is intentionally a **fast feedback** gate; deep runtime
certification runs on demand through GitHub Actions → `ci` →
**Run workflow** (`operation: comprehensive`), or locally with
`bash automation/ci_verify.sh all`. All PR checks, optional checks,
accepted-main packaging and dependency maintenance are defined in
`.github/workflows/ci.yml`; packaging is not part of PR feedback.

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
| [#70, exact PR](https://github.com/iamaman11/alegria-site/actions/runs/45265863563) | **4m 49s** | Dependency-scoped Worker Docker context; PR gate passed, release not built |
| [#71, exact PR](https://github.com/iamaman11/alegria-site/actions/runs/45267484566) | **4m 57s** | Main release gating fixes; architecture-and-rust 281s, other owners ≤54s |

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
- Source-only crawler, protobuf and run-report checks run once in `verify`.
  The former parallel `rust-contract-smokes` job was removed; real Rust-backed
  deep checks still execute in `comprehensive`, never as a synthetic PR PASS.
  Cargo metadata and its toolchain/cache setup remain only with the actual
  Rust impact/compilation owner; Python-only `verify` needs no Rust setup.
- The dependency-impact report is produced on PRs and used by the single Rust
  compilation owner. No other PR job compiles the same broad Cargo graph.

## Release Worker image (inside the same CI workflow)

The repository has **one** GitHub Actions workflow: `.github/workflows/ci.yml`.
On an accepted push to `main`, its `CI` gate verifies the merge against the
exact successful PR source; only then does `select-image` decide whether to run
`package`. When the shared owner registry selects the Temporal Worker image,
`package` builds the real Docker image, runs its runtime smoke, signs the
archive, and publishes an exact-source artifact **in the same CI run**.
Changes limited to an explicit allowlist of CI/Windows operator maintenance
do not rebuild unchanged runtime bytes, even though their PR verification
remains fail-closed. Mixed or unknown source changes retain image selection.
The maintenance allowlist and release-decision function now belong to the same
`ci_impact.py` registry as the Cargo and image graph, not to copied YAML
conditions. CI control changes can require full PR tests while leaving the
shipped image unchanged; a mixed CI + Worker input still requires packaging.

GitHub can briefly report no associated PR for a merge immediately after the
`main` push. The exact-head provenance gate retries that one association lookup
for at most 30 seconds, then fails closed. It never accepts a mismatching PR,
different source tree, failed or absent PR job as release proof. The original
main run [#45268441516](https://github.com/iamaman11/alegria-site/actions/runs/45268441516)
failed before image selection because of this indexing window; a fixed main
push must still prove the complete accepted-main release path.

Before Buildx, automation/prepare_worker_build_context.py uses the existing
ci_impact.py Cargo graph and Worker image roots to create a reduced exact-source
Docker context. Cargo still sees every original workspace manifest and the
unchanged Cargo.lock; unrelated workspace members get only inert source
placeholders. Only live Worker/CLI consumer Rust sources, SQLx offline data,
protobuf inputs and delivered Python/Cloudflare assets are copied. Independent
changes to analytics_svc, gsc_sync, integration_harness, outbox_worker or
reconcile therefore cannot invalidate the release compilation layer unless
the Cargo dependency graph changes. BuildKit's existing GHA cache is reused;
no second impact selector, release workflow or cargo-chef was added.
The Docker release also isolates the Cloudflare `npm install` stage behind
its package manifest: source-only, website asset-only and source-SHA-only
changes cannot invalidate that dependency install. The GHA Buildx exporter
uses low-level zstd compression while retaining intermediate Rust dependency
layers (`mode=max`) when manifests/lock, the Docker recipe, or unknown native
inputs change. Ordinary Rust implementation edits and static assets reuse that
existing cache without exporting it again. A later assets-only release may
recompile the exact current Rust source; its runtime, SHA, archive and
attestation checks remain mandatory. Main CI #450 measured 134.8s for the
dependency skeleton, 39.2s for the real Rust source build, and 169.2s for the
full Buildx cache export (61.8s preparation + 107.4s upload). The new export
policy targets this measured critical path; verify the five-minute goal on a
real accepted-main run.

The Dockerfile now requires worker-context.json produced by that exact-SHA
preparation. For local reproduction, prepare the context from a clean checked
out commit using Cargo and build with that directory as context, specifying
the repository Dockerfile using -f. The repository root is intentionally not
a release context. PR CI checks that the prepared workspace still resolves
with cargo metadata --locked; it does not pay for a release build.
The five-minute target still requires real accepted-main warm-cache timings.

For an explicit rebuild, dispatch `ci` on **main** with
`operation: worker-image`: it re-verifies the accepted-PR provenance before
packaging, and never packages an arbitrary unreviewed branch. Windows staging
requires a successful `ci` run containing a successful `package` job for the
exact commit; it verifies the `ci.yml` signer and loads the archive without
compiling on Windows. A successful CI run with `package` skipped does **not**
certify a deployable Worker artifact for that SHA.

Cold image compilation **can still exceed five minutes**: that packaging is
release work rather than a requirement before each experimental PR review.

## Deep verification on demand

Dispatch `ci` with `operation: comprehensive` (the default); that job retains all 16 real-worker Truth
certification fixtures, Qdrant REST→gRPC transport, the manual LLM lifecycle,
rebuild/extraction Rust-backed checks, semantic fixtures, protocol checks,
and the remaining PostgreSQL regressions.

Run it before a release, or after changes to Temporal workflow behavior,
truth admission, publishing, or SQL schema. These checks were **moved out of
the PR critical path**, not deleted or represented as having run on PR.

For an isolated dependency lock refresh, dispatch `ci` with
`operation: dependency-refresh` on a non-`main` review branch.
This operation never writes directly to `main`; the resulting lockfile
candidate still enters through the ordinary PR acceptance gate.

## Trusted compiler-cache warm-up

The `rust-cache-prime` job also lives in `ci.yml`. Only successful,
provenance-checked main pushes may run it, and it performs heavy work only if
Rust compilation inputs changed. Changes limited to CI files or the
`integration_harness/src/tests/` regression sources do not launch another
long compiler build. Mixed changes including production Rust still prime.
It now reuses the **same exact-source Cargo graph, reverse consumers and
integration-suite selection** as the PR Rust owner: affected crate tests are
compiled without running them, reverse consumers are checked, and the E2E
feature profile is compiled only when an affected live suite needs it. For
Temporal/Truth suites that spawn real workers, the corresponding debug binaries
are prepared once, using the suite's existing shared command. A leaf-crate
change does not unconditionally rebuild all three harness profiles. These
debug builds are not release artifacts; Docker builds its own release image.
PRs read the shared compiler cache without uploading PR-local compiler
objects. This is *not* proof that a cold PR will finish
within five minutes; exact new-PR timings and cache errors must be measured.
The PR Rust owner and this trusted-main primer now share CI-only dev/test
profiles with `debug=0`; production release profiles and local developer
defaults stay unchanged. This targets the measured 14-GB unarchived Rust
test build tree, reducing symbol emission and linking work. It intentionally
invalidates previous debug-enabled sccache objects on the first PR, so only
subsequent warmed runs can establish whether the change achieves ≤300s.

## Dependency strategy

- Cargo already uses 22 workspace members. Another crate split only helps if
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
