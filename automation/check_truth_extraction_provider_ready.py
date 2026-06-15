#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import sys

from local_env import load_local_env


def configured_providers() -> list[dict[str, str]]:
    providers: list[dict[str, str]] = []
    vertex_project = (
        os.environ.get("VERTEX_GEMINI_PROJECT")
        or os.environ.get("GOOGLE_CLOUD_PROJECT")
        or os.environ.get("GCLOUD_PROJECT")
    )
    if vertex_project and os.environ.get("VERTEX_GEMINI_ENABLED", "true").lower() not in {"0", "false"}:
        providers.append(
            {
                "provider": "vertex_gemini",
                "model": os.environ.get("VERTEX_GEMINI_TRUTH_MODEL")
                or os.environ.get("VERTEX_GEMINI_MODEL")
                or "gemini-2.5-pro",
                "config_path": "VERTEX_GEMINI_PROJECT|GOOGLE_CLOUD_PROJECT|GCLOUD_PROJECT + ADC",
            }
        )
    if os.environ.get("SEO_TRUTH_LLM_LOCAL_ENDPOINT") or os.environ.get("SEO_LLM_LOCAL_ENDPOINT"):
        providers.append(
            {
                "provider": "local_compatible",
                "model": os.environ.get("SEO_TRUTH_LLM_LOCAL_MODEL")
                or os.environ.get("SEO_LLM_LOCAL_MODEL")
                or "local-truth-extraction-model",
                "config_path": "SEO_TRUTH_LLM_LOCAL_ENDPOINT|SEO_LLM_LOCAL_ENDPOINT",
            }
        )
    if os.environ.get("OPENAI_API_KEY"):
        providers.append(
            {
                "provider": "openai",
                "model": os.environ.get("OPENAI_TRUTH_MODEL")
                or os.environ.get("OPENAI_SEO_MODEL")
                or "gpt-5.2",
                "config_path": "OPENAI_API_KEY",
            }
        )
    if os.environ.get("ANTHROPIC_API_KEY"):
        providers.append(
            {
                "provider": "anthropic",
                "model": os.environ.get("ANTHROPIC_TRUTH_MODEL")
                or os.environ.get("ANTHROPIC_SEO_MODEL")
                or "claude-4.5-sonnet",
                "config_path": "ANTHROPIC_API_KEY",
            }
        )
    if os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY"):
        providers.append(
            {
                "provider": "gemini",
                "model": os.environ.get("GEMINI_TRUTH_MODEL")
                or os.environ.get("GEMINI_SEO_MODEL")
                or "gemini-2.5-pro",
                "config_path": "GEMINI_API_KEY|GOOGLE_API_KEY",
            }
        )
    return providers


def main() -> int:
    load_local_env()
    providers = configured_providers()
    if not providers:
        print("TRUTH_EXTRACTION_PROVIDER_READY: NOT_READY")
        print(
            "- recommended for current Step 5: VERTEX_GEMINI_PROJECT (or GOOGLE_CLOUD_PROJECT) with ADC; "
            "fallback GEMINI_API_KEY (fallback GOOGLE_API_KEY); "
            "alternatives remain SEO_TRUTH_LLM_LOCAL_ENDPOINT|SEO_LLM_LOCAL_ENDPOINT, "
            "OPENAI_API_KEY, ANTHROPIC_API_KEY"
        )
        return 2

    print("TRUTH_EXTRACTION_PROVIDER_READY: OK")
    print(json.dumps({"configured_providers": providers}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
