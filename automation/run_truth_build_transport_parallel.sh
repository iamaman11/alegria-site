#!/usr/bin/env bash
# Prepare real Truth Worker binaries and prove Qdrant REST/gRPC independently.
# Their isolated Cargo targets permit overlap without a shared target lock.
set -euo pipefail

if [[ $# -ne 2 || -z "$1" || -z "$2" ]]; then
  echo 'usage: run_truth_build_transport_parallel.sh <workers-target> <suite-target>' >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKERS_TARGET_DIR="$(realpath -m "$1")"
SUITE_TARGET_DIR="$(realpath -m "$2")"
if [[ "$WORKERS_TARGET_DIR" == "$SUITE_TARGET_DIR" ]]; then
  echo 'Truth Worker and transport Cargo targets must differ' >&2
  exit 2
fi
cd "$ROOT_DIR"

monotonic_ms() {
  python3 -c 'import time; print(time.monotonic_ns() // 1_000_000)'
}
started_ms="$(monotonic_ms)"

# Keep Cargo's default scheduler: on actual CI #222 splitting a 4-CPU runner
# into 2+2 jobs lengthened the real Qdrant transport test by 45s, and did not
# reduce parallel preparation time. Retain per-crate timings for diagnosis.

(
  set -e
  bash automation/measure_ci_phase.sh truth_worker_build cargo build --manifest-path app/rust/Cargo.toml \
    --target-dir "$WORKERS_TARGET_DIR" \
    --timings \
    -p temporal_worker \
    --bin temporal_worker
  bash automation/measure_ci_phase.sh truth_outbox_build cargo build --manifest-path app/rust/Cargo.toml \
    --target-dir "$WORKERS_TARGET_DIR" \
    --timings \
    -p outbox_worker \
    --bin outbox_worker
) &
worker_build_pid=$!

(
  bash automation/measure_ci_phase.sh truth_transport cargo test --manifest-path app/rust/Cargo.toml \
    --target-dir "$SUITE_TARGET_DIR" \
    --timings \
    -p integration_harness \
    --features e2e \
    --lib \
    qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval \
    -- --nocapture
) &
transport_pid=$!

# Observe both results even if one branch fails. Do not let a successful wait
# hide a failed Cargo command; the subsequent Truth suite is never started.
worker_rc=0
if wait "$worker_build_pid"; then :; else worker_rc=$?; fi
transport_rc=0
if wait "$transport_pid"; then :; else transport_rc=$?; fi

echo "CI_TRUTH_PREP_PARALLEL elapsed_ms=$(($(monotonic_ms) - started_ms)) worker_exit=$worker_rc transport_exit=$transport_rc"
if [[ "$worker_rc" -ne 0 ]]; then exit "$worker_rc"; fi
if [[ "$transport_rc" -ne 0 ]]; then exit "$transport_rc"; fi
