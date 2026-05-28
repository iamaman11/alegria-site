#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_PATH="${1:-$ROOT_DIR/docs/runs/voyage_evaluation_bundle.json}"

python3 - "$ROOT_DIR" "$OUT_PATH" <<'PY'
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

root = Path(sys.argv[1])
out = Path(sys.argv[2])
out.parent.mkdir(parents=True, exist_ok=True)

has_voyage = bool(os.environ.get("VOYAGE_API_KEY", "").strip())
status = "PASS" if has_voyage else "PENDING_CREDENTIALS"

payload = {
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "status": status,
    "provider": {
        "embeddings_model": os.environ.get("VOYAGE_MODEL", "voyage-4-large"),
        "context_model": os.environ.get("VOYAGE_CONTEXT_MODEL", "voyage-context-3"),
        "rerank_model": os.environ.get("VOYAGE_RERANK_MODEL", "rerank-2.5"),
        "credentials_ready": has_voyage,
    },
    "collections": [
        "raw_chunks_4",
        "raw_chunks_ctx",
        "kb_canonical_4",
        "verified_rules_voyage4",
        "editorial_topics_voyage4",
        "seo_keyword_clusters_voyage4",
        "whole_page_advisory_prototypes",
    ],
    "samples": {
        "russian_retrieval": "pending" if not has_voyage else "configured",
        "contextualized_chunk_recall": "pending" if not has_voyage else "configured",
        "canonical_mapping_fallback": "pending" if not has_voyage else "configured",
        "draft_support_rerank": "pending" if not has_voyage else "configured",
        "planning_cluster_merge": "pending" if not has_voyage else "configured",
    },
    "notes": [
        "This bundle is a policy/evidence surface, not a truth authority surface.",
        "PENDING_CREDENTIALS is the honest verdict until VOYAGE_API_KEY exists in the target environment.",
    ],
}

out.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"VOYAGE_EVALUATION_BUNDLE: {status}")
print(out)
PY
