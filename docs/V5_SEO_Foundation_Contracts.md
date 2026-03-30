# V5 SEO Foundation Contracts

**Status:** working draft companion protocol  
**Purpose:** canonical owner of cross-document SEO ownership, artifact semantics, identity, storage, scope, and registry governance.

---

## 1. Relation To V2

This document extends `V5_Ultimate_Extraction_Protocol_V2.md` for product-layer SEO artifacts.

This document does **not** redefine:

- truth-plane semantics,
- retrieval-plane semantics,
- serving-plane semantics,
- graph merge identity formulas owned by V2,
- ontology governance owned by V2,
- validation and publish-gate rules owned by V2.

If this document conflicts with V2 on any V2-owned rule, V2 wins.

---

## 2. Canonical Precedence And Conflict Rule

The precedence chain is:

`V5_Ultimate_Extraction_Protocol_V2.md > SEO companion owner > non-owner reference`

Companion conflict handling:

- if a rule class is owned here, sibling SEO documents must reference it instead of redefining it,
- if a sibling document needs stricter behavior, it may add stricter local gating, but may not change this document's base contract,
- if two sibling documents appear to assign different meaning to the same artifact, this document's artifact classification and storage placement win.

---

## 3. Authority Matrix

| Rule class | Canonical owner |
|---|---|
| extraction / truth / retrieval / serving meaning | `V5_Ultimate_Extraction_Protocol_V2.md` |
| authority matrix and conflict precedence | this document |
| artifact classification and plane semantics | this document |
| deterministic identity for SEO artifacts | this document |
| storage placement for SEO artifacts | this document |
| page scope model | this document |
| SEO registries and governance posture | this document |
| SERP reliability rules | `V5_SERP_Intelligence_Protocol.md` |
| page taxonomy / intent / URL / linking / cannibalization | `V5_SEO_Information_Architecture_Protocol.md` |
| blueprint / draft / lifecycle / traceability / rebuild / QA | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| escalation / metrics / feedback loop / automation safety | `V5_SEO_Operations_And_Optimization_Protocol.md` |

---

## 4. Artifact Classification Matrix

### 4.1 Canonical classes

Allowed SEO artifact classes in this companion set:

- `product_derived_graph_object`
- `retrieval_artifact`
- `serving_artifact`
- `planning_artifact`
- `operational_control_artifact`

SEO artifacts are not truth-core objects unless V2 explicitly says otherwise. Competitor-derived signals never become truth-core objects through this document set.

### 4.2 Matrix

| Artifact | Primary class | Allowed projections | Truth-core allowed |
|---|---|---|---|
| `KeywordCluster` | `product_derived_graph_object` | `retrieval_artifact` | no |
| `PageNode` | `product_derived_graph_object` | `serving_artifact` | no |
| `SERPPattern` | `planning_artifact` | `retrieval_artifact` | no |
| `PageBlueprint` | `planning_artifact` | `retrieval_artifact` | no |
| `LinkRecommendation` | `planning_artifact` | none | no |
| `Draft` | `serving_artifact` | `planning_artifact` | no |
| `CannibalizationConflict` | `operational_control_artifact` | none | no |
| `ContentGap` | `planning_artifact` | `product_derived_graph_object` | no |
| `SearchFeature` | `planning_artifact` | `retrieval_artifact` | no |
| `SectionTemplate` | `planning_artifact` | `retrieval_artifact` | no |

### 4.3 Hard rules

- No SEO artifact may promote truth status.
- No competitor-derived or SERP-derived object may be treated as verified procedural fact.
- `Draft` is never a truth object.
- `PageBlueprint` is never a publishable fact object.
- `CannibalizationConflict` is a control finding, not a graph fact.

---

## 5. Deterministic Identity Matrix

Every persistent SEO artifact must have:

- `artifact_type`
- `artifact_key`
- `schema_version`
- `owner_document`
- `scope_signature`
- `derivation_version`

### 5.1 Canonical key rules

