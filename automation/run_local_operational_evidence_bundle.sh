#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${LOCAL_OPS_EVIDENCE_REPORT_PATH:-/tmp/local_operational_evidence_bundle_current.json}"
ENV_KEY="${LOCAL_OPS_EVIDENCE_ENV_KEY:-local-dev}"
RUN_CLEAN_ACCEPTANCE="${LOCAL_OPS_EVIDENCE_RUN_CLEAN_ACCEPTANCE:-1}"
RUN_RELEASE_GATE="${LOCAL_OPS_EVIDENCE_RUN_RELEASE_GATE:-1}"
RUN_LIVE_PROVIDER_GATE="${LOCAL_OPS_EVIDENCE_RUN_LIVE_PROVIDER_GATE:-0}"
RUN_RETRIEVAL_CONTRACT_GATE="${LOCAL_OPS_EVIDENCE_RUN_RETRIEVAL_CONTRACT_GATE:-0}"

CLEAN_ACCEPTANCE_REPORT="${LOCAL_OPS_EVIDENCE_CLEAN_ACCEPTANCE_REPORT:-/tmp/clean_acceptance_bundle_current.json}"
RELEASE_GATE_REPORT="${LOCAL_OPS_EVIDENCE_RELEASE_GATE_REPORT:-/tmp/release_restore_gate_current.json}"
LIVE_PROVIDER_EVIDENCE="${LOCAL_OPS_EVIDENCE_LIVE_PROVIDER_EVIDENCE:-$ROOT_DIR/docs/runs/live_provider_minimal_scope_evidence.json}"
LEGACY_REPLAY_EVIDENCE="${LOCAL_OPS_EVIDENCE_LEGACY_REPLAY_EVIDENCE:-$ROOT_DIR/docs/runs/seo_site_build_legacy_replay_evidence.json}"
RETRIEVAL_CONTRACT_EVIDENCE="${LOCAL_OPS_EVIDENCE_RETRIEVAL_CONTRACT_EVIDENCE:-$ROOT_DIR/docs/runs/retrieval_contract_gate_evidence.json}"
CLI_TARGET_DIR="${LOCAL_OPS_EVIDENCE_CLI_TARGET_DIR:-$ROOT_DIR/app/rust/target/local-ops-evidence-cli}"

cd "$ROOT_DIR"

if [[ "$RUN_CLEAN_ACCEPTANCE" == "1" ]]; then
  CLEAN_ACCEPTANCE_REPORT_PATH="$CLEAN_ACCEPTANCE_REPORT" \
    bash automation/run_clean_acceptance_bundle.sh
fi

if [[ "$RUN_RELEASE_GATE" == "1" ]]; then
  rm -f "$RELEASE_GATE_REPORT"
  (
    cd app/rust
    CARGO_TARGET_DIR="$CLI_TARGET_DIR" \
      cargo run -q -p cli_tools -- \
      seo-release-restore-gate \
      --root "$ROOT_DIR" \
      --report-json "$RELEASE_GATE_REPORT"
  )
fi

if [[ "$RUN_LIVE_PROVIDER_GATE" == "1" ]]; then
  bash automation/run_live_provider_minimal_scope_gate.sh || true
fi

if [[ "$RUN_RETRIEVAL_CONTRACT_GATE" == "1" ]]; then
  RETRIEVAL_CONTRACT_GATE_REPORT_PATH="$RETRIEVAL_CONTRACT_EVIDENCE" \
    bash automation/run_retrieval_contract_gate.sh || true
fi

python3 automation/check_live_provider_smoke_evidence_schema.py
python3 automation/check_seo_legacy_replay_evidence_schema.py
python3 automation/check_retrieval_contract_gate.py "$RETRIEVAL_CONTRACT_EVIDENCE"

python3 - <<'PY' \
  "$REPORT_PATH" \
  "$ENV_KEY" \
  "$CLEAN_ACCEPTANCE_REPORT" \
  "$RELEASE_GATE_REPORT" \
  "$LIVE_PROVIDER_EVIDENCE" \
  "$LEGACY_REPLAY_EVIDENCE" \
  "$RETRIEVAL_CONTRACT_EVIDENCE"
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

