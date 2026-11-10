#!/usr/bin/env python3
"""Offline behavioral tests for independent HTTPS content-hash acceptance."""
from __future__ import annotations

import json
import tempfile
from pathlib import Path
from unittest.mock import patch

from check_public_http_readback import verify

ORIGIN = "https://public.alegria.test"


class Response:
    status = 200

    def __init__(self, url: str, payload: bytes):
        self.url = url
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def geturl(self) -> str:
        return self.url

    def read(self) -> bytes:
        return self.payload


def must_fail(root: Path, response_map: dict[str, bytes], *, redirect: bool = False) -> None:
    def open_url(request, timeout):
        url = request.full_url
        destination = "https://foreign.example/" if redirect else url
        return Response(destination, response_map[url])
    try:
        with patch("urllib.request.urlopen", side_effect=open_url):
            verify(root, timeout=2.0)
    except ValueError:
        return
    raise AssertionError("unsafe/incorrect public readback was accepted")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="alegria-http-readback-") as tmp:
        root = Path(tmp)
        (root / "topic").mkdir()
        artifacts = {
            "index.html": b"<html>Homepage</html>",
            "topic/index.html": b"<html>Topic</html>",
            "sitemap.xml": b"<urlset/>",
            "robots.txt": b"User-agent: *\n",
        }
        urls = {
            "index.html": ORIGIN + "/",
            "topic/index.html": ORIGIN + "/topic",
            "sitemap.xml": ORIGIN + "/sitemap.xml",
            "robots.txt": ORIGIN + "/robots.txt",
        }
        (root / "alegria-static-manifest.json").write_text(
            json.dumps({"base_url": ORIGIN, "pages": [
                {"canonical_url_path": "/", "status": "published"},
                {"canonical_url_path": "/topic", "status": "published"},
            ]}), encoding="utf-8",
        )
        for name, content in artifacts.items():
            (root / name).write_bytes(content)
        response_map = {urls[name]: content for name, content in artifacts.items()}

        def open_url(request, timeout):
            return Response(request.full_url, response_map[request.full_url])

        with patch("urllib.request.urlopen", side_effect=open_url):
            report = verify(root, timeout=2.0)
        if report["published_page_count"] != 2 or len(report["verified_urls"]) != 4:
            raise AssertionError("valid public readback was not fully certified")

        tampered = dict(response_map)
        tampered[ORIGIN + "/topic"] = b"<html>Wrong content</html>"
        must_fail(root, tampered)
        must_fail(root, response_map, redirect=True)
        (root / "topic/index.html").unlink()
        must_fail(root, response_map)

    print("PUBLIC_HTTP_READBACK_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
