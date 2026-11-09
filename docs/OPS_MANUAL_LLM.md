# Manual LLM mode — one canonical Temporal workflow

This is a **draft feature**, not a claim of production readiness. In the canonical
`SeoSiteBuildCanonicalCutoverWorkflow`, editorial generation can use this ChatGPT
conversation without an API key. The previous deterministic and API provider
adapters remain available. No extra container, HTTP server or secondary workflow
is introduced.

## Configure

Apply `app/db/migrations/20261109_0001_manual_llm_jobs.sql` once to an
existing PostgreSQL database (the initial `app/db/schema.sql` contains the
same idempotent definition for new databases). Keep the ordinary Temporal worker
and all production gates running.

Set `SEO_LLM_PROVIDER=chat_manual` on the **Temporal worker** (not the CLI).
When the canonical workflow reaches editorial generation, the existing activity
persists an immutable typed input and returns `awaiting_manual`. Temporal
durably waits; it never substitutes generated content or an empty success.

Build and run the **existing** Rust CLI from `app/rust`:

```bash
cargo run -p cli_tools -- llm-next
```

It prints either `NO_PENDING_LLM_JOBS` or a JSON job with
`task_id`, `schema_version`, `input_sha256`, `operation` and `payload`.
Pass `payload.request` and supporting verified source material to the
LLM for editorial section generation. Preserve the job IDs unchanged.

A response file has exactly this envelope (replace the placeholders and choose
**actual section roles** listed in the job's editorial brief):

```json
{
  "task_id": "<from llm-next>",
  "schema_version": 1,
  "input_sha256": "<from llm-next>",
  "response": {
    "sections": {
      "<actual-role>": "Markdown grounded in the verified supporting evidence"
    }
  }
}
```

Submit it with:

```bash
cargo run -p cli_tools -- llm-submit --response-file response.json --model-key gpt-6
```

`DATABASE_URL` and `TEMPORAL_URL` use the existing environment variables.
CLI also accepts `--database-url`, `--temporal-url` and
`--temporal-namespace` as needed. Connection requires access to the
same database and Temporal namespace as the canonical worker. ChatGPT cannot
be invoked autonomously as a service through a consumer subscription; this is
a supervised exchange via existing chat tools.

The command validates the envelope and section output using the existing Rust
editorial generator/overlay, persists the result, and sends the dedicated
`llm_result_ready` Temporal signal. If signaling fails, the submitted result
remains committed; re-running the **identical** command safely re-signals it.
A different answer or model for a completed task is rejected.

The signal **never** approves publication. The existing editorial normalization,
content validation, draft QA, CMS review and distinct human approval path remain.

## Acceptance status and limitations

- The current manual adapter handles **canonical editorial generation**.
- Existing provider-independent truth extraction JSON parsing is hardened to
  reject malformed evidence instead of silently producing an empty successful
  result. The compatibility `raw_knowledge_ingestion` is **not** the canonical
  truth ingestion path, so a manual truth extraction adapter is **not yet wired
  into canonical knowledge verification**. Do not claim full two-operation
  implementation or real first-site acceptance from this PR.
- CI must compile this branch, test invalid hashes/schema versions, duplicate
  responses and replay, and run an exact Temporal + PostgreSQL E2E before merging.
  No production workflow-history replay safety is asserted: adding activities
  and commands requires standard worker versioning/drain for in-flight histories.
- A working DataForSEO/Voyage pipeline, independently verified source claims
  and human approval remain required for the first real published site.