report_path = Path(sys.argv[1])
env_key = sys.argv[2]
clean_acceptance_path = Path(sys.argv[3])
release_gate_path = Path(sys.argv[4])
live_provider_path = Path(sys.argv[5])
legacy_replay_path = Path(sys.argv[6])
retrieval_contract_path = Path(sys.argv[7])


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def require(path: Path, label: str) -> dict:
    if not path.exists():
        raise SystemExit(f"missing required artifact for {label}: {path}")
    return load_json(path)


clean_acceptance = require(clean_acceptance_path, "clean_acceptance")
release_gate = require(release_gate_path, "release_restore_gate")
live_provider = require(live_provider_path, "live_provider")
legacy_replay = require(legacy_replay_path, "legacy_replay")
retrieval_contract = require(retrieval_contract_path, "retrieval_contract")

blocking_reasons: list[str] = []
external_blockers: list[str] = []

if clean_acceptance.get("status") != "PASS":
    blocking_reasons.append("clean acceptance bundle is not PASS")
if release_gate.get("status") != "ok":
    blocking_reasons.append("release/restore gate is not ok")
if legacy_replay.get("evidence_status") != "PASS":
    blocking_reasons.append("legacy replay evidence is not PASS")
if retrieval_contract.get("status") != "pass":
    blocking_reasons.append("retrieval contract gate is not pass")

live_status = live_provider.get("evidence_status")
retrieval_status = retrieval_contract.get("status")
if live_status == "PASS" and retrieval_status == "pass":
    overall_status = "PASS"
elif retrieval_status and retrieval_status.startswith("blocked_"):
    overall_status = "BLOCKED_ON_RETRIEVAL_CONTRACT"
    if retrieval_status == "blocked_provider_capability":
        external_blockers.append("retrieval contract is blocked on Voyage provider capability or credentials")
elif live_status == "PENDING_CREDENTIALS":
    overall_status = "BLOCKED_ON_LIVE_PROVIDER"
    external_blockers.append("live truth-extraction provider credentials are missing")
else:
    overall_status = "FAIL"
    blocking_reasons.append("live provider evidence is FAIL")

if blocking_reasons:
    overall_status = "FAIL"

payload = {
    "artifact_id": "local_operational_evidence_bundle",
    "environment_key": env_key,
    "evidence_status": overall_status,
    "status_reason": (
        "All local operational evidence surfaces pass except live-provider closure, which remains blocked on external credentials."
        if overall_status == "BLOCKED_ON_LIVE_PROVIDER"
        else (
            "All local operational evidence surfaces passed."
            if overall_status == "PASS"
            else "One or more required operational evidence surfaces are not passing."
        )
    ),
    "blocking_reasons": blocking_reasons,
    "external_blockers": external_blockers,
    "checks": {
        "clean_acceptance_bundle": clean_acceptance.get("status"),
        "release_restore_gate": release_gate.get("status"),
        "legacy_replay_evidence": legacy_replay.get("evidence_status"),
        "retrieval_contract_gate": retrieval_status,
        "live_provider_minimal_scope": live_status,
    },
    "artifacts": {
        "clean_acceptance_bundle": str(clean_acceptance_path),
        "release_restore_gate": str(release_gate_path),
        "legacy_replay_evidence": str(legacy_replay_path),
        "retrieval_contract_gate": str(retrieval_contract_path),
        "live_provider_minimal_scope": str(live_provider_path),
    },
    "required_for_pass": [
        "clean acceptance bundle passes from an isolated build root",
        "release/restore gate returns ok",
        "legacy replay evidence is PASS for the target environment",
        "retrieval contract gate is pass under hard-required Voyage/Qdrant policy",
        "live provider minimal scope evidence is PASS with a real provider",
    ],
    "updated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
}

report_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
print(f"report: {report_path}")
print(f"LOCAL_OPERATIONAL_EVIDENCE_BUNDLE: {overall_status}")
PY
