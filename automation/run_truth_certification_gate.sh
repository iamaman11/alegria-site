#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-$ROOT_DIR/docs/runs/truth_certification_local_ci_current.json}"

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

TRUTH_CERT_REPORT_PATH="$REPORT_PATH" \
cargo test --manifest-path app/rust/Cargo.toml \
  -p integration_harness \
  --features e2e \
  --lib \
  truth_certification_suite_matches_fixture_expectations \
  -- --nocapture

python3 automation/check_truth_certification_regression.py "$REPORT_PATH"
