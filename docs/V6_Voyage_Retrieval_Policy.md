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

Legacy collection names may continue to exist during migration, especially `ontology` for concept retrieval materialization and `seo_link_targets` for planning support. They are compatibility surfaces, not the target naming policy.

Named vectors are not part of the accepted first full migration. Alegria uses separate collections per retrieval surface.

---

## 5. Surface Map

### 5.1 Active now

#### Raw crawl / source context retrieval

- `raw_crawl_adapter` writes:
  - `raw_chunks_4`
  - `raw_chunks_ctx`
- `raw_chunks_4` uses `voyage-4-large` with `input_type=document`
- `raw_chunks_ctx` uses `voyage-context-3` with one list-of-lists request per page, `input_type=document`
- when provider credentials are absent, deterministic fingerprint vectors remain a bootstrap-only fallback for projection continuity

#### Whole-page semantic advisory

- `whole_page_semantic_pass` uses:
  - `voyage-4-large` prototype retrieval from `whole_page_advisory_prototypes`
  - `rerank-2.5` as optional second-stage advisory rerank
- this lane remains non-authoritative

#### Generic semantic search

- generic text retrieval uses `voyage-4-large`
- query embeddings use `input_type=query`

#### Ontology / canonical concept materialization

- the current executable materialization path still writes concept vectors into legacy collection `ontology`
- this remains an accepted compatibility surface until `kb_canonical_4` is fully populated in runtime

### 5.2 Target-next, not yet universally active

- `kb_canonical_4` as the canonical vector-fallback collection for canonical mapping
- `verified_rules_4` for truth-support retrieval
- `editorial_topics_4` for draft/planning topical retrieval
- `seo_keyword_clusters_4` for planning cluster retrieval
- `rerank-2.5` inside draft support ordering and planning merge/link refinement
- non-authoritative semantic-loss diagnostics in `completeness_judge`

These are target policy surfaces. They must not be documented elsewhere as already universal runtime behavior until code and evidence land.

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
- vector fallback may propose or route to `needs_hitl`
- vector fallback may not silently verify truth

### Steps 18-31

- retrieval may improve support recall and diagnostics
- retrieval may not assign truth verdicts

### Steps 32-36

- retrieval projection remains downstream-only from admissible runtime artifacts

### Steps 37-42

- planning retrieval/rerank expansion is target policy, not yet universally active

### Steps 43-56

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
