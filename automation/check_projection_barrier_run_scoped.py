#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
MIGRATION = ROOT / "app/db/migrations/20260514_0001_sync_outbox_run_mode_contract.sql"
SEO_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
PORTS_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
SCENARIO = ROOT / "app/rust/crates/seo_application/src/scenario.rs"


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    migration = MIGRATION.read_text(encoding="utf-8")
    seo_adapter = SEO_ADAPTER.read_text(encoding="utf-8")
    ports_adapter = PORTS_ADAPTER.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    scenario = SCENARIO.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        "ADD COLUMN IF NOT EXISTS run_id TEXT NOT NULL DEFAULT ''",
        "idx_system_sync_outbox_target_run_status",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")
        if needle not in migration:
            failures.append(f"migration missing `{needle}`")

    for needle in [
        "pub async fn read_projection_sync_status_for_run(",
        "AND outbox.run_id = $1",
        "AND run_id = $1",
    ]:
        if needle not in seo_adapter:
            failures.append(f"scoped projection status query missing `{needle}`")

    if "read_projection_sync_status_for_run(self.pool, run_id)" not in ports_adapter:
        failures.append("SqlxSeoRuntimeRepository does not load run-scoped projection status")
    if "read_projection_sync_status_for_run(pool, run_id)" not in activities:
        failures.append("Temporal activities still use global projection barrier")
    if "load_projection_barrier_status(run_id)" not in scenario:
        failures.append("Scenario path does not request projection status by run_id")

    if failures:
        print("PROJECTION_BARRIER_RUN_SCOPED: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("PROJECTION_BARRIER_RUN_SCOPED: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
