# #94 PR CI: measured critical path and failed PR write-through cache trial

Source: [successful run #534](https://github.com/iamaman11/alegria-site/actions/runs/45623135814),
head `c19fed70d9fbeb62629b9d09b409bebe764e3272`, GitHub REST
`actions/runs/45623135814` and `actions/runs/45623135814/jobs?per_page=100`.
This run is an observation, **not** evidence for a future source revision.
All times are UTC on 2026-11-25 and use GitHub's actual job and step timestamps.

| Boundary | Started | Finished | Duration |
| --- | --- | --- | ---: |
| Workflow creation → first mandatory runner | 13:23:46 | 13:24:12 | 26 s |
| Impact plan, including runner | 13:24:12 | 13:24:21 | 9 s |
| Temporal exact-source E2E compiler | 13:24:24 | 13:27:30 | 186 s |
| Temporal `cargo test --no-run` inside compiler | 13:24:33 | 13:27:24 | 171 s |
| Legacy manual Temporal shard, with real containers | 13:27:32 | 13:28:48 | 76 s |
| Actual manual test execution | 13:27:40 | 13:28:47 | 67 s |
| Final mandatory aggregate gate | 13:28:51 | 13:28:57 | 6 s |
| **End-to-end source → green gate** | **13:23:46** | **13:28:58** | **312 s** |

The compiler's native `CARGO_FEEDBACK_JSON` reported 170.381 seconds,
29 rebuilt workspace targets, and one exact-source 136,619,632-byte Temporal
E2E test executable. Native `sccache 0.16` reported 307 Rust hits,
202 Rust misses, 148 C/C++ misses, and 54 seconds cumulative cache-read time.
The compiler uses **read-only** trusted-main cache in the baseline. It never
compiles or runs the existing full-stack scenario with a fake provider.

The 180-second full-CI SLA therefore needs **at least 132 seconds of actual
critical-path improvement**, not a faster policy step or a reduced live-test
inventory. The impact job's own steps consumed only 9 seconds; 26 seconds was
initial GitHub runner provisioning, so removing `needs: impact` would not
reliably save the 30 seconds sometimes attributed to it.

## V6.1 independent source-SHA experiment: PR cache writes regressed the cold build

[V6.1 CI run #45627832681](https://github.com/iamaman11/alegria-site/actions/runs/45627832681),
head `c2054f12d0ff74bf15faab2439f18cc7f5c6bfc9`, used the same exact-source
Temporal E2E compiler but made that one job the sole PR-local `sccache`
`READ_WRITE` writer with `CARGO_BUILD_JOBS=3`. Parallel PR compilers remained
read-only. This run includes additional legitimate V6 PostgreSQL/Qdrant code,
so source diff is a confounder; timings below are measurements rather than an
isolated controlled causal experiment.

The run finished fully green: 32 jobs succeeded, and eight conditional jobs
were correctly skipped (including optional Truth certification). The
mandatory aggregate gate passed. The event-to-aggregate
gate took **413 seconds**, from 13:51:22 to 13:58:15 UTC (versus 312 seconds
for V5). The critical path was impact (10 s), Temporal compiler job (294 s,
including the 270.596-second Cargo compilation), real manual Temporal E2E
(90 s), and the aggregate gate (9 s). Real `verify_d` exercised 10 PostgreSQL
tests, and the `vector` shard exercised both required Qdrant/PG tests.

| Temporal E2E compiler measurement | V5 read-only | V6.1 write-through |
| --- | ---: | ---: |
| Actual `cargo test --locked --no-run` elapsed | 170.381 s | **270.596 s** |
| Rust compiler object cache hits / misses | 307 / 202 | 366 / 143 |
| Cache hit-read cumulative duration | 54.174 s | **153.331 s** |
| Cache write cumulative duration | 0 s | **138.757 s** |
| Cache writes / errors / HTTP429 | 0 / 0 / 0 | 253 / 0 / 0 |
| Cumulative actual compiler execution | 217.365 s | 192.705 s |

The write-through compiler was **100.215 seconds slower**, although it had
more cache hits and less compiler work. Its cumulative cache-read time was
99.157 seconds higher; concurrent GHA cache read/write contention is a
plausible explanation, but cannot be proved from one different-source pair.
Zero HTTP429 errors did **not** imply a low-latency cache backend. Increasing
`CARGO_BUILD_JOBS` to 6/8 might overlap waits but could instead increase
provider pressure and memory contention; no evidence supports an automatic
speedup. This experiment **failed its wall-clock objective**.

**V6.2 response:** restore `SCCACHE_GHA_RW_MODE=READ_ONLY` for the Temporal
compiler, remove its special `CARGO_BUILD_JOBS=3`, debug logging and cache
reporting step. All PR Rust compilers return to the accepted-main read-only
cache policy. The exact-source Cargo command, artifact SHA verification and
every real E2E shard remain unchanged. This is a *conservative regression
revert*, **not** a claim that the complete CI meets ≤180 seconds.

The V6.1 run independently proved both new mandatory external-provider tests:
`vector` executed two real Qdrant/PG tests (`CI_E2E_SHARD=PASS tests=2`), and
`verify_d` executed 10 real PG tests (`CI_E2E_SHARD=PASS tests=10`), including
native tenant LOGIN/RLS and cross-tenant TRUNCATE rejection.

## Future measurement required before an SLA claim

Collect three **different immutable PR head SHAs** with comparable source
changes, use the event timestamp and **final gate completion** (not only Rust
compiler time), and capture:

1. `CI_CARGO_FEEDBACK_JSON` compilation seconds and rebuilt workspace units.
2. Native sccache object hit/miss/read counters (without enabling a PR writer).
3. Actual temporal harness, manual/blocked/draft, and full gate completion.
4. All real E2E counted by exact test-name filters, no skipped required rows.

The first cold SHA may remain slow. Neither cache hits nor three reruns of
the *same* source SHA prove ≤180 seconds for changed code. A substantial
remaining opportunity is the provider-free V3 harness split and ultimately
the legacy/manual E2E compiler's 29-crate full-provider dependency closure;
neither change is silently substituted for mandatory real tests. No industrial
SLA pass is claimed before independent remote green-gate measurements.
