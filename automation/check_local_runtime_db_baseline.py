#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import shutil
import sys

from local_db_baseline import BASELINE_TABLES, check_database_baseline
from local_env import resolve_database_url


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database-url", default=None)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    if shutil.which("psql") is None:
        print("BASELINE_CHECK: FAILED")
        print("- `psql` is not available")
        return 1

    database_url = args.database_url or resolve_database_url()
    try:
        result = check_database_baseline(database_url)
    except Exception as exc:
        if args.json:
            print(
                json.dumps(
                    {
                        "database_url": database_url,
                        "status": "FAILED",
                        "reason": str(exc),
                    },
                    ensure_ascii=False,
                )
            )
        else:
            print("BASELINE_CHECK: FAILED")
            print(f"- {exc}")
        return 1

    payload = {
        "database_url": database_url,
        "status": result.status,
        "missing_tables": list(result.missing_tables),
        "required_tables": [f"{schema}.{table}" for schema, table in BASELINE_TABLES],
        "reason": result.reason(),
    }
    if args.json:
        print(json.dumps(payload, ensure_ascii=False))
    else:
        print(f"BASELINE_CHECK: {result.status}")
        print(f"- {result.reason()}")
    return 0 if result.is_ready else 2


if __name__ == "__main__":
    sys.exit(main())
