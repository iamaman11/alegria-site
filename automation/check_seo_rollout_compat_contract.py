#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]

FILES = {
    "build_modes": ROOT / "docs/OPS_TEMPORAL_BUILD_MODES.md",
    "runbook": ROOT / "docs/OPS_RUNTIME_RUNBOOK.md",
    "plan": ROOT / "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md",
    "starter": ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs",
    "registration": ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs",
    "load_input": ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs",
    "barrier": ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs",
}

REQUIRED = {
    "build_modes": [
        "Когда обязателен новый `workflow type`",
        "Rollback / drain policy",
        "старый build-id нельзя выключать до явного drain старых history;",
        "новый workflow type вводится параллельно старому",
    ],
    "runbook": [
        "New workflow type is mandatory when workflow branch/order/signal semantics change",
        "Rollback/drain policy: old worker build-id stays alive until old histories drain",
        "Versioned outbox contract:",
    ],
    "plan": [
        "strict sequential execution",
        "Step 1 — Close `R0` Compatibility Perimeter",
        "legacy payload/default behavior",
        "current-run `run_id` stamping coverage",
        "compatibility window",
    ],
    "starter": [
        'default_value = "publish_with_hitl"',
        "normalize_run_mode(&run_mode)",
    ],
    "registration": [
        'unwrap_or_else(|| "publish_with_hitl".to_string())',
    ],
    "load_input": [
        'if input.run_mode.trim().is_empty() {',
        'input.run_mode = "publish_with_hitl".to_string();',
    ],
    "barrier": [
        "pub async fn read_projection_sync_status_for_run(",
        "AND outbox.run_id = $1",
        "AND run_id = $1",
    ],
}


def read_with_includes(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8")
    import re
    parent = path.parent
    for match in re.finditer(r'include!\("([^"]+)"\);', text):
        include_path = parent / match.group(1)
        if include_path.exists():
            text += "\n" + read_with_includes(include_path)
    return text


def clean_str(s: str) -> str:
    return "".join(s.split())


def main() -> int:
    failures: list[str] = []
    for key, path in FILES.items():
        text = read_with_includes(path)
        for needle in REQUIRED.get(key, []):
            if clean_str(needle) not in clean_str(text):
                failures.append(f"{path.relative_to(ROOT)} missing `{needle}`")

    if failures:
        print("SEO_ROLLOUT_COMPAT_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_ROLLOUT_COMPAT_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
