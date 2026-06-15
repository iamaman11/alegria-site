#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/editorial_llm_adapter.rs"


def main() -> int:
    text = ADAPTER.read_text(encoding="utf-8")
    import re
    # Resolve any include!("...") statements to check the complete source text
    parent_dir = ADAPTER.parent
    for match in re.finditer(r'include!\("([^"]+)"\);', text):
        include_file = parent_dir / match.group(1)
        if include_file.exists():
            text += "\n" + include_file.read_text(encoding="utf-8")

    failures: list[str] = []
    for needle in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "SEO_LLM_LOCAL_ENDPOINT",
        "SEO_LLM_PROVIDER",
        "SEO_LLM_ALLOW_DETERMINISTIC_FALLBACK_ON_ERROR",
        "deterministic_fixture",
        "generate<'a>",
        "post_json_with_retries",
    ]:
        if needle not in text:
            failures.append(f"editorial adapter missing `{needle}`")

    if failures:
        print("EDITORIAL_PROVIDER_ROUTING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("EDITORIAL_PROVIDER_ROUTING: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
