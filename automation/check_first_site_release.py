#!/usr/bin/env python3
"""Prove the live immutable release came from this canonical Temporal run.

Run inside temporal-worker: its Docker-owned release symlinks use container
paths, so inspecting a similarly named host directory would be misleading.
This verifies provenance; static HTML and remote HTTPS bytes are checked
independently by the existing public validators.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from uuid import UUID


def verify_release(site: Path, release_root: Path, run_id: str) -> dict:
    canonical = str(UUID(run_id))
    if run_id != canonical:
        raise ValueError("expected canonical workflow run_id must be a UUID")
    if not site.is_symlink():
        raise ValueError("public site must be the managed immutable release symlink")
    release = site.resolve(strict=True)
    root = release_root.resolve(strict=True)
    if not release.is_dir() or release.parent != root or not release.name.endswith("-published"):
        raise ValueError("public site does not point to a direct immutable release")
    marker = release / ".alegria_static_site"
    if not marker.is_file() or marker.is_symlink() or (
        marker.read_bytes() != b"alegria_immutable_release@1\n"
    ):
        raise ValueError("immutable release marker is invalid")
    manifest = json.loads((release / "alegria-static-manifest.json").read_text(encoding="utf-8"))
    if manifest.get("publication_run_id") != canonical:
        raise ValueError("public release was not published by this canonical workflow run")
    key = manifest.get("publication_page_node_key")
    revision = manifest.get("publication_revision_id")
    pages = manifest.get("pages")
    if not isinstance(pages, list) or not pages or manifest.get("page_count") != len(pages):
        raise ValueError("public release contains no complete page manifest")
    if not isinstance(key, str) or not key or not isinstance(revision, str) or not revision:
        raise ValueError("release lacks an originating approved revision")
    matched = [
        page for page in pages
        if isinstance(page, dict)
        and page.get("page_node_key") == key
        and page.get("revision_id") == revision
        and page.get("status") == "published"
    ]
    if len(matched) != 1 or any(
        not isinstance(page, dict) or page.get("status") != "published"
        for page in pages
    ):
        raise ValueError("release does not contain the published originating revision")
    return {
        "status": "PASS",
        "run_id": canonical,
        "revision_id": revision,
        "page_node_key": key,
        "page_count": len(pages),
        "release_id": release.name,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True)
    parser.add_argument(
        "--public-output-dir",
        default=os.environ.get("SEO_STATIC_OUTPUT_DIR", ""),
    )
    parser.add_argument(
        "--release-root",
        default=os.environ.get("SEO_STATIC_RELEASE_ROOT", ""),
    )
    args = parser.parse_args()
    if not args.public_output_dir or not args.release_root:
        print("FIRST_SITE_RELEASE: FAILED - release paths are not configured")
        return 1
    try:
        report = verify_release(
            Path(args.public_output_dir), Path(args.release_root), args.run_id,
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"FIRST_SITE_RELEASE: FAILED - {exc}")
        return 1
    print(f"FIRST_SITE_RELEASE: OK run_id={report['run_id']} pages={report['page_count']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
