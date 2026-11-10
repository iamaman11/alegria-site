#!/usr/bin/env python3
"""Behavioral regression for the deployable public static-output contract."""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "automation/check_public_static_output.py"
ORIGIN = "https://alegria.test"


def check(output: Path, *, should_pass: bool, reason: str = "") -> None:
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(output)],
        capture_output=True,
        text=True,
        check=False,
    )
    output_text = result.stdout + result.stderr
    if (result.returncode == 0) != should_pass:
        raise AssertionError(f"unexpected checker verdict: {output_text}")
    if reason and reason not in output_text:
        raise AssertionError(f"expected failure {reason!r}: {output_text}")


def site(output: Path) -> None:
    (output / "topic").mkdir(parents=True, exist_ok=True)
    pages = [
        {
            "page_node_key": "home",
            "revision_id": "home-rev",
            "canonical_url_path": "/",
            "status": "published",
            "title": "Home",
        },
        {
            "page_node_key": "topic",
            "revision_id": "topic-rev",
            "canonical_url_path": "/topic",
            "status": "published",
            "title": "Topic",
        },
    ]
    (output / "alegria-static-manifest.json").write_text(
        json.dumps({"base_url": ORIGIN, "page_count": len(pages), "pages": pages}),
        encoding="utf-8",
    )
    (output / "robots.txt").write_text("User-agent: *\\nAllow: /\\n", encoding="utf-8")
    (output / "sitemap.xml").write_text(
        '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
        f"<url><loc>{ORIGIN}/</loc></url>"
        f"<url><loc>{ORIGIN}/topic</loc></url></urlset>",
        encoding="utf-8",
    )
    for filename, url in [("index.html", "/"), ("topic/index.html", "/topic")]:
        (output / filename).write_text(
            f'<html lang="ru"><head><link rel="canonical" '
            f'href="{ORIGIN}{url}"></head><body><h1>Useful</h1></body></html>',
            encoding="utf-8",
        )


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="alegria-public-contract-") as temp:
        output = Path(temp)
        site(output)
        check(output, should_pass=True)

        (output / "topic/index.html").unlink()
        check(output, should_pass=False, reason="manifest page has no HTML artifact")
        site(output)

        (output / "topic/index.html").write_text(
            f'<html lang="ru"><link rel="canonical" href="{ORIGIN}/wrong">',
            encoding="utf-8",
        )
        check(output, should_pass=False, reason="canonical URL does not match manifest route")
        site(output)

        (output / "sitemap.xml").write_text(
            f"<urlset><url><loc>{ORIGIN}/</loc></url></urlset>", encoding="utf-8"
        )
        check(output, should_pass=False, reason="sitemap URLs do not match manifest")
        site(output)

        (output / "extra").mkdir()
        (output / "extra/index.html").write_text("<html></html>", encoding="utf-8")
        check(output, should_pass=False, reason="unlisted HTML artifact")
        (output / "extra/index.html").unlink()

        # Non-HTML files were previously accepted and could be uploaded to
        # public Cloudflare Static Assets without appearing in the manifest.
        (output / "export.json").write_text('{"private":"must-not-publish"}', encoding="utf-8")
        check(output, should_pass=False, reason="unlisted file in public output")
        (output / "export.json").unlink()

        (output / "extra/data.txt").write_text("unapproved", encoding="utf-8")
        check(output, should_pass=False, reason="unlisted file in public output")
        (output / "extra/data.txt").unlink()

        # The immutable marker is intentionally private and is permitted only
        # as release metadata; other hidden files are not.
        (output / ".alegria_static_site").write_text(
            "alegria_immutable_release@1" + chr(10), encoding="utf-8"
        )
        check(output, should_pass=True)
        (output / ".alegria_static_site").unlink()
        (output / ".private").write_text("do not publish", encoding="utf-8")
        check(output, should_pass=False, reason="unlisted file in public output")
        (output / ".private").unlink()

        manifest_file = output / "alegria-static-manifest.json"
        manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
        manifest["page_count"] = 1
        manifest_file.write_text(json.dumps(manifest), encoding="utf-8")
        check(output, should_pass=False, reason="manifest page_count")
    print("PUBLIC_STATIC_OUTPUT_COMPLETENESS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
