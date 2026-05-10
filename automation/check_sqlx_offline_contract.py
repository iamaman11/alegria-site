#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
WORKSPACE_CARGO = WORKSPACE / "Cargo.toml"
SQLX_CACHE = WORKSPACE / ".sqlx"
CI_VERIFY = ROOT / "automation" / "ci_verify.sh"
TEMPORAL_DOCKERFILE = ROOT / "app" / "rust" / "services" / "temporal" / "Dockerfile"


def must_contain(path: Path, needle: str) -> str | None:
    text = path.read_text(encoding="utf-8")
    if needle not in text:
        return f"missing `{needle}` in {path.relative_to(ROOT)}"
    return None


def main() -> int:
    failures: list[str] = []
    if not SQLX_CACHE.exists():
        failures.append("missing app/rust/.sqlx cache directory")
    else:
        cache_files = list(SQLX_CACHE.glob("query-*.json"))
        if not cache_files:
            failures.append("app/rust/.sqlx has no query cache files")

    for item in [
        must_contain(WORKSPACE_CARGO, '"macros"'),
        must_contain(CI_VERIFY, "SQLX_OFFLINE=true cargo check -q"),
        must_contain(CI_VERIFY, "SQLX_OFFLINE=true cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml --"),
        must_contain(TEMPORAL_DOCKERFILE, "ENV SQLX_OFFLINE=true"),
    ]:
        if item is not None:
            failures.append(item)

    if failures:
        print("SQLX_OFFLINE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SQLX_OFFLINE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
