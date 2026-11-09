#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

REQUIRED = [
    ROOT / "infra" / "backups" / "backup_business_pg.sh",
    ROOT / "infra" / "backups" / "backup_temporal_pg.sh",
    ROOT / "infra" / "backups" / "restore_business_pg.sh",
    ROOT / "infra" / "backups" / "restore_temporal_pg.sh",
    ROOT / "infra" / "backups" / "restore_drill.sh",
    ROOT / "infra" / "pgbouncer" / "pgbouncer.ini",
    ROOT / "infra" / "pgbouncer" / "userlist.txt",
    ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md",
]


def main() -> int:
    failures: list[str] = []
    for path in REQUIRED:
        if not path.exists():
            failures.append(f"missing file: {path.relative_to(ROOT)}")
            continue
        if path.suffix == ".sh" and not os.access(path, os.X_OK):
            failures.append(f"script is not executable: {path.relative_to(ROOT)}")

    runbook = (ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md").read_text(encoding="utf-8")
    for needle in ("restore_drill", "backup_business_pg.sh", "backup_temporal_pg.sh", "alegria_pgbouncer"):
        if needle not in runbook:
            failures.append(f"missing `{needle}` in docs/OPS_RUNTIME_RUNBOOK.md")

    drill = (ROOT / "infra" / "backups" / "restore_drill.sh").read_text(encoding="utf-8")
    for guard in ("business_db=", "temporal_db=", "createdb -U postgres",
                  "createdb -U temporal", "dropdb -U postgres",
                  "dropdb -U temporal", "ON_ERROR_STOP=1"):
        if guard not in drill:
            failures.append(f"restore drill lacks isolated-database guard: {guard}")
    if "restore_business_pg.sh" in drill or "restore_temporal_pg.sh" in drill:
        failures.append("restore drill must never call destructive live-schema restoration")

    if failures:
        print("BACKUP_RESTORE_LAYOUT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("BACKUP_RESTORE_LAYOUT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
