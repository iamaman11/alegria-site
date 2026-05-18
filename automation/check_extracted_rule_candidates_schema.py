#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    failures: list[str] = []
    for needle in [
        "CREATE TABLE IF NOT EXISTS extracted.rule_candidates",
        "rule_candidate_id      TEXT PRIMARY KEY",
        "context_key            TEXT NOT NULL REFERENCES kb.visa_contexts(context_key)",
        "raw_section_id         BIGINT NOT NULL REFERENCES raw.sections(id)",
        "role                   TEXT NOT NULL",
        "concept_canonical_key  TEXT NOT NULL",
        "params                 JSONB NOT NULL DEFAULT '{}'::jsonb",
        "scope                  JSONB NOT NULL DEFAULT '{}'::jsonb",
        "evidence_section_id    BIGINT NOT NULL REFERENCES raw.sections(id)",
        "evidence_quote         TEXT NOT NULL",
        "span_start             INTEGER NOT NULL CHECK (span_start >= 0)",
        "span_end               INTEGER NOT NULL CHECK (span_end > span_start)",
        "source_snapshot_hash   TEXT NOT NULL",
        "llm_provider           TEXT NOT NULL",
        "llm_model              TEXT NOT NULL",
        "prompt_version         TEXT NOT NULL",
        "epistemic_status       TEXT NOT NULL DEFAULT 'candidate'",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    if failures:
        print("EXTRACTED_RULE_CANDIDATES_SCHEMA: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("EXTRACTED_RULE_CANDIDATES_SCHEMA: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
