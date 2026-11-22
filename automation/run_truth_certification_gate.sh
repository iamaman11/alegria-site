#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-/tmp/truth_certification_local_ci_current.json}"
LOG_PATH="$(mktemp /tmp/truth-cert-gate.XXXXXX.log)"
TARGET_ROOT="${TRUTH_CERT_TARGET_ROOT:-$ROOT_DIR/app/rust/target}"
SUITE_TARGET_DIR="${TRUTH_CERT_SUITE_TARGET_DIR:-$TARGET_ROOT}"
# All Cargo commands are sequential. E2E runs prebuilt native Workers via
# explicit binary paths; no nested Cargo build can contend for this target.
TEMPORAL_BIN="$SUITE_TARGET_DIR/debug/temporal_worker"
OUTBOX_BIN="$SUITE_TARGET_DIR/debug/outbox_worker"

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

# Build real Workers in the already-warm suite target, then launch the full
# fixture harness. Standalone Truth runs also verify Qdrant transport here;
# canonical CI uses the same real test in its compiled integration sibling.
# No overlapping Cargo processes or locks within either target.
bash automation/run_truth_prepare.sh "$SUITE_TARGET_DIR"

for worker_bin in "$TEMPORAL_BIN" "$OUTBOX_BIN"; do
  if [[ ! -x "$worker_bin" ]]; then
    echo "Missing or non-executable Truth Worker binary: $worker_bin" >&2
    exit 1
  fi
done
echo "CI_TRUTH_WORKER_BINARIES temporal=$TEMPORAL_BIN outbox=$OUTBOX_BIN"

run_suite() {
  rm -f "$LOG_PATH"
  INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN" \
  INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN" \
  TRUTH_CERT_REPORT_PATH="$REPORT_PATH" \
  bash automation/measure_ci_phase.sh truth_suite cargo test --manifest-path app/rust/Cargo.toml \
    --target-dir "$SUITE_TARGET_DIR" \
    --timings \
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
