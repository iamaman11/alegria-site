#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"

REQUIRED_MESSAGES = [
    "SeoScopePayload",
    "SeoVerifiedFactSupportState",
    "SeoDraftSectionState",
    "SeoTraceabilityEntryState",
    "SeoPublishBlockerState",
    "SerpPatternState",
    "KeywordClusterState",
    "PageNodeState",
    "PageBlueprintState",
    "PageBriefState",
    "DraftState",
    "LinkRecommendationState",
    "CannibalizationConflictState",
    "ContentGapState",
    "SeoSiteBuildInputPayload",
    "SerpIngestInputPayload",
    "SerpIngestOutputPayload",
    "SerpNormalizeInputPayload",
    "SerpNormalizeOutputPayload",
    "OpportunityBuildInputPayload",
    "OpportunityBuildOutputPayload",
    "IaBuildInputPayload",
    "IaBuildOutputPayload",
    "LinkRecommendInputPayload",
    "LinkRecommendOutputPayload",
    "DraftAssembleInputPayload",
    "DraftAssembleOutputPayload",
    "DraftQaInputPayload",
    "DraftQaOutputPayload",
    "CmsPublishInputPayload",
    "CmsPublishOutputPayload",
    "RebuildDetectInputPayload",
    "RebuildDetectOutputPayload",
]

REQUIRED_META_FIELDS = [
    "string retry_class",
    "string executor_version",
    "string derivation_version",
    "string scope_signature",
    "uint32 max_retries",
]


def main() -> int:
    proto = PROTO.read_text(encoding="utf-8")
    failures: list[str] = []
    for message in REQUIRED_MESSAGES:
        if f"message {message}" not in proto:
            failures.append(f"missing `{message}`")
    for field in REQUIRED_META_FIELDS:
        if field not in proto:
            failures.append(f"StepContractMeta missing `{field}`")
    if failures:
        print("SEO_PROTO_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_PROTO_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
