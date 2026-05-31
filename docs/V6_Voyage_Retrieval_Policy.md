# V6 Voyage Retrieval Policy

**Status:** current runtime satellite  
**Class:** `current-runtime-satellite`  
**Owner:** Alegria retrieval and semantic-support plane  
**Priority rule:** this file owns current-versus-target Voyage usage policy under `V6`. When it conflicts with historical `V5` retrieval wording, this file wins.

---

## 1. Purpose

This file defines:

- which Voyage models Alegria uses by surface;
- which Qdrant collections belong to each retrieval surface;
- parameter policy for `input_type`, `truncation`, vector dimension, and dtype;
- the non-authority boundary between retrieval and truth;
- which parts are active now and which remain target policy for later tranches.

---

## 2. Non-Authority Boundary

Voyage and Qdrant are retrieval support only.

They may:

- improve recall,
- improve clustering,
- improve support ordering,
- widen diagnostics,
- surface semantic neighbors,
- widen planning and rebuild advisory sets.

They may not:

- write `verified.rule_instances`,
- assign `verified` truth,
- override `candidate_validation`,
- override `truth_adjudication`,
- override `publish_*`,
- act as final authority in canonical mapping.

Retrieval may widen analysis. It may not redefine truth authority.

---

## 3. Canonical Model Policy

### 3.1 Active defaults

- general embeddings: `voyage-4-large`
- contextualized chunk embeddings: `voyage-context-3`
- reranker: `rerank-2.5`

Historical defaults such as `voyage-3-large` and `voyage-multilingual-2` are not current target policy.

### 3.2 Parameter policy

- retrieval indexing:
  - `input_type=document`
- retrieval query:
  - `input_type=query`
- peer clustering / similarity among same-class texts:
  - `input_type` omitted
- truth- and draft-adjacent evidence text:
  - `truncation=false`
- primary quality vectors:
  - `output_dimension=1024`
  - `output_dtype=float`

Quantized outputs and reduced dimensions remain optional later optimization lanes, not primary quality policy.

---

## 4. Collection Policy

The current accepted collection families are:

- `raw_chunks_4`
- `raw_chunks_ctx`
- `kb_canonical_4`
- `verified_rules_4`
- `editorial_topics_4`
- `seo_keyword_clusters_4`
- `whole_page_advisory_prototypes`

Legacy SEO retrieval collection names are not accepted in the final runtime contract. `seo_keyword_clusters`, `seo_draft_support_sections`, and `seo_link_targets` are migrated into voyage4 collections and rejected for new `kb.qdrant_points` rows. `ontology` may exist only as a non-canonical concept materialization surface; canonical vector mapping may not use it as a substitute for `kb_canonical_4`.

Named vectors are not part of the accepted first full migration. Alegria uses separate collections per retrieval surface.

## 4.1 Hard-required retrieval contract

When the hard-required retrieval lane is enabled, these env flags govern canonical runtime:

- `RETRIEVAL_CAPABILITY_REQUIRED=true`
- `CANONICAL_VECTOR_RETRIEVAL_REQUIRED=true`
- `CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED=true`
- `VOYAGE_RERANK_REQUIRED=true`

In that mode:

- `seo_preflight` must return `pass` for the retrieval contract;
- missing provider capability, missing collections, stale collections, or incomplete projection state block canonical runtime;
- production gate must fail instead of silently downgrading retrieval quality;
- blocked verdicts must be emitted as machine-readable evidence, not only log lines.

---

## 5. Surface Map

### 5.1 Active now

#### Raw crawl / source context retrieval

- `raw_crawl_adapter` writes:
  - `raw_chunks_4`
  - `raw_chunks_ctx`
- `raw_chunks_4` uses `voyage-4-large` with `input_type=document`
- `raw_chunks_ctx` uses `voyage-context-3` with one list-of-lists request per page, `input_type=document`
- production-quality retrieval lanes are contract-required; missing provider capability yields explicit block status rather than silent projection continuity fallback
- raw chunk projection no longer emits deterministic fingerprint vectors for `raw_chunks_4` or `raw_chunks_ctx`; missing Voyage embeddings/contextualized embeddings hard-block the projection

#### Whole-page semantic advisory

- `whole_page_semantic_pass` uses:
  - `voyage-4-large` prototype retrieval from `whole_page_advisory_prototypes`
  - `rerank-2.5` as optional second-stage advisory rerank
- this lane remains non-authoritative

#### Generic semantic search

- generic text retrieval uses `voyage-4-large`
- query embeddings use `input_type=query`

#### Ontology / canonical concept materialization

- concept vectors may still be materialized into legacy collection `ontology` for migration and replay continuity
- canonical runtime mapping does not use `ontology` as a vector substitute
- canonical vector retrieval targets `kb_canonical_4` only; missing `kb_canonical_4` under required contract is a hard block

#### Voyage4 projection producers

