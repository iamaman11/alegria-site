#!/usr/bin/env python3
"""Fail-closed reachability gate for one SQLx 0.9.0 optional-driver RustSec advisory.

The PostgreSQL-only Alegria workspace does not enable SQLite. Cargo.lock still
resolves sqlx-macros-core's optional sqlx-sqlite package, locked to
libsqlite3-sys 0.37.0; SQLx 0.9.0 restricts that optional package to <0.38,
whereas RUSTSEC-2026-0459 is fixed in >=0.39.0.

This check MUST pass before the narrowly configured cargo-audit exception is
effective. Any active sqlite dependency, new version, additional audit ignore,
or missing audit config FAILS CI. No vulnerability in a built crate is waived.
"""

from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
RUST = ROOT / "app" / "rust"
ADVISORY = "RUSTSEC-2026-0459"


def check() -> None:
    locked = tomllib.loads((RUST / "Cargo.lock").read_text(encoding="utf-8"))
    sqlite = [
        package
        for package in locked["package"]
        if package["name"] == "libsqlite3-sys"
    ]
    audit = tomllib.loads((RUST / ".cargo" / "audit.toml").read_text(encoding="utf-8"))
    ignored = audit.get("advisories", {}).get("ignore", [])
    if len(sqlite) != 1 or sqlite[0]["version"] != "0.37.0":
        raise SystemExit(
            "SQLITE_AUDIT_SCOPE: lockfile changed; remove/re-evaluate waiver before CI"
        )
    if ignored != [ADVISORY]:
        raise SystemExit(
            "SQLITE_AUDIT_SCOPE: exactly one reviewed optional-only advisory is allowed"
        )

    manifest = tomllib.loads((RUST / "Cargo.toml").read_text(encoding="utf-8"))
    features = manifest["workspace"]["dependencies"]["sqlx"]["features"]
    if "postgres" not in features or any("sqlite" in feature for feature in features):
        raise SystemExit("SQLITE_AUDIT_SCOPE: workspace no longer PostgreSQL-only")

    # Cargo resolves workspace-wide normal/build/dev dependencies for all
    # platforms, not only Linux. A future feature activating SQLite must fail.
    command = [
        "cargo", "tree", "--locked", "--workspace", "--target", "all",
        "-e", "normal,build,dev", "--prefix", "none",
    ]
    result = subprocess.run(
        command, cwd=RUST, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise SystemExit(
            f"SQLITE_AUDIT_SCOPE: cannot prove reachability: {result.stderr[-1800:]}"
        )
    if re.search(r"(?m)^(?:libsqlite3-sys|sqlx-sqlite) v", result.stdout):
        raise SystemExit(
            "SQLITE_AUDIT_SCOPE: SQLite is compiled; remove vulnerable package, "
            "do NOT whitelist RUSTSEC-2026-0459"
        )
    print(
        "SQLITE_AUDIT_SCOPE: PASS — SQLx optional SQLite is not in any "
        "workspace target; exact advisory waiver is lockfile-only"
    )


if __name__ == "__main__":
    check()
