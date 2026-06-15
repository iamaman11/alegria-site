#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
CLI_MAIN = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"

REQUIRED_NEEDLES = [
    "SeoCutoverShadowVerify {",
    "legacy_run_id: String,",
    "cutover_run_id: String,",
    "strict: bool,",
    "async fn seo_cutover_shadow_verify(",
    "CUTOVER_PHASE_M1_LEDGER_STEPS",
    "CUTOVER_PHASE_M2_LEDGER_STEPS",
    "LEGACY_PHASE_M2_LEDGER_STEPS",
    "CUTOVER_PHASE_M1_NON_LEDGERED_ACTIVITY_STEPS",
    "cms_publish_event_summary",
    "\"phase_m2\"",
    "load_verified_support_bundle.* remains a non-ledgered activity surface",
    "\"projection_barrier(semantic_projection)\"",
    "\"raw_knowledge_ingestion\"",
]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "main.rs" and "cli_tools" in path.parts:
        sub_dir = path.parent / "cli"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    text = read_file_resolved(CLI_MAIN)
    missing = [needle for needle in REQUIRED_NEEDLES if needle not in text]
    if missing:
        print("SHADOW_VERIFICATION_SURFACE: FAILED")
        for needle in missing:
            print(f"- missing shadow verification surface marker: {needle}")
        return 1
    print("SHADOW_VERIFICATION_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
