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
    "CUTOVER_PHASE_M1_NON_LEDGERED_ACTIVITY_STEPS",
    "load_verified_support_bundle.* remains a non-ledgered activity surface",
    "\"projection_barrier(semantic_projection)\"",
    "\"raw_knowledge_ingestion\"",
]


def main() -> int:
    text = CLI_MAIN.read_text(encoding="utf-8")
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
