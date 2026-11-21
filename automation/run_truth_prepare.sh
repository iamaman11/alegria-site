#!/usr/bin/env bash
# Single native Cargo target: build Workers, then Qdrant transport and suite.
# All operations are sequential. Runtime process isolation is still real.
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo 'usage: run_truth_prepare.sh <shared-cargo-target>' >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="$(realpath -m "$1")"
cd "$ROOT_DIR"

bash automation/measure_ci_phase.sh truth_worker_build cargo build --manifest-path app/rust/Cargo.toml \
  --locked --target-dir "$TARGET_DIR" --timings -p temporal_worker --bin temporal_worker

bash automation/measure_ci_phase.sh truth_outbox_build cargo build --manifest-path app/rust/Cargo.toml \
  --locked --target-dir "$TARGET_DIR" --timings -p outbox_worker --bin outbox_worker

bash automation/measure_ci_phase.sh truth_transport cargo test --manifest-path app/rust/Cargo.toml \
  --locked --target-dir "$TARGET_DIR" --timings -p integration_harness --features e2e --lib \
  qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval -- --nocapture

echo "CI_TRUTH_PREP_SHARED_TARGET=PASS target=$TARGET_DIR"
