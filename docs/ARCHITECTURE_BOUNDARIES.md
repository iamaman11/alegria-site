# Alegria: owners and Rust boundaries (#43)

This is the short source-linked map for the ongoing modularization. The five
capabilities below are *responsibilities*, not five mandatory processes or five
mandatory crates. A target marked **planned** is not yet an implemented API.
The current dependency graph is evidence of coupling, not the desired design.

| Owner | Public Rust API and state | Allowed dependencies / current migration |
| --- | --- | --- |
| Core | `primitives::{stable_id,source_key,page_key,concept_key,context_key,hash}`: shared IDs and value objects; no mutable domain state | Shared data types and deterministic libraries only. **Planned:** remove `contracts` and unrelated parsers/SEO/Truth code. |
| Acquisition | `acquisition::html_sections::{extract_meta_typed,extract_sections_typed,HtmlSection}` and `acquisition::pdf_parser`: parse source documents without judging their truth | `primitives` for stable hashes, parser libraries. Existing fetch/store effects remain in `infrastructure::adapters::raw_crawl_adapter` until their own adapter-boundary slice. No dependency on Truth, SEO or Publishing. |
| Truth / review | **Implemented:** `truth_domain::{truth_candidates,fact_verifier,truth_governance,contradictions,review_decisions,evidence,procedural_candidates,persisted_candidates}` owns validated candidate types, governance, contradiction evaluation, scoped review veto, UTF-8 evidence spans, completeness/freshness, procedural mapping, persisted candidate eligibility and canonical selection. CLI retains its JSON transport wrapper. **Planned:** remaining review operations | Temporal still loads raw evidence/review decisions from PostgreSQL and assembles multi-section adjudication; `infrastructure` retains SQL persistence and JSON conversion, but no longer chooses the canonical verified candidate or classifies persisted eligibility. `primitives::facts_extractor` remains dormant (not exported). |
| Knowledge / retrieval | **Planned:** `seo_ports::knowledge` and `seo_application::knowledge` for typed query/projection operations; PostgreSQL owns authoritative facts | Existing `graph_neo4j`, `vector_qdrant` and concrete materialization adapters implement effects. Search/ranking never changes a Truth decision. No SQL/Neo4j/Qdrant SDK types in public application operations. |
| SEO / editorial | Existing `seo_domain` and `seo_application` for SEO/content decisions; **planned:** owned editorial modules with typed draft/revision decisions | Reads confirmed facts and search results through application operations; cannot write Truth state or publish directly. `primitives::{writer,block_validator}` and mixed `seo_steps` logic are later ownership moves. |
| Publishing | **Planned:** `seo_application::publishing` for revision approval, release transition and recovery decisions; owner of publication state | File/SQL/HTTPS/Cloudflare effects remain behind ports. Existing `infrastructure::adapters::sqlx_seo_cms_adapter` contains mixed logic to separate in a complete publishing slice. Approval of a fact, editorial approval of a revision, and successful release are distinct types. |
| Runtime / composition | Existing `services/{temporal,outbox_worker,cli_tools,...}`: wiring, scheduling, retries, telemetry | Calls typed application operations and constructs adapters. Runtime can import application and adapter crates; domain/application cannot import runtime or implementation crates. |

### Rules for every code slice

- A capability owns its state transitions and output types; another capability
  reads the public result, and requests changes through the owner's operation.
  Cross-capability flows are composed in Application, without cyclic imports.
- Domain decisions are ordinary testable Rust functions; external effects are
  isolated at actual ports. Do not introduce generic storage or plugin layers.
- Move the operation *and all active callers/tests* together. No deprecated
  re-export or parallel implementation. The source-controlled Cargo DAG and
  public module signatures must reflect the actual boundary.
- Preserve persisted identifiers/provenance and active workflow data while
  changing code ownership. A historical identifier is not a Rust API alias.

**First working slice:** HTML section extraction and PDF parsing move from
`primitives` into `acquisition`. The active crawl adapter now calls Acquisition;
HTML parsing tests move unchanged with the implementation. `lopdf` and
`ego-tree` are no longer dependencies of Core. The HTML validation in
`primitives::block_validator` still uses `scraper` and belongs to a later
editorial/publishing slice; the Core cleanup is not falsely marked complete.
