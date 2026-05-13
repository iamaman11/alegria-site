#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEO_STEPS = ROOT / "app/rust/crates/seo_steps/src"

FORBIDDEN = [
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "reqwest::",
    "rig_core",
    "rig_vertexai",
    "OpenAiEditorialClient",
    "AnthropicEditorialClient",
    "GeminiEditorialClient",
    "SEO_LLM_PROVIDER",
]


def main() -> int:
    failures: list[str] = []
    for path in sorted(SEO_STEPS.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for needle in FORBIDDEN:
            if needle in text:
                failures.append(f"{path.relative_to(ROOT)} contains forbidden LLM/HTTP runtime `{needle}`")
    if failures:
        print("NO_LLM_SDK_IN_SEO_STEPS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("NO_LLM_SDK_IN_SEO_STEPS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
