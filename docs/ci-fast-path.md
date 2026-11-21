# CI for the experimental Alegria app

The pull-request gate is intentionally a **fast feedback** gate; deep runtime
certification is available separately through GitHub Actions → `ci-deep` →
**Run workflow**, or locally with `bash automation/ci_verify.sh all`.

## Measured result (GitHub Actions, 21 November 2026)

| Exact PR CI | Full wall-clock | Slowest work |
| --- | ---: | --- |
| [#223](https://github.com/iamaman11/alegria-site/actions/runs/44915481077) | ~7m 02s | Truth 416s; Rust contracts 368s; architecture 356s; verify 322s |
| [#224](https://github.com/iamaman11/alegria-site/actions/runs/44917627978) | **3m 28s** | RustSec 199s; architecture 189s |

Both runs passed every required job. The timings above are **warm-cache**
observations, not a guarantee for a cold Docker/Rust rebuild.

## On each pull request

- Real `cargo test --workspace --locked` and one PostgreSQL e2e regression.
- Python/SQLx/SEO/Truth policy contracts and native Windows operator checks.
- RustSec dependency audit, real Docker worker image and provenance artifact,
  Cloudflare static build, Temporal/PostgreSQL and storage/graph container checks.
- A single Rust compilation owner; the static contract jobs no longer compile
  or repeatedly bootstrap Rust/Temporal/PostgreSQL.

## Deep verification on demand

The manually dispatched `ci-deep.yml` retains all 16 real-worker Truth
certification fixtures, Qdrant REST→gRPC transport, the manual LLM lifecycle,
rebuild/extraction Rust-backed checks, semantic fixtures, protocol checks,
and the remaining PostgreSQL regressions.

Run it before a release, or after changes to Temporal workflow behavior,
truth admission, publishing, or SQL schema. These checks were **moved out of
the PR critical path**, not deleted or represented as having run on PR.

## Dependency strategy

- Cargo already uses 20 workspace members. Another crate split only helps if
  it actually removes a heavy transitive dependency from a frequently tested
  package; it does not improve a four-way rebuild of the same graph.
- Preserve one owner for Cargo registry cache writes and use `sccache` in the
  Rust owner. Avoid archiving multi-gigabyte `target/` directories.
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
