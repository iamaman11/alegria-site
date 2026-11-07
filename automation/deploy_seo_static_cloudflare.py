#!/usr/bin/env python3
"""Explicit Cloudflare Workers Static Assets upload of a certified Alegria release.

This command is a deployment *adapter*, not a second site builder. An actual
approved, immutable release must already exist. It does not set CMS published
status; the canonical finalizer still requires independent HTTPS evidence.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from check_public_http_readback import verify
from check_public_static_output import validate_origin

ROOT = Path(__file__).resolve().parents[1]
CLOUD_CONFIG = ROOT / "infra" / "cloudflare" / "seo-static"
EXPECTED_ACCOUNT = "4426df1449e417511bc7697d60b7f62f"
EXPECTED_ORIGIN = "https://seo-staging.alegria.by"


def validate_release(directory: Path, *, root: Path, expected_origin: str) -> Path:
    release = directory.resolve(strict=True)
    release_root = root.resolve(strict=True)
    if release.parent != release_root or not release.is_dir():
        raise ValueError("release must be an immediate immutable child of SEO_STATIC_RELEASE_ROOT")
    if not release.name.endswith("-published"):
        raise ValueError("only immutable -published releases may be deployed")
    marker = release / ".alegria_static_site"
    if not marker.is_file() or marker.is_symlink() or marker.read_bytes() != b"alegria_immutable_release@1\n":
        raise ValueError("release is missing the canonical immutable marker")
    manifest_file = release / "alegria-static-manifest.json"
    if not manifest_file.is_file() or manifest_file.is_symlink():
        raise ValueError("release is missing a regular manifest file")
    manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    origin = str(manifest.get("base_url", "")).rstrip("/")
    if validate_origin(origin) or origin != expected_origin:
        raise ValueError("release origin must match the dedicated SEO staging hostname")
    pages = manifest.get("pages")
    if not isinstance(pages, list) or not pages or manifest.get("page_count") != len(pages):
        raise ValueError("release manifest must contain non-empty, matching page_count")
    if any(not isinstance(p, dict) or p.get("status") != "published" for p in pages):
        raise ValueError("only pages with actual published status are permitted")
    if any(p.is_symlink() for p in release.rglob("*")):
        raise ValueError("release tree must not contain symlinks")
    return release


def preflight(release: Path) -> None:
    result = subprocess.run(
        [sys.executable, str(ROOT / "automation" / "check_public_static_output.py"), str(release)],
        cwd=ROOT, check=False,
    )
    if result.returncode != 0:
        raise ValueError("canonical public static output validation failed")


def run_cf_deploy(cf_binary: Path, release: Path) -> bool:
    proc = subprocess.run(
        [str(cf_binary), "deploy", str(release)],
        cwd=CLOUD_CONFIG,
        check=False,
    )
    return proc.returncode == 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("release", type=Path)
    parser.add_argument("--release-root", type=Path, required=True)
    parser.add_argument("--previous-release", type=Path)
    parser.add_argument("--deploy", action="store_true", help="explicitly allow Cloudflare deployment")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args(argv)

    try:
        release = validate_release(args.release, root=args.release_root, expected_origin=EXPECTED_ORIGIN)
        preflight(release)
        previous = None
        if args.previous_release is not None:
            previous = validate_release(
                args.previous_release, root=args.release_root, expected_origin=EXPECTED_ORIGIN
            )
            preflight(previous)

        if not args.deploy:
            print("SEO_CLOUDFLARE_DEPLOY: PREFLIGHT_ONLY; no public change made")
            return 0

        if os.environ.get("CLOUDFLARE_ACCOUNT_ID") != EXPECTED_ACCOUNT:
            raise ValueError("CLOUDFLARE_ACCOUNT_ID must match the Alegria zone account")
        if not os.environ.get("CLOUDFLARE_API_TOKEN"):
            raise ValueError("CLOUDFLARE_API_TOKEN is required; never put it in the repository")
        cf = CLOUD_CONFIG / "node_modules" / ".bin" / "cf"
        if not cf.is_file():
            raise ValueError("pinned cf CLI missing; install infra/cloudflare/seo-static dependencies")

        try:
            if not run_cf_deploy(cf, release):
                raise RuntimeError("Cloudflare CLI deployment returned non-zero")
            report = verify(release, timeout=20.0)
        except Exception:
            if previous is not None:
                if not run_cf_deploy(cf, previous):
                    raise RuntimeError("Cloudflare rollback failed; manual recovery required")
                verify(previous, timeout=20.0)
            else:
                print(
                    "WARNING: first deployment has no prior release; inspect the live Worker "
                    "before any CMS publication", file=sys.stderr
                )
            raise

        if args.report is not None:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"SEO_CLOUDFLARE_DEPLOY: HTTPS_VERIFIED pages={report['published_page_count']}")
        return 0
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError, Exception) as exc:
        print(f"SEO_CLOUDFLARE_DEPLOY: FAILED - {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
