# Alegria: owners and Rust boundaries (#43)

This is the short source-linked map for the ongoing modularization. The five
capabilities below are *responsibilities*, not five mandatory processes or five
mandatory crates. A target marked **planned** is not yet an implemented API.
The current dependency graph is evidence of coupling, not the desired design.

| Owner | Public Rust API and state | Allowed dependencies / current migration |
| --- | --- | --- |
| Core | `primitives::{stable_id,source_key,page_key,concept_key,context_key,hash}`: shared IDs and value objects; no mutable domain state | Shared data types and deterministic libraries only. `contracts`, `scraper`, `regex`, `serde` and editorial writer/validator dependencies removed from Core. **Planned:** move remaining SEO-specific identity and dormant Truth code. |
| Acquisition | `acquisition::html_sections::{extract_meta_typed,extract_sections_typed,HtmlSection}` and `acquisition::pdf_parser`: parse source documents without judging their truth | `primitives` for stable hashes, parser libraries. Existing fetch/store effects remain in `infrastructure::adapters::raw_crawl_adapter` until their own adapter-boundary slice. No dependency on Truth, SEO or Publishing. |
| Truth / review | **Implemented:** `truth_domain::{truth_candidates,fact_verifier,truth_governance,contradictions,review_decisions,evidence,procedural_candidates,persisted_candidates,semantic_truth_write}` owns validated candidates, governance, contradictions, scoped review veto, evidence, procedural mapping, persisted eligibility/canonical selection and per-semantic-slot Truth write decisions. CLI retains its JSON transport wrapper. **Planned:** remaining review operations | Temporal orchestrates source-registry reads and SQL/projection writes and now consumes a typed Truth write outcome; `infrastructure` retains SQL persistence and JSON conversion. `primitives::facts_extractor` remains dormant (not exported). |
| Knowledge / retrieval | **Planned:** `seo_ports::knowledge` and `seo_application::knowledge` for typed query/projection operations; PostgreSQL owns authoritative facts | Existing `graph_neo4j`, `vector_qdrant` and concrete materialization adapters implement effects. Search/ranking never changes a Truth decision. No SQL/Neo4j/Qdrant SDK types in public application operations. |
| SEO / editorial | Existing `seo_domain::{writer,block_validator}` owns deterministic HTML composition and typed block QA; `seo_application` owns content operations. **Planned:** typed draft/revision decisions | Reads confirmed facts and search results through application operations; cannot write Truth state or publish directly. JSON decoding stays in `infrastructure::adapters::editorial_block_validation_adapter`; mixed `seo_steps` and remaining `primitives::seo` logic are later ownership moves. |
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

**Working slices:** HTML/PDF acquisition parsing moved from Core into
`acquisition`, with active crawl consumers and tests. Truth extraction,
governance and write outcomes now belong to `truth_domain`; SQL effects stay
in adapters. Editorial HTML rendering and block validation moved from Core
into `seo_domain`, with the active Temporal caller and Rust migration guard.
Core no longer pulls HTML parser or serialization libraries for editorial QA.
The remaining SEO identity and Publishing ownership work is not marked done.
