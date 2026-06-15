#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
RAW_CRAWL = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"
TRUTH_LLM = ROOT / "app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs"
SEO_PORTS_SQLX = ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs"
TEMPORAL_STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"
WORKFLOWS_MOD = ROOT / "app/rust/services/temporal/src/workflows/mod.rs"
PRIMITIVES_LIB = ROOT / "app/rust/crates/primitives/src/lib.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "raw_crawl_adapter.rs":
        sub_dir = path.parent / "raw_crawl_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "seo_ports_sqlx_adapter.rs":
        sub_dir = path.parent / "seo_ports_sqlx_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "temporal_starter.rs":
        sub_dir = path.parent / "temporal_starter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    raw_crawl = read_file_resolved(RAW_CRAWL)
    truth_llm = TRUTH_LLM.read_text(encoding="utf-8")
    seo_ports_sqlx = read_file_resolved(SEO_PORTS_SQLX)
    temporal_starter = read_file_resolved(TEMPORAL_STARTER)
    workflows_mod = WORKFLOWS_MOD.read_text(encoding="utf-8")
    primitives_lib = PRIMITIVES_LIB.read_text(encoding="utf-8")

    failures: list[str] = []

    required = {
        RAW_CRAWL: [
            "truth_extraction_llm_adapter::extract_rule_candidates",
            "INSERT INTO extracted.rule_candidates",
        ],
        TRUTH_LLM: [
            "Return a JSON object with top-level key `candidates`.",
            "evidence_quote",
            "span_start",
            "span_end",
        ],
        SEO_PORTS_SQLX: [
            "blocked:no_truth_extraction_provider",
            "pending_review:needs_truth_adjudication",
            "empty:no_admissible_verified_rules",
        ],
    }
    for path, needles in required.items():
        text = read_file_resolved(path)
        for needle in needles:
            if needle not in text:
                failures.append(f"{path.relative_to(ROOT)} missing `{needle}`")

    for forbidden in [
        "facts_extractor",
        "facts_extractor_json",
        "extract_facts_typed",
        "source_allows_auto_verify",
        "WorkflowKind::FactExtraction",
        "FactExtractionWorkflow",
    ]:
        if forbidden in raw_crawl:
            failures.append(f"raw crawl still contains legacy `{forbidden}`")
        if forbidden in truth_llm:
            failures.append(f"truth extraction adapter still contains legacy `{forbidden}`")
        if forbidden in seo_ports_sqlx:
            failures.append(f"seo_ports_sqlx_adapter still contains legacy `{forbidden}`")
        if forbidden in temporal_starter:
            failures.append(f"temporal_starter still contains legacy `{forbidden}`")
        if forbidden in workflows_mod:
            failures.append(f"workflows/mod.rs still contains legacy `{forbidden}`")
        if forbidden in primitives_lib:
            failures.append(f"primitives/lib.rs still exports legacy `{forbidden}`")

    if "persist_from_pipeline_state" in raw_crawl or "ensure_extracted_concepts" in raw_crawl:
        failures.append("raw_knowledge_ingestion still contains verified-truth bridge helpers")

    if failures:
        print("EXTRACTION_RUNTIME_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("EXTRACTION_RUNTIME_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
