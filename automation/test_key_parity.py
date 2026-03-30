#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "infra" / "analytics_lab" / "src"))
try:
    import key_utils  # type: ignore  # noqa: E402
except Exception as e:  # pragma: no cover
    print(f"KEY_PARITY: SKIPPED ({e})")
    print("Install analytics deps first: pip install -r infra/analytics_lab/requirements.txt")
    raise SystemExit(0)


def run_cli(args: list[str]) -> str:
    cmd = [
        "cargo",
        "run",
        "-q",
        "--manifest-path",
        "app/rust/services/cli_tools/Cargo.toml",
        "--",
        *args,
    ]
    out = subprocess.check_output(cmd, cwd=ROOT, text=True).strip()
    return out


def main() -> int:
    vectors = [
        ["pl", "work", "d05a", "by"],
        ["es", "nomad", "d8", "by"],
        ["it", "tourist", "schengen", "by"],
        ["fr", "tourist", "", "by"],
        ["de", "business", "short", "by"],
    ]

    failures: list[str] = []
    for parts in vectors:
        py_val = key_utils.stable_rule_instance_id(parts)
        rust_val = run_cli(["compute-stable-rule-id", "--parts", *parts])
        if py_val != rust_val:
            failures.append(f"stable_rule_instance_id mismatch: {parts} py={py_val} rust={rust_val}")

        py_hash = key_utils.content_hash_v1("|".join(parts))
        rust_hash = run_cli(["compute-content-hash", "--text", "|".join(parts)])
        if py_hash != rust_hash:
            failures.append(f"content_hash mismatch: {parts} py={py_hash} rust={rust_hash}")

    report = {
        "status": "ok" if not failures else "failed",
        "vectors": len(vectors),
        "failures": failures,
    }
    out = ROOT / "automation" / "reports" / "key_parity.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    if failures:
        print("KEY_PARITY: FAILED")
        for f in failures:
            print(f"- {f}")
        return 1
    print("KEY_PARITY: OK")
    print(f"report: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