| Artifact | Deterministic identity rule |
|---|---|
| `KeywordCluster` | hash of `market + locale + scope_signature + normalized_seed_keyword + intent_type + clustering_version` |
| `PageNode` | hash of `scope_signature + page_type + dominant_intent + canonical_slug` |
| `SERPPattern` | hash of `market + locale + pattern_type + normalized_pattern_signature + observation_window + pattern_version` |
| `PageBlueprint` | hash of `page_type + dominant_intent + scope_class + blueprint_version` |
| `LinkRecommendation` | hash of `source_page_key + target_page_key + link_role + anchor_strategy + scoring_version` |
| `Draft` | hash of `page_node_key + blueprint_key + truth_snapshot_ref + assembly_version + draft_revision` |
| `CannibalizationConflict` | hash of `sorted(page_key_a,page_key_b) + conflict_reason + detector_version` |
| `ContentGap` | hash of `scope_signature + normalized_missing_topic + evidence_snapshot_ref + detector_version` |
| `SearchFeature` | hash of `market + locale + normalized_query + feature_type + capture_date` |
| `SectionTemplate` | hash of `page_type + section_role + template_version` |

### 5.2 Hard rules

- Identity must be reproducible from normalized inputs only.
- Runtime-generated UUIDs may exist as storage row ids, but are not canonical artifact identity.
- Changing derivation logic requires a version bump in the key inputs or `derivation_version`.
- A change in `scope_signature` always creates a distinct artifact identity.

---

## 6. Storage Placement Matrix

Canonical storage meaning for SEO artifacts:

- `Postgres`: source of record for persistent SEO operational and planning objects
- `Neo4j`: graph projection for graph-eligible derived objects
- `Qdrant`: retrieval projection for retrieval-eligible derived objects
- `CMS`: serving destination for approved drafts and published pages
- `temp_cache`: transient candidate generation and scoring only

| Artifact | Canonical store | Allowed projections |
|---|---|---|
| `KeywordCluster` | `Postgres` | `Neo4j`, `Qdrant` |
| `PageNode` | `Postgres` | `Neo4j`, `CMS` |
| `SERPPattern` | `Postgres` | `Qdrant` |
| `PageBlueprint` | `Postgres` | `Qdrant` |
| `LinkRecommendation` | `Postgres` | `temp_cache` |
| `Draft` | `Postgres` | `CMS` |
| `CannibalizationConflict` | `Postgres` | none |
| `ContentGap` | `Postgres` | `Neo4j`, `Qdrant` |
| `SearchFeature` | `Postgres` | `Qdrant` |
| `SectionTemplate` | `Postgres` | `Qdrant` |

### 6.1 Hard rules

- No SEO artifact may exist only in a projection store.
- `Neo4j` and `Qdrant` are projections for SEO artifacts in this companion set, not their source of record.
- `Draft` may be mirrored into CMS only after draft-level QA passes.
- `temp_cache` may not be treated as durable state.

---

## 7. SEO Page Scope Contract

### 7.1 Canonical fields

Every page-scoped artifact must carry:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`
- `scope_signature`

### 7.2 Scope normalization

- Missing dimension values are forbidden for page contracts.
- A broad scope must be expressed with explicit sentinel values such as `all`, not with silent null.
- `scope_signature` is the hash of the normalized scope tuple.
- The same `canonical_slug` under different `scope_signature` values is a conflict candidate unless IA explicitly allows it.

### 7.3 Hard rules

- No `PageNode`, `PageBlueprint`, `Draft`, `CannibalizationConflict`, or `LinkRecommendation` may exist without `scope_signature`.
- Scope must be evaluated before intent, linking, and URL decisions.
- Different scope signatures may not silently share the same canonical page identity.

---

## 8. SEO Registries And Governance

### 8.1 Controlled registries

This document defines the registry catalog for:

- `page_type`
- `intent_type`
- `search_feature`
- `anchor_strategy`
- `section_template`
- `cta_pattern`

### 8.2 Registry statuses

Allowed statuses:

- `proposed`
- `active`
- `deprecated`
- `blocked_for_new_use`
- `archived`

### 8.3 Owner roles

Base roles:

- `seo_protocol_owner`
- `seo_registry_curator`
- `seo_ia_owner`
- `seo_editorial_owner`
- `seo_ops_owner`
- `domain_reviewer`

### 8.4 Governance rules

- New registry entries require an owner, schema support, and at least one downstream consumer.
- Deprecated entries remain available for replay and audit.
- `blocked_for_new_use` requires a backward-compatibility path.
- Registry conflicts escalate according to the operations protocol.

---

## 9. Required Contract Tables

This document is the owner of:

- `authority_matrix`
- `artifact_classification_matrix`
- `seo_artifact_identity_matrix`
- `seo_storage_placement_matrix`
- `seo_page_scope_contract`
- `seo_registry_catalog`

Sibling documents must reference these tables instead of recreating them.

---

## 10. Canonical Hard Rules

1. No SEO artifact may bypass explicit plane classification.
2. No SEO artifact may be stored without deterministic identity.
3. No SEO artifact may be introduced without a canonical store.
4. No page-scoped SEO artifact may omit `scope_signature`.
5. No sibling companion document may redefine artifact class or storage placement.

---

## 11. Scope And Versioning Addendum

### 11.1 Scope normalization algorithm

All scope-bearing SEO artifacts must derive `scope_signature` from the same canonical normalization function.

Normalization order:

1. trim surrounding whitespace for every scope dimension;
2. reject empty string after trim;
3. map explicit broad scope only through sentinel value `all`;
4. validate each dimension against its registry or allowed format;
5. serialize the normalized tuple in canonical field order;
6. hash the canonical tuple string with `blake3`.

Canonical normalized tuple order:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`

