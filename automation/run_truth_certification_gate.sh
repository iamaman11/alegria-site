#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-/tmp/truth_certification_local_ci_current.json}"
LOG_PATH="$(mktemp /tmp/truth-cert-gate.XXXXXX.log)"

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

run_suite() {
  rm -f "$LOG_PATH"
  TRUTH_CERT_REPORT_PATH="$REPORT_PATH" \
  cargo test --manifest-path app/rust/Cargo.toml \
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
