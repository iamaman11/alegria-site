#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
CLI_MAIN = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"

FORBIDDEN = [
    "async fn signal_seo_workflow_resume(",
    "async fn resolve_seo_workflow_id(",
    "sqlx_seo_cms_adapter::apply_human_review_decision(",
    "connect_client(",
    "UntypedSignal::<UntypedWorkflow>::new(\"resume\")",
]


def main() -> int:
    text = CLI_MAIN.read_text(encoding="utf-8")
    errors = [needle for needle in FORBIDDEN if needle in text]
    if errors:
        print("CLI_TOOLS_SEO_SURFACE: FAILED")
        for needle in errors:
            print(f"- forbidden direct SEO mutation surface in cli_tools: {needle}")
        return 1
    print("CLI_TOOLS_SEO_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
