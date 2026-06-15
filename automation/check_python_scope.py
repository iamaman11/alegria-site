#!/usr/bin/env python3
# Перенесён из pipeline/automation/ в automation/ (R5-R9 migration complete).
# ROOT теперь parents[1] (alegria-site/).
# ALLOWED_SQL_FILES: пуст — прямой SQL в Python запрещён во всей кодовой базе (R5-R8 завершены).
# ALLOWED_DRIVER_FILES: пуст — neo4j/qdrant_client в Python удалены из runtime (R&D допускается только в infra/analytics_lab).
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TARGETS = [
    ROOT / "automation",
]

# R5-R8 complete: no Python runtime module is allowed to use SQL directly.
# Exceptions:
# - explicit smoke-test scripts use psql on purpose for behavioral fail-safe verification;
# - local baseline/replay probes are ops diagnostics, not runtime modules;
# - contract scanners may contain SQL snippets only as strings they expect in Rust code.
ALLOWED_SQL_FILES: set[Path] = {
    (ROOT / "automation" / "bootstrap_graph_contract_projections.py").resolve(),
    (ROOT / "automation" / "bootstrap_local_runtime_baseline.py").resolve(),
    (ROOT / "automation" / "bootstrap_retrieval_contract_collections.py").resolve(),
    (ROOT / "automation" / "capture_seo_legacy_replay_inventory.py").resolve(),
    (ROOT / "automation" / "check_extraction_runtime_contract.py").resolve(),
    (ROOT / "automation" / "check_global_navigation_policy.py").resolve(),
    (ROOT / "automation" / "check_llm_extraction_contract.py").resolve(),
    (ROOT / "automation" / "check_rebuild_queue_execution_path.py").resolve(),
    (ROOT / "automation" / "check_seo_provenance_invariants.py").resolve(),
    (ROOT / "automation" / "check_serp_intelligence_contract.py").resolve(),
    (ROOT / "automation" / "local_db_baseline.py").resolve(),
    (ROOT / "automation" / "smoke_outbox_qdrant.py").resolve(),
    (ROOT / "automation" / "smoke_broken_schema_to_dlq_behavioral.py").resolve(),
    (ROOT / "automation" / "smoke_exhausted_retry_to_dlq_behavioral.py").resolve(),
    (ROOT / "automation" / "smoke_stale_outbox_reclaim_behavioral.py").resolve(),
    (ROOT / "automation" / "smoke_pending_hitl_not_failure_behavioral.py").resolve(),
    (ROOT / "automation" / "smoke_real_provider_minimal_scope.py").resolve(),
}

SQL_PATTERN = re.compile(
    r"\b(INSERT\s+INTO|UPDATE\s+\w+\s+SET|DELETE\s+FROM|SELECT\s+.+\s+FROM)\b",
    re.IGNORECASE,
)

# R8 complete: neo4j/qdrant_client are forbidden in Python runtime.
FORBIDDEN_DB_DRIVERS = re.compile(
    r"\bimport\s+(neo4j|qdrant_client)\b|\bfrom\s+(neo4j|qdrant_client)\s+import\b"
)
ALLOWED_DRIVER_FILES: set[Path] = set()


def scan_python_files() -> list[Path]:
    files: list[Path] = []
    for base in TARGETS:
        if not base.exists():
            continue
        files.extend(
            p
            for p in base.rglob("*.py")
            if ".venv" not in p.parts and "__pycache__" not in p.parts
        )
    return sorted(files)


def main() -> int:
    violations: list[str] = []
    for file_path in scan_python_files():
        text = file_path.read_text(encoding="utf-8")
        code_only = "\n".join(
            line for line in text.splitlines() if not line.lstrip().startswith("#")
        )
        rel = file_path.relative_to(ROOT)
        if FORBIDDEN_DB_DRIVERS.search(code_only) and file_path.resolve() not in ALLOWED_DRIVER_FILES:
            violations.append(f"{rel}: forbidden direct graph/vector driver import in Python")

        if file_path.resolve() not in ALLOWED_SQL_FILES and SQL_PATTERN.search(code_only):
            violations.append(f"{rel}: SQL statement detected outside allowed DB modules")

    if violations:
        print("PYTHON_SCOPE: FAILED")
        for v in violations:
            print(f"- {v}")
        return 1

    print("PYTHON_SCOPE: OK")
    print("allowed_driver_files:")
    for p in sorted(ALLOWED_DRIVER_FILES):
        print(f"  - {p.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
