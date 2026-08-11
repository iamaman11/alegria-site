#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "app" / "rust" / "dist" / "static-site"
OPAQUE_SEGMENT = re.compile(r"^[0-9a-f]{48,}$", re.IGNORECASE)
INTERNAL_ROLE = re.compile(
    r"\b(?:child_hub|child_to_hub|hub_child|hub_to_child|country_visa_hub|"
    r"country_hub_to_visa_hub|visa_hub_to_country_hub|visa_country_hub|contextual)\b"
)


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def validate_origin(origin: str) -> list[str]:
    failures: list[str] = []
    parsed = urlparse(origin)
    if parsed.scheme != "https":
        failures.append(f"base_url must use https: {origin!r}")
    if not parsed.netloc:
        failures.append(f"base_url has no hostname: {origin!r}")
    host = (parsed.hostname or "").lower()
    if host in {"example.com", "www.example.com", "localhost", "127.0.0.1"}:
        failures.append(f"placeholder/local base_url is not launch-safe: {origin!r}")
    if parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
        failures.append(f"base_url must be an origin without path/query/fragment: {origin!r}")
    return failures


def opaque_path(path: str) -> bool:
    return any(OPAQUE_SEGMENT.fullmatch(part or "") for part in path.split("/"))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_dir", nargs="?", default=str(DEFAULT_OUTPUT))
    args = parser.parse_args()

    output_dir = Path(args.output_dir)
    if not output_dir.is_absolute():
        output_dir = ROOT / output_dir

    failures: list[str] = []
    manifest_path = output_dir / "alegria-static-manifest.json"
    sitemap_path = output_dir / "sitemap.xml"
    robots_path = output_dir / "robots.txt"

    for required in [manifest_path, sitemap_path, robots_path]:
        if not required.exists():
            failures.append(f"missing public artifact: {rel(required)}")

    if failures:
        print("PUBLIC_STATIC_OUTPUT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    origin = str(manifest.get("base_url", ""))
    failures.extend(validate_origin(origin))
    normalized_origin = origin.rstrip("/")

    pages = manifest.get("pages")
    if not isinstance(pages, list) or not pages:
        failures.append("manifest has no public pages")
        pages = []

    for page in pages:
        if not isinstance(page, dict):
            failures.append("manifest contains malformed page entry")
            continue
        path = str(page.get("canonical_url_path", ""))
        title = str(page.get("title", ""))
        status = str(page.get("status", ""))
        if status != "published":
            failures.append(f"non-published page leaked into public manifest: {path} status={status!r}")
        if opaque_path(path):
            failures.append(f"opaque internal identifier leaked into public route: {path}")
        if OPAQUE_SEGMENT.fullmatch(title.strip()):
            failures.append(f"opaque internal identifier leaked into public title: {title}")

    sitemap = sitemap_path.read_text(encoding="utf-8")
    if normalized_origin and f"<loc>{normalized_origin}/" not in sitemap:
        failures.append("sitemap does not use manifest base_url")
    if "example.com" in sitemap or "localhost" in sitemap:
        failures.append("sitemap contains placeholder/local origin")

    robots = robots_path.read_text(encoding="utf-8")
    if "User-agent:" not in robots:
        failures.append("robots.txt is malformed")

    for html_path in output_dir.rglob("*.html"):
        text = html_path.read_text(encoding="utf-8")
        if re.search(r'<html\s+lang=["\'](?:und|)["\']', text, re.IGNORECASE):
            failures.append(f"undefined/empty html language: {rel(html_path)}")
        if "https://example.com" in text or "http://localhost" in text or "https://localhost" in text:
            failures.append(f"placeholder/local origin leaked into HTML: {rel(html_path)}")
        if INTERNAL_ROLE.search(text):
            failures.append(f"internal link-role label leaked into user-visible HTML: {rel(html_path)}")
        canonical = re.search(r'<link\s+rel=["\']canonical["\']\s+href=["\']([^"\']+)', text, re.IGNORECASE)
        if not canonical:
            failures.append(f"missing canonical link: {rel(html_path)}")
        elif normalized_origin and not canonical.group(1).startswith(normalized_origin + "/"):
            failures.append(f"canonical origin mismatch: {rel(html_path)} -> {canonical.group(1)}")

    if failures:
        print("PUBLIC_STATIC_OUTPUT: FAILED")
        for failure in sorted(set(failures)):
            print(f"- {failure}")
        return 1

    print(f"PUBLIC_STATIC_OUTPUT: OK pages={len(pages)} output={rel(output_dir)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
