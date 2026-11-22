#!/usr/bin/env bash
# The same real REST -> gRPC outbox regression can reuse the existing
# integration_harness e2e target instead of compiling it again elsewhere.
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo 'usage: run_qdrant_transport_gate.sh <shared-cargo-target>' >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="$(realpath -m "$1")"
cd "$ROOT_DIR"

bash automation/measure_ci_phase.sh truth_transport cargo test --manifest-path app/rust/Cargo.toml \
  --locked --target-dir "$TARGET_DIR" --timings -p integration_harness --features e2e --lib \
  qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval -- --nocapture
