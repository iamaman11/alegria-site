#!/usr/bin/env python3
"""
Smoke-test: вставляем событие в sync_outbox → ждём обработки Rust outbox_worker → проверяем в Qdrant.

Перенесён из pipeline/automation/ в automation/.
Переписан без rust_bridge и QdrantIngestor (оба заархивированы в R5/R9):
  - DB операции: subprocess psql
  - Qdrant проверка: urllib.request (REST API)
  - payload_hash / idempotency_key: только через Rust cli_tools, без Python hashing path

Требует:
  DATABASE_URL  — строка подключения к PostgreSQL (default: localhost:5433)
  QDRANT_URL    — base URL Qdrant (default: http://localhost:6333)
"""
from __future__ import annotations

import argparse
import binascii
import json
import os
import struct
import subprocess
import time
import urllib.request
import uuid

from local_env import resolve_database_url


ROOT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUST_DIR = os.path.join(ROOT_DIR, "app", "rust")


def _cargo_hash_bytes(payload_bytes: bytes) -> str:
    payload_hex = binascii.hexlify(payload_bytes).decode("ascii")
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "cli_tools", "--", "compute-bytes-hash", "--hex", payload_hex],
        cwd=RUST_DIR,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo compute-bytes-hash failed: {result.stderr.strip()}")
    return result.stdout.strip()


def _cargo_hash_text(text: str) -> str:
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "cli_tools", "--", "compute-content-hash", "--text", text],
        cwd=RUST_DIR,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo compute-content-hash failed: {result.stderr.strip()}")
    return result.stdout.strip()


def _varint(value: int) -> bytes:
    out = bytearray()
    while True:
        to_write = value & 0x7F
        value >>= 7
        if value:
            out.append(to_write | 0x80)
        else:
            out.append(to_write)
            return bytes(out)


def _tag(field_no: int, wire_type: int) -> bytes:
    return _varint((field_no << 3) | wire_type)


def _field_bytes(field_no: int, payload: bytes) -> bytes:
    return _tag(field_no, 2) + _varint(len(payload)) + payload


def _field_string(field_no: int, value: str) -> bytes:
    return _field_bytes(field_no, value.encode("utf-8"))


def _field_u32(field_no: int, value: int) -> bytes:
    return _tag(field_no, 0) + _varint(value)


def _field_f32(field_no: int, value: float) -> bytes:
    return _tag(field_no, 5) + struct.pack("<f", value)


def _encode_qdrant_entity_payload(payload: dict[str, str]) -> bytes:
    parts = []
    for field_no, key in (
        (1, "rule_instance_id"),
        (2, "context_key"),
        (3, "rule_type_key"),
        (4, "concept_key"),
        (5, "role_type"),
        (6, "source_key"),
    ):
        value = payload.get(key, "")
        if value:
            parts.append(_field_string(field_no, value))
    return b"".join(parts)


def _encode_qdrant_upsert_command(
    *,
    event_id: str,
    collection_name: str,
    entity_type: str,
    entity_key: str,
    point_id: str,
    vector: list[float],
    payload: dict[str, str],
    distance: str,
    vector_size: int,
) -> bytes:
    parts = [
        _field_string(1, event_id),
        _field_string(2, collection_name),
        _field_string(3, entity_type),
        _field_string(4, entity_key),
        _field_string(5, point_id),
    ]
    parts.extend(_field_f32(6, item) for item in vector)
    payload_bytes = _encode_qdrant_entity_payload(payload)
    if payload_bytes:
        parts.append(_field_bytes(7, payload_bytes))
    parts.append(_field_string(8, distance))
    parts.append(_field_u32(9, vector_size))
    return b"".join(parts)


def _psql(sql: str) -> str:
    db_url = resolve_database_url()
    result = subprocess.run(
        ["psql", db_url, "-t", "-A", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def enqueue_smoke_event(collection_name: str, point_id: str) -> str:
    event_id = str(uuid.uuid4())
    payload_bytes = _encode_qdrant_upsert_command(
        event_id=event_id,
        collection_name=collection_name,
        entity_type="smoke_test",
        entity_key=point_id,
        point_id=point_id,
        vector=[0.11, 0.22, 0.33, 0.44],
        payload={"source_key": "automation_smoke"},
        distance="cosine",
        vector_size=4,
    )
    payload_hex = binascii.hexlify(payload_bytes).decode("ascii")
    phash = _cargo_hash_bytes(payload_bytes)
    idempotency_key = _cargo_hash_text(f"{point_id}|QdrantUpsertCommand|{phash}")
    sql = (
        "INSERT INTO system.sync_outbox "
        "(aggregate_type, aggregate_key, target_system, event_type, payload_type, schema_version, idempotency_key, payload_bytes, payload_hash) "
        f"VALUES ('smoke_test', '{point_id}', 'qdrant', 'QdrantUpsertCommand', "
        f"'alegria.sync.v1.QdrantUpsertCommand', 1, '{idempotency_key}', decode('{payload_hex}', 'hex'), '{phash}') "
        "RETURNING event_id::text;"
    )
    return _psql(sql)


def wait_outbox_status(event_id: str, timeout_sec: int) -> tuple[str, str | None]:
    deadline = time.time() + timeout_sec
    last_status = "pending"
    last_error: str | None = None
    while time.time() < deadline:
        row = _psql(
            f"SELECT status, last_error FROM system.sync_outbox "
            f"WHERE event_id = '{event_id}';"
        )
        if row:
            parts = row.split("|")
            last_status = parts[0] if parts else "pending"
            last_error = parts[1] if len(parts) > 1 and parts[1] else None
            if last_status in {"done", "failed"}:
                return last_status, last_error
        time.sleep(1.0)
    return last_status, last_error


def verify_qdrant_point(collection_name: str, point_id: str) -> bool:
    qdrant_url = os.environ.get("QDRANT_URL", "http://localhost:6333")
    url = f"{qdrant_url}/collections/{collection_name}/points/{point_id}"
    try:
        with urllib.request.urlopen(url, timeout=5) as resp:
            data = json.loads(resp.read())
            return data.get("result") is not None
    except Exception:
        return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke-test outbox -> qdrant materialization.")
    parser.add_argument("--timeout-sec", type=int, default=60)
    parser.add_argument("--collection-name", default="kb_canonical_smoke")
    args = parser.parse_args()

    point_id = str(uuid.uuid4())
    event_id = enqueue_smoke_event(collection_name=args.collection_name, point_id=point_id)
    print(f"ENQUEUED event_id={event_id} point_id={point_id}")

    status, last_error = wait_outbox_status(event_id=event_id, timeout_sec=args.timeout_sec)
    print(f"OUTBOX status={status}")
    if last_error:
        print(f"OUTBOX last_error={last_error}")

    if status != "done":
        return 2

    point_exists = verify_qdrant_point(args.collection_name, point_id)
    print(f"QDRANT point_exists={point_exists}")
    return 0 if point_exists else 3


if __name__ == "__main__":
    raise SystemExit(main())
