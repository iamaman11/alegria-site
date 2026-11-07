#!/usr/bin/env python3
"""Independently compare deployed HTTPS bytes with the certified public artifact."""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse

from check_public_static_output import validate_origin


ROOT = Path(__file__).resolve().parents[1]


def public_paths(manifest: dict) -> list[tuple[str, str]]:
    paths = [("sitemap.xml", "/sitemap.xml"), ("robots.txt", "/robots.txt")]
    pages = manifest.get("pages")
    if not isinstance(pages, list) or not pages:
        raise ValueError("manifest contains no published pages")
    for page in pages:
        if not isinstance(page, dict) or page.get("status") != "published":
            raise ValueError("public readback requires published pages only")
        route = page.get("canonical_url_path")
        if not isinstance(route, str) or not route.startswith("/"):
            raise ValueError("invalid page route")
        if (
            "//" in route or "\\\\" in route or "?" in route or "#" in route
            or any(part in {".", ".."} for part in route.split("/"))
        ):
            raise ValueError("unsafe public page route")
        path = "index.html" if route == "/" else route.strip("/") + "/index.html"
        paths.append((path, route))
    if len({p for p, _ in paths}) != len(paths):
        raise ValueError("duplicate routes in public manifest")
    return paths


def verify(output_dir: Path, *, timeout: float) -> dict:
    manifest = json.loads(
        (output_dir / "alegria-static-manifest.json").read_text(encoding="utf-8")
    )
    origin = str(manifest.get("base_url", "")).rstrip("/")
    problems = validate_origin(origin)
    if problems:
        raise ValueError("; ".join(problems))
    origin_parts = urlparse(origin)
    results = []
    for relative_file, route in public_paths(manifest):
        local_path = output_dir / relative_file
        if not local_path.is_file():
            raise ValueError(f"missing local artifact: {relative_file}")
        local_bytes = local_path.read_bytes()
        url = origin + route
        request = urllib.request.Request(
            url, headers={"User-Agent": "Alegria-Public-Acceptance/1", "Accept-Encoding": "identity"}
        )
        with urllib.request.urlopen(request, timeout=timeout) as response:
            if response.status != 200:
                raise ValueError(f"unexpected HTTP status at {route}: {response.status}")
            final_url = urlparse(response.geturl())
            if final_url.scheme != "https" or final_url.netloc != origin_parts.netloc:
                raise ValueError(f"cross-origin or non-HTTPS redirect at {route}")
            remote_bytes = response.read()
            remote_hash = hashlib.sha256(remote_bytes).hexdigest()
        expected_hash = hashlib.sha256(local_bytes).hexdigest()
        if remote_hash != expected_hash:
            raise ValueError(
                f"public content mismatch at {route}: expected sha256={expected_hash}, actual={remote_hash}"
            )
        results.append(
            {"url": url, "status": 200, "sha256": expected_hash, "bytes": len(local_bytes)}
        )
    return {
        "evidence_status": "PASS",
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "origin": origin,
        "published_page_count": len(manifest["pages"]),
        "verified_urls": results,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--timeout", type=float, default=20.0)
    args = parser.parse_args()
    try:
        report = verify(args.output_dir, timeout=args.timeout)
    except (OSError, ValueError, KeyError, json.JSONDecodeError, urllib.error.URLError) as exc:
        print(f"PUBLIC_HTTPS_READBACK: FAILED - {exc}")
        return 1
    if args.report is not None:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"PUBLIC_HTTPS_READBACK: OK pages={report['published_page_count']} urls={len(report['verified_urls'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
