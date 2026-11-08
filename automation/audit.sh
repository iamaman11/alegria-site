#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

MODE="${1:-fast}"
REPORT_JSON="automation/reports/consistency_report.json"
REPORT_MD="automation/reports/consistency_report.md"

run_fast() {
  echo "== Alegria automation audit (fast) =="
  python3 automation/check_docs_layout.py
  python3 automation/check_python_scope.py
  python3 automation/check_end_to_end_invariants.py
  bash automation/check_python_rust_boundary.sh
  python3 automation/check_rust_adapter_boundaries.py
  python3 automation/check_rust_layer_isolation.py
  python3 automation/check_json_boundary_policy.py
}

run_full() {
  echo "== Alegria automation audit (full) =="
  bash automation/ci_verify.sh
}

case "$MODE" in
  fast)
    run_fast
    ;;
  full)
    run_full
    ;;
  *)
    echo "Usage: bash automation/audit.sh [fast|full]"
    exit 2
    ;;
esac

python3 - <<'PY'
import json
from pathlib import Path

root = Path(".")
src = root / "automation/reports/consistency_report.json"
dst = root / "automation/reports/consistency_report.md"

if not src.exists():
    print("no consistency_report.json — skipping markdown render")
    raise SystemExit(0)

rep = json.loads(src.read_text(encoding="utf-8"))
status = rep.get("status", "unknown").upper()
findings = rep.get("findings", [])
errors = [f for f in findings if f.get("level") == "error"]
warns = [f for f in findings if f.get("level") == "warn"]

lines = []
lines.append(f"# Consistency Report: {status}")
lines.append("")
lines.append(f"- errors: {len(errors)}")
lines.append(f"- warnings: {len(warns)}")
lines.append("")

if findings:
    lines.append("## Findings")
    lines.append("")
    for f in findings:
        lines.append(f"- [{f.get('level')}] {f.get('code')}: {f.get('message')}")
    lines.append("")

cmds = rep.get("commands", [])
if cmds:
    lines.append("## Commands")
    lines.append("")
    for c in cmds:
        state = "OK" if c.get("ok") else "FAIL"
        lines.append(f"- {c.get('cmd')}: {state}")
    lines.append("")

dst.write_text("\n".join(lines), encoding="utf-8")
print(f"report md: {dst}")
PY

echo "Audit completed."
echo "MD: $REPORT_MD"
