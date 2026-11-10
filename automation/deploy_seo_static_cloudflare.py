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
import tempfile
import shutil

from check_public_http_readback import verify
from check_public_static_output import validate_origin

ROOT = Path(__file__).resolve().parents[1]
CLOUD_CONFIG = ROOT / "infra" / "cloudflare" / "seo-static"
EXPECTED_ACCOUNT = "78e5fcb3762c1473113d51838556da74"
EXPECTED_ORIGIN = "https://alegria-seo-staging.alegria-site.workers.dev"


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


def prepare_public_assets(release: Path, destination: Path) -> Path:
    """Copy only website assets: release ledger/marker are never public."""
    destination.mkdir(parents=True, exist_ok=False)
    for item in release.rglob("*"):
        relative = item.relative_to(release)
        if relative.name == "alegria-static-manifest.json" or any(
            part.startswith(".") for part in relative.parts
        ):
            continue
        target = destination / relative
        if item.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        elif item.is_file() and not item.is_symlink():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(item, target)
        else:
            raise ValueError(f"unsupported asset in immutable release: {relative}")
    if not (destination / "index.html").is_file():
        raise ValueError("static release must contain a public homepage")
    return destination


def run_cf_deploy(cf_binary: Path, release: Path) -> bool:
    """Native cf build + prebuilt deploy; never deploy a raw directory argument."""
    with tempfile.TemporaryDirectory(prefix="alegria-cloudflare-assets-") as directory:
        public = prepare_public_assets(release, Path(directory) / "public")
        env = os.environ.copy()
        env["CI"] = "1"
        env["ALEGRIA_CF_PUBLIC_DIR"] = str(public)
        built = subprocess.run(
            [str(cf_binary), "build"], cwd=CLOUD_CONFIG, env=env, check=False
        )
        if built.returncode != 0:
            return False
        # cf --prebuilt must use a real validated Build Output, not stale artifacts.
        if not (CLOUD_CONFIG / ".cloudflare/output/v0/config.json").is_file():
            return False
        output_assets = CLOUD_CONFIG / ".cloudflare/output/v0/workers/default/assets"
        if not output_assets.is_dir():
            return False
        # Only the exact approved asset set may be published. A stale build
        # output can contain extra routes even when every expected file matches.
        # Reject symlinks as well as missing, altered or unexpected assets.
        source_files = {
            source.relative_to(public)
            for source in public.rglob("*")
            if source.is_file()
        }
        # Vite emits an unreferenced empty JS entry for build-entry.js.
        # Remove only this provably empty synthetic chunk; never deploy it
        # or exempt arbitrary additional files from the approved asset set.
        for entry in output_assets.glob("assets/build-entry-*.js"):
            if not entry.is_file() or entry.is_symlink() or entry.read_bytes().strip():
                return False
            entry.unlink()
        built_entries = list(output_assets.rglob("*"))
        if any(entry.is_symlink() for entry in built_entries):
            return False
        built_files = {
            entry.relative_to(output_assets)
            for entry in built_entries
            if entry.is_file()
        }
        if source_files != built_files:
            return False
        for relative in source_files:
            if (output_assets / relative).read_bytes() != (public / relative).read_bytes():
                return False
        if (output_assets / "alegria-static-manifest.json").exists() or (
            output_assets / ".alegria_static_site"
        ).exists():
            return False
        uploaded = subprocess.run(
            [str(cf_binary), "deploy", "--prebuilt", "--mode", "production"],
            cwd=CLOUD_CONFIG,
            env=env,
            check=False,
        )
        return uploaded.returncode == 0


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
            raise ValueError("CLOUDFLARE_ACCOUNT_ID must match the dedicated Alegria account")
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
    except Exception as exc:  # Fail closed on unexpected provider/validation errors.
        print(f"SEO_CLOUDFLARE_DEPLOY: FAILED - {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
