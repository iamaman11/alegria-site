#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
ARCHIVE = DOCS / "_archive"
KNOWLEDGE = DOCS / "knowledge"
RUNS = DOCS / "runs"
SELF = Path(__file__).resolve()
ALLOWED_DOCS_SUBDIRS = {"_archive", "runs"}
RUN_ARTIFACT_SUFFIXES = {".json", ".jsonl", ".md"}


def main() -> int:
    violations: list[str] = []

    if KNOWLEDGE.exists():
        violations.append("docs/knowledge must not exist (docs must be flat in docs/ root)")

    if not DOCS.exists():
        violations.append("docs directory is missing")
    else:
        for d in sorted(p for p in DOCS.iterdir() if p.is_dir()):
            if d.name not in ALLOWED_DOCS_SUBDIRS:
                violations.append(f"unexpected subdirectory in docs/: {d}")

        # Any active file must be in docs root (not nested), except docs/_archive/**
        # and immutable machine-generated run artifacts under docs/runs/**.
        for p in sorted(DOCS.rglob("*")):
            if not p.is_file():
                continue
            if ARCHIVE in p.parents:
                continue
            if RUNS in p.parents:
                if p.suffix.lower() not in RUN_ARTIFACT_SUFFIXES:
                    violations.append(f"unexpected run artifact suffix under docs/runs: {p}")
                continue
            if p.parent != DOCS:
                violations.append(f"active doc is nested, must be in docs root: {p}")

        # Active docs ownership must be current user (root-owned active docs are forbidden)
        uid = os.getuid()
        for p in sorted(DOCS.iterdir()):
            if not p.is_file():
                continue
            if p.stat().st_uid != uid:
                violations.append(f"active doc is not user-owned: {p}")

    # Active text files must not refer to docs/knowledge/*
    text_suffixes = {".md", ".txt", ".sh", ".py", ".toml", ".yml", ".yaml", ".json"}
    bad_ref = re.compile(r"docs/knowledge/")
    for base in [DOCS, ROOT / "automation"]:
        if not base.exists():
            continue
        for p in sorted(base.rglob("*")):
            if not p.is_file():
                continue
            if p.resolve() == SELF:
                continue
            if ARCHIVE in p.parents:
                continue
            if p.suffix.lower() not in text_suffixes:
                continue
            try:
                txt = p.read_text(encoding="utf-8")
            except Exception:
                continue
            if bad_ref.search(txt):
                violations.append(f"stale reference to docs/knowledge in active file: {p}")

    if violations:
        print("DOCS_LAYOUT: FAILED")
        for v in violations:
            print(f"- {v}")
        return 1

    print("DOCS_LAYOUT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
