#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_ROOT="${CLEAN_ACCEPTANCE_TARGET_ROOT:-$ROOT_DIR/app/rust/target/clean-acceptance}"
REPORT_PATH="${CLEAN_ACCEPTANCE_REPORT_PATH:-/tmp/clean_acceptance_bundle_current.json}"
WHOLE_PAGE_REPORT_PATH="${WHOLE_PAGE_SEMANTIC_REPORT_PATH:-/tmp/whole_page_semantic_current.json}"
TRUTH_CERT_REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-/tmp/truth_certification_local_ci_current.json}"
ISOLATED_CARGO_HOME="${CLEAN_ACCEPTANCE_CARGO_HOME:-}"

cd "$ROOT_DIR"

bash automation/clean_build_residue.sh
mkdir -p "$TARGET_ROOT"
rm -f "$REPORT_PATH"

if [[ -n "$ISOLATED_CARGO_HOME" ]]; then
  mkdir -p "$ISOLATED_CARGO_HOME"
  export CARGO_HOME="$ISOLATED_CARGO_HOME"
fi

cargo check --manifest-path app/rust/Cargo.toml \
  --target-dir "$TARGET_ROOT/cargo-check" \
  -p infrastructure \
  -p temporal_worker

WHOLE_PAGE_SEMANTIC_TARGET_DIR="$TARGET_ROOT/whole-page-semantic-gate" \
WHOLE_PAGE_SEMANTIC_REPORT_PATH="$WHOLE_PAGE_REPORT_PATH" \
  bash automation/run_whole_page_semantic_gate.sh

TRUTH_CERT_TARGET_ROOT="$TARGET_ROOT/truth-certification" \
TRUTH_CERT_REPORT_PATH="$TRUTH_CERT_REPORT_PATH" \
  bash automation/run_truth_certification_gate.sh

python3 automation/check_docs_layout.py
python3 automation/check_truth_governance_policy.py
git diff --check

python3 - <<'PY' "$REPORT_PATH" "$TARGET_ROOT" "$WHOLE_PAGE_REPORT_PATH" "$TRUTH_CERT_REPORT_PATH" "${CARGO_HOME:-}"
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
target_root = sys.argv[2]
whole_page_report = sys.argv[3]
truth_cert_report = sys.argv[4]
cargo_home = sys.argv[5]

payload = {
    "artifact_id": "clean_acceptance_bundle",
    "status": "PASS",
    "cargo_target_root": target_root,
    "cargo_home": cargo_home,
    "checks": {
        "cargo_check": "PASS",
        "whole_page_semantic_gate": "PASS",
        "truth_certification_gate": "PASS",
        "docs_layout": "PASS",
        "truth_governance_policy": "PASS",
        "git_diff_check": "PASS",
    },
    "artifacts": {
        "whole_page_semantic_report": whole_page_report,
        "truth_certification_report": truth_cert_report,
    },
}

report_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
print(f"report: {report_path}")
print("CLEAN_ACCEPTANCE_BUNDLE: PASS")
PY
