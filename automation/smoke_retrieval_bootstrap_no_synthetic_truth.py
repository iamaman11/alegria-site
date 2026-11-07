#!/usr/bin/env python3
"""Prove readiness bootstrap cannot fabricate search evidence."""
from __future__ import annotations

from unittest.mock import patch

import bootstrap_retrieval_contract_collections as bootstrap


def main() -> int:
    calls: list[tuple[str, str, dict | None]] = []
    sql_statements: list[str] = []

    def fake_request(
        method: str, path: str, payload: dict | None = None
    ) -> tuple[int, str]:
        calls.append((method, path, payload))
        return 200, '{"status":"ok"}'

    def fake_psql(sql: str) -> None:
        sql_statements.append(sql)

    with (
        patch.object(bootstrap, "request", side_effect=fake_request),
        patch.object(bootstrap, "psql", side_effect=fake_psql),
    ):
        result = bootstrap.main()

    assert result == 0, "collection bootstrap must complete"
    count = len(bootstrap.REQUIRED_COLLECTIONS)
    assert len(calls) == 2 * count, "each collection must be created and cleaned"
    assert len(sql_statements) == count, "legacy SQL probes must be cleaned"
    for collection in bootstrap.REQUIRED_COLLECTIONS:
        assert any(
            method == "PUT"
            and path == f"/collections/{collection}"
            and payload == {"vectors": {"size": 1024, "distance": "Cosine"}}
            for method, path, payload in calls
        ), f"missing actual collection bootstrap for {collection}"
        assert any(
            method == "POST"
            and path == f"/collections/{collection}/points/delete?wait=true"
            and isinstance(payload, dict)
            and len(payload.get("points", [])) == 1
            for method, path, payload in calls
        ), f"legacy probe was not deleted for {collection}"
        assert any(
            sql.lstrip().startswith("DELETE")
            and "kb.qdrant_points" in sql
            and f"retrieval_contract_probe:{collection}" in sql
            for sql in sql_statements
        ), f"legacy SQL probe not cleaned for {collection}"
    assert not any("/points?wait=true" in path for _, path, _ in calls)
    assert not any("INSERT INTO kb.qdrant_points" in sql for sql in sql_statements)
    print("RETRIEVAL_BOOTSTRAP_NO_SYNTHETIC_TRUTH: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
