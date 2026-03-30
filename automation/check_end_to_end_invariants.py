#!/usr/bin/env python3
# Перенесён из pipeline/automation/ в automation/ (R5-R9 migration complete).
# ROOT теперь parents[1] (alegria-site/).
# Удалены устаревшие проверки: serp/ingest.py, hashing.py, url_norm.py, rust_bridge —
# все перенесены в Rust и заархивированы.
from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def must(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def resolve_run_file(filename: str) -> Path:
    """Resolve run artifact in flat docs layout first, legacy nested layout second."""
    flat = ROOT / "docs" / f"run_gemini3_global_186_20260320_top10__{filename}"
    if flat.exists():
        return flat
    legacy = ROOT / "docs" / "run_gemini3_global_186_20260320_top10" / filename
    return legacy


def main() -> int:
    failures: list[str] = []

    schema_path = ROOT / "app" / "db" / "schema.sql"
    results_path = resolve_run_file("results.jsonl")
    enrich_path = resolve_run_file("enrichment_resolve.jsonl")

    for required in [schema_path, results_path, enrich_path]:
        must(required.exists(), f"missing required file: {required}", failures)
    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1

    schema = read_text(schema_path)

    # Schema invariants.
    must("PRIMARY KEY (run_id, job_id)" in schema, "schema missing PK(run_id,job_id) for raw snapshots", failures)
    must("PRIMARY KEY (run_id, job_id, rank)" in schema, "schema missing PK(run_id,job_id,rank) for top10", failures)
    must(
        "source_key       TEXT PRIMARY KEY" in schema or "source_key TEXT PRIMARY KEY" in schema,
        "schema missing PK(source_key)",
        failures,
    )
    must(
        "url_norm          TEXT NOT NULL UNIQUE" in schema or "url_norm TEXT NOT NULL UNIQUE" in schema,
        "schema missing UNIQUE(url_norm) for crawl queue",
        failures,
    )

    # Data-level invariants: quick JSONL integrity checks.
    unique_jobs: set[str] = set()
    rank_pairs: set[tuple[str, int]] = set()
    seen_source_keys: set[str] = set()
    duplicate_jobs = 0
    duplicate_ranks = 0
    duplicate_source_keys = 0

    with results_path.open("r", encoding="utf-8") as f:
        for line_no, line in enumerate(f, start=1):
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError as e:
                failures.append(f"results.jsonl invalid JSON at line {line_no}: {e}")
                continue
            jid = rec.get("job_id")
            if not isinstance(jid, str):
                failures.append(f"results.jsonl line {line_no}: missing/invalid job_id")
                continue
            if jid in unique_jobs:
                duplicate_jobs += 1
            unique_jobs.add(jid)
            top10 = rec.get("top10", [])
            if not isinstance(top10, list):
                failures.append(f"results.jsonl line {line_no}: top10 is not list")
                continue
            for item in top10:
                rank = item.get("rank")
                if not isinstance(rank, int):
                    failures.append(f"results.jsonl line {line_no}: rank missing/int")
                    continue
                pair = (jid, rank)
                if pair in rank_pairs:
                    duplicate_ranks += 1
                rank_pairs.add(pair)

    with enrich_path.open("r", encoding="utf-8") as f:
        for line_no, line in enumerate(f, start=1):
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError as e:
                failures.append(f"enrichment_resolve.jsonl invalid JSON at line {line_no}: {e}")
                continue
            sk = rec.get("source_key")
            if not isinstance(sk, str):
                failures.append(f"enrichment_resolve.jsonl line {line_no}: missing source_key")
                continue
            if sk in seen_source_keys:
                duplicate_source_keys += 1
            seen_source_keys.add(sk)

    must(duplicate_jobs == 0, f"duplicate job_id detected in results.jsonl: {duplicate_jobs}", failures)
    must(duplicate_ranks == 0, f"duplicate (job_id, rank) detected in results.jsonl: {duplicate_ranks}", failures)
    must(
        duplicate_source_keys == 0,
        f"duplicate source_key detected in enrichment_resolve.jsonl: {duplicate_source_keys}",
        failures,
    )

    if failures:
        print("INVARIANTS: FAILED")
        for item in failures:
            print(f"- {item}")
        return 1

    print("INVARIANTS: OK")
    print(f"jobs={len(unique_jobs)} rank_pairs={len(rank_pairs)} source_keys={len(seen_source_keys)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
