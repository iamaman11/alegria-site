# #94 PR CI: actual critical-path baseline and guarded cache experiment

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

## Measurable experiment introduced after this baseline

The required `rust-e2e-temporal-harness` compiler is the **only PR-local
`sccache` writer** (`READ_WRITE`, `CARGO_BUILD_JOBS=3`), while the remaining
parallel PR jobs remain `READ_ONLY`. GitHub Actions cache ref scoping keeps a
pull-request cache under its PR merge ref; the trusted-main cache is never
written by this job. The same Cargo command compiles the **current immutable
source** and the existing SHA-bound artifact manifest is still validated by
every E2E runner. All selected 47 live scenarios (48 if optional Truth
certification is applicable) remain mandatory. The job logs native cache hit,
write, HTTP-429 and read-only diagnostics; cache-write failures do not replace
or excuse compiler/test failures.

Collect three **different immutable PR head SHAs** with comparable source
changes, use the event timestamp and **final gate completion** (not only Rust
compiler time), and capture:

1. `CI_CARGO_FEEDBACK_JSON` compilation seconds and rebuilt workspace units.
2. `SCCACHE_CACHE_SUMMARY` Rust cache hit/miss/write/HTTP-429 counters.
3. Actual temporal harness, manual/blocked/draft, and full gate completion.
4. All real E2E counted by exact test-name filters, no skipped required rows.

The first cold SHA may remain slower, and GitHub cache write-rate limits may
make warm PR builds slower. These are **open measurement risks**; neither
cache hits nor three reruns of the *same* source SHA prove ≤180 seconds for
changed code. If cache-write pressure is degraded or full CI regresses,
revert the one PR `READ_WRITE` job to `READ_ONLY` without changing any test
routing, then prioritize extracting the provider-free V3 harness from the
legacy/full-provider Cargo dependency closure. No industrial SLA pass is
claimed before actual independent remote measurements.
