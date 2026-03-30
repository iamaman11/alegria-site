#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PROTO_DIRS = [
    ROOT / "app" / "contracts" / "proto",
    ROOT / "app" / "analytics_lab" / "proto",
]
OUT_DEFAULT = ROOT / "automation" / "PROTO_CONTRACTS.md"


MESSAGE_RE = re.compile(r"^\s*message\s+([A-Za-z_]\w*)\s*\{", re.MULTILINE)
SERVICE_RE = re.compile(r"^\s*service\s+([A-Za-z_]\w*)\s*\{", re.MULTILINE)
PACKAGE_RE = re.compile(r'^\s*package\s+([A-Za-z0-9_.]+)\s*;', re.MULTILINE)


def sha256_text(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def collect_proto_files() -> list[Path]:
    files: list[Path] = []
    for d in PROTO_DIRS:
        if d.exists():
            files.extend(sorted(d.glob("*.proto")))
    return files


def render_contract() -> str:
    files = collect_proto_files()
    lines: list[str] = []
    lines.append("# Proto Contracts")
    lines.append("")
    lines.append("Generated file. Do not edit manually.")
    lines.append("")
    lines.append("## Inventory")
    lines.append("")
    lines.append("| Proto | Package | Messages | Services | SHA256 |")
    lines.append("|---|---|---:|---:|---|")

    for p in files:
        raw = p.read_bytes()
        txt = raw.decode("utf-8")
        pkg = ""
        m = PACKAGE_RE.search(txt)
        if m:
            pkg = m.group(1)
        msg_count = len(MESSAGE_RE.findall(txt))
        svc_count = len(SERVICE_RE.findall(txt))
        rel = p.relative_to(ROOT).as_posix()
        lines.append(
            f"| `{rel}` | `{pkg}` | {msg_count} | {svc_count} | `{sha256_text(raw)}` |"
        )

    lines.append("")
    lines.append("## Message/Service Index")
    lines.append("")
    for p in files:
        txt = p.read_text(encoding="utf-8")
        rel = p.relative_to(ROOT).as_posix()
        messages = sorted(set(MESSAGE_RE.findall(txt)))
        services = sorted(set(SERVICE_RE.findall(txt)))
        lines.append(f"### `{rel}`")
        lines.append("")
        lines.append(f"- messages: {', '.join(messages) if messages else '—'}")
        lines.append(f"- services: {', '.join(services) if services else '—'}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate proto contracts markdown for automation.")
    parser.add_argument("--out", default=str(OUT_DEFAULT))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    out = Path(args.out)
    content = render_contract()
    if args.check:
        if not out.exists():
            print(f"PROTO_CONTRACTS: FAILED\n- missing file: {out}")
            return 1
        existing = out.read_text(encoding="utf-8")
        if existing != content:
            print("PROTO_CONTRACTS: FAILED")
            print("- PROTO_CONTRACTS.md is out of date. Run: bash automation/contract_generate.sh")
            return 1
        print("PROTO_CONTRACTS: OK")
        return 0

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(content, encoding="utf-8")
    print(f"PROTO_CONTRACTS: generated {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