- `verified_truth_write` emits verified rule projections to `verified_rules_4` with `voyage-4-large`, `input_type=document`, `truncation=false`, float 1024-d vectors, and typed Qdrant outbox payloads when Voyage credentials are present; under `RETRIEVAL_CAPABILITY_REQUIRED=true`, missing Voyage credentials hard-block instead of falling back to fake vectors
- `verified_truth_write` deletes demoted/rejected rule projections from `verified_rules_4` ledger and, under the hard-required contract, from Qdrant itself
- `opportunity_build` emits keyword cluster projections to `seo_keyword_clusters_4`
- IA page-node projection, draft assembly, QA persistence, and semantic link search use `editorial_topics_4` instead of `seo_link_targets` or `seo_draft_support_sections`
- Qdrant materialization for `kb_canonical_4`, `editorial_topics_4`, and `seo_keyword_clusters_4` replaces bootstrap fingerprint payloads with `voyage-4-large`, `input_type=document`, `truncation=false`, float 1024-d vectors at dispatch time; missing Voyage credentials block materialization
- `draft_assemble` source-context assembly reads and reranks support blocks in this order: `verified_rules_4`, `raw_chunks_ctx`, `editorial_topics_4`, `raw_chunks_4`
- DB collection contracts migrate legacy `seo_keyword_clusters`, `seo_draft_support_sections`, and `seo_link_targets` ledger rows into voyage4 collection names and reject those legacy names for new `kb.qdrant_points` rows

These producers are active projection surfaces. Their retrieval consumers remain bounded by the step policy below and may not become authority paths.

### 5.2 Target-next, not yet universally active

- `rerank-2.5` inside planning merge/refinement surfaces beyond link recommendation
- live evidence proving every required collection is populated, fresh, and projection-complete under the hard-required retrieval contract

`kb_canonical_4` is now the only accepted canonical vector retrieval surface in runtime policy. The voyage4 projection producers above are active; remaining items here require explicit evidence before they can be marked universally active.

---

## 6. Step Policy

### Step 0

Search-demand clustering uses:

- `voyage-4-large`
- `input_type` omitted
- float 1024-d vectors

This applies to Russian, mixed Russian/English, and multilingual demand text. `voyage-multilingual-2` is not the target model for this surface.

### Steps 1-12

- `seo_preflight` reports retrieval capability readiness
- `crawl_sources` projects both standard and contextualized raw chunk lanes
- `whole_page_semantic_pass` may use advisory retrieval and rerank, but remains non-authoritative

### Steps 13-17

- canonical mapping stays symbolic-first
- once symbolic tiers are exhausted, runtime must resolve against `kb_canonical_4` with rerank; pseudo-vector scoring is not allowed
- under required contract, unresolved vector retrieval yields explicit block state and cannot silently downgrade

### Steps 18-31

- retrieval may improve support recall and diagnostics
- `completeness_judge` now records non-authoritative semantic neighborhood diagnostics (`raw_chunks_ctx` with policy-allowed fallback to `raw_chunks_4`) and reranked evidence refs
- `resolution_loop` now records machine-readable retrieval trace refs from `verified_rules_4` and `kb_canonical_4` for blocked/needs_hitl branches
- `contradiction_gate` now records semantic-neighbor retrieval refs from `verified_rules_4` for conflict review pressure
- retrieval may not assign truth verdicts

### Steps 32-36

- retrieval projection remains downstream-only from admissible runtime artifacts

### Steps 37-42

- planning projection to `seo_keyword_clusters_4` is active
- planning retrieval/rerank expansion beyond existing semantic link search remains target policy, not yet universally active

### Steps 43-56

- draft/topical support projection to `editorial_topics_4` is active
- draft support ordering, unsupported-claim retrieval, plagiarism-risk retrieval, and rebuild neighborhood widening remain target policy unless a specific tranche marks them active

---

## 7. Active vs Historical Docs

Historical `V5` retrieval documents may still describe:

- `voyage-context-3` as the intended retrieval-ready vector surface,
- `kb_canonical` or other older collection naming,
- earlier model defaults.

Those documents are reference or target context only.

Current-runtime truth under `V6` is:

- `voyage-4-large` is the active general embedding default,
- `voyage-context-3` remains a first-class contextualized capability,
- `rerank-2.5` is the accepted reranker target,
- `voyage-multilingual-2` is not the target architecture default.

---

## 8. Machine-Verification Expectations

Policy changes here require alignment with:

- live Rust adapters under `app/rust/crates/infrastructure/src/adapters/**`,
- `temporal_starter` preflight / ontology tooling,
- `OPS_RUNTIME_RUNBOOK.md`,
- `V6_Expert_Truth_Graph_Runtime.md`,
- `V6_SeoSiteBuildWorkflow_Working_Plan.md`,
- Voyage retrieval surface checks in `automation/**`.
