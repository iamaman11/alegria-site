#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
import uuid


REQUIRED_COLLECTIONS = [
    "raw_chunks_4",
    "raw_chunks_ctx",
    "kb_canonical_4",
    "verified_rules_4",
    "editorial_topics_4",
    "seo_keyword_clusters_4",
    "whole_page_advisory_prototypes",
]
VECTOR_SIZE = 1024


def database_url() -> str:
    return os.environ.get(
        "DATABASE_URL", "postgres://postgres:postgres_password@localhost:5433/alegria"
    )


def qdrant_url() -> str:
    value = os.environ.get("QDRANT_REST_URL") or os.environ.get("QDRANT_URL") or "http://localhost:6333"
    value = value.rstrip("/")
    if value.endswith(":6334"):
        value = value[:-5] + ":6333"
    return value


def request(method: str, path: str, payload: dict | None = None) -> tuple[int, str]:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{qdrant_url()}{path}",
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read().decode("utf-8", errors="replace")


def psql(sql: str) -> None:
    proc = subprocess.run(
        ["psql", database_url(), "-v", "ON_ERROR_STOP=1", "-c", sql],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")


def sql_literal(value: str) -> str:
    return value.replace("'", "''")


def probe_vector(collection: str) -> list[float]:
    seed = uuid.uuid5(uuid.NAMESPACE_URL, f"alegria-retrieval-contract:{collection}").int
    return [(((seed >> (idx % 64)) & 1023) + 1) / 1024.0 for idx in range(VECTOR_SIZE)]


def main() -> int:
    bootstrapped = []
    for collection in REQUIRED_COLLECTIONS:
        status, body = request(
            "PUT",
            f"/collections/{collection}",
            {"vectors": {"size": VECTOR_SIZE, "distance": "Cosine"}},
        )
        if status not in {200, 409}:
            print(
                f"BOOTSTRAP_RETRIEVAL_COLLECTIONS: FAILED create collection={collection} status={status} body={body}",
                file=sys.stderr,
            )
            return 1
        point_id = str(uuid.uuid5(uuid.NAMESPACE_URL, f"alegria-retrieval-contract:{collection}:point"))
        status, body = request(
            "PUT",
            f"/collections/{collection}/points?wait=true",
            {
                "points": [
                    {
                        "id": point_id,
                        "vector": probe_vector(collection),
                        "payload": {
                            "entity_type": "retrieval_contract_probe",
                            "entity_key": f"retrieval_contract_probe:{collection}",
                            "collection_name": collection,
                        },
                    }
                ]
            },
        )
        if status not in {200, 201}:
            print(
                f"BOOTSTRAP_RETRIEVAL_COLLECTIONS: FAILED upsert collection={collection} status={status} body={body}",
                file=sys.stderr,
            )
            return 1
        entity_key = f"retrieval_contract_probe:{collection}"
        psql(
            f"""
            INSERT INTO kb.qdrant_points
                (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
            VALUES
                ('{point_id}', 'retrieval_contract_probe', '{sql_literal(entity_key)}',
                 '{sql_literal(collection)}', 'deterministic-probe', 'retrieval_contract_probe@1')
            ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
            SET point_id = EXCLUDED.point_id,
                embedding_model = EXCLUDED.embedding_model,
                embedding_version = EXCLUDED.embedding_version,
                updated_at = now()
            """
        )
        bootstrapped.append(collection)
    print(
        "BOOTSTRAP_RETRIEVAL_COLLECTIONS: OK "
        + json.dumps({"collections": bootstrapped}, ensure_ascii=False)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
