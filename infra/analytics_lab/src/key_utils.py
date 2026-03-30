from __future__ import annotations

import blake3


def content_hash_v1(text: str) -> str:
    return blake3.blake3(text.encode("utf-8")).hexdigest()


def normalize_context_key(raw: str) -> str:
    return raw.strip().lower().replace(" ", "_")


def stable_rule_instance_id(parts: list[str]) -> str:
    return content_hash_v1("|".join(parts))
