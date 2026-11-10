#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${WHOLE_PAGE_SEMANTIC_REPORT_PATH:-/tmp/whole_page_semantic_current.json}"
TARGET_DIR="${WHOLE_PAGE_SEMANTIC_TARGET_DIR:-$ROOT_DIR/app/rust/target/whole-page-semantic-gate}"

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

cargo run --manifest-path app/rust/services/temporal/Cargo.toml \
  --target-dir "$TARGET_DIR" \
  --bin whole_page_semantic_report -- \
  --fixture-dir app/rust/crates/integration_harness/fixtures/whole_page_semantic \
  --report-json "$REPORT_PATH"

python3 automation/check_whole_page_semantic_regression.py "$REPORT_PATH"