Canonical normalization rules by field:

- `market`: lowercase slug, registry-backed, example `eu`, `pl`, `global`
- `locale`: lowercase BCP-47 style string, example `en-us`, `pl-pl`
- `country_code`: uppercase ISO alpha-2 or sentinel `ALL`
- `visa_type`: lowercase snake case, registry-backed
- `applicant_profile`: lowercase snake case, registry-backed

Forbidden inputs:

- null as implicit broad scope
- mixed registry and free-text values for the same dimension
- case-variant aliases after normalization

Canonical serialized string:

`market=<market>|locale=<locale>|country_code=<country_code>|visa_type=<visa_type>|applicant_profile=<applicant_profile>`

Canonical signature:

`scope_signature = blake3(canonical_scope_tuple_string)`

### 11.2 Scope validation rules

- The raw normalized tuple is the semantic source of record.
- `scope_signature` is a deterministic derivative key, not an independent semantic field.
- Two records with the same normalized tuple must never produce different signatures.
- Two records with different normalized tuples must never share the same signature.
- Every runtime step receiving scope-bearing input must reject non-normalized scope before persistence.

### 11.3 Derivation version policy

Every version-bearing SEO artifact must separate:

- source refresh,
- semantic derivation change,
- storage-row supersession,
- projection rebuild.

A version bump is mandatory when artifact meaning, scoring, conflict behavior, or output contract changes.

A version bump is not mandatory when only source data is refreshed under unchanged logic.

Required bump triggers:

- scoring formula change
- scope normalization logic change
- section-template rule change affecting artifact meaning
- blueprint assembly semantics change
- conflict-detector logic change
- SERP pattern normalization or reliability classification change
- rebuild detector decision logic change

Artifact-specific version fields remain canonical:

- `cluster_version`
- `pattern_version`
- `blueprint_version`
- `assembly_version`
- `scoring_version`
- `detector_version`

Versioning rules:

- backward-compatible additive output rule: bump minor version
- incompatible semantic reinterpretation: bump major version
- audit-only metadata change: no derivation bump required
- source refresh under unchanged logic: new row or update allowed only if artifact identity inputs remain identical by policy

### 11.4 Multi-version artifact coexistence

Old and new derived versions may coexist when needed for:

- audit
- replay
- phased rebuild
- analytics comparison

Coexistence rules:

- only one version per artifact family may be `active_for_new_generation`
- superseded versions remain queryable for audit and replay
- rebuild jobs must use the highest non-deprecated derivation version approved for production
- analytics must distinguish `active`, `superseded`, and `archived` artifact versions explicitly
- no runtime path may silently overwrite an artifact produced by a prior derivation version under the same semantic key

### 11.5 Hard implementation invariants added by this addendum

1. `scope_signature` must always be reproducible from the canonical raw scope tuple.
2. No logic-changing artifact rewrite may occur without a derivation version bump.
3. Multi-version coexistence is allowed for audit, but active-generation ownership must remain singular.
