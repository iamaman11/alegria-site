#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-/tmp/truth_certification_local_ci_current.json}"
LOG_PATH="$(mktemp /tmp/truth-cert-gate.XXXXXX.log)"
NESTED_TARGET_ROOT="$ROOT_DIR/app/rust/target/integration-harness-nested"
SUITE_TARGET_DIR="$ROOT_DIR/app/rust/target/integration-harness-suite"
TEMPORAL_TARGET_DIR="$NESTED_TARGET_ROOT/temporal_worker"
OUTBOX_TARGET_DIR="$NESTED_TARGET_ROOT/outbox_worker"
TEMPORAL_BIN="$TEMPORAL_TARGET_DIR/debug/temporal_worker"
OUTBOX_BIN="$OUTBOX_TARGET_DIR/debug/outbox_worker"

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

cargo build --manifest-path app/rust/Cargo.toml \
  --target-dir "$TEMPORAL_TARGET_DIR" \
  -p temporal_worker \
  --bin temporal_worker

cargo build --manifest-path app/rust/Cargo.toml \
  --target-dir "$OUTBOX_TARGET_DIR" \
  -p outbox_worker \
  --bin outbox_worker

run_suite() {
  rm -f "$LOG_PATH"
  INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN" \
  INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN" \
  TRUTH_CERT_REPORT_PATH="$REPORT_PATH" \
  cargo test --manifest-path app/rust/Cargo.toml \
    --target-dir "$SUITE_TARGET_DIR" \
    -p integration_harness \
    --features e2e \
    --lib \
    truth_certification_suite_matches_fixture_expectations \
    -- --nocapture 2>&1 | tee "$LOG_PATH"
}

is_transient_infra_failure() {
  grep -Eq \
    "probe (neo4j|qdrant|temporal) harness failed|Connection reset by peer|bind text stub listener failed" \
    "$LOG_PATH"
}

if ! run_suite; then
  if is_transient_infra_failure; then
    echo "truth certification gate hit transient infra bootstrap failure; retrying once" >&2
    sleep 3
    run_suite
  else
    exit 1
  fi
fi

python3 automation/check_truth_certification_regression.py "$REPORT_PATH"
