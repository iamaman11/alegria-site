# V5 SEO Information Architecture Protocol

**Status:** working draft companion protocol  
**Purpose:** canonical owner of page taxonomy, intent rules, site hierarchy, canonical URL policy, opportunity prioritization, linking policy, cannibalization handling, localization, and content reuse boundaries.

---

## 1. Relation To Foundation And V2

This document depends on:

- `V5_Ultimate_Extraction_Protocol_V2.md` for plane boundaries and extraction semantics
- `V5_SEO_Foundation_Contracts.md` for artifact classes, scope semantics, identity, and storage
- `V5_SERP_Intelligence_Protocol.md` for reliable SERP-derived opportunity signals

This document does not redefine truth, retrieval, or serving semantics.

---

## 2. Page Taxonomy

Canonical `page_type` registry values for v1:

- `hub`
- `overview`
- `detail`
- `faq`
- `checklist`
- `comparison`
- `troubleshooting`
- `profile_specific`
- `supporting_editorial`

`page_type` controls:

- hierarchy eligibility,
- blueprint eligibility,
- linking obligations,
- content reuse limits,
- canonical URL pattern family.

---

## 3. Intent Model

Canonical `intent_type` values for v1:

- `informational`
- `transactional`
- `navigational`
- `investigative`
- `comparison`
- `troubleshooting`

### 3.1 Dominant intent rule

Every `PageNode` must have exactly one `dominant_intent`.

### 3.2 Secondary intent rule

Secondary intents are allowed only when:

- they do not justify their own page under current scope,
- they do not create URL conflict,
- they remain subordinate to the dominant task of the page.

If a secondary intent becomes independently valuable, the page must split or the subordinate section must be demoted.

---

## 4. Site Hierarchy And Canonical URL Policy

### 4.1 Hierarchy classes

- `hub` pages aggregate a cluster and link outward
- `overview` pages summarize a scoped theme
- `detail` pages solve a single narrow task
- `supporting_editorial` pages explain or support, but do not own transactional intent

### 4.2 Canonical URL rule

A canonical URL is owned by one `PageNode` with one `scope_signature` and one `dominant_intent`.

### 4.3 URL construction inputs

Canonical URL policy must be derived from:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`
- `page_type`
- `dominant_intent`
- `canonical_slug`

Two pages may not share a canonical URL when their `scope_signature` differs unless localization policy explicitly declares them equivalent translations.

---

## 5. Localization Policy

- `locale` defines language/region presentation, not procedural scope.
- `hreflang` is allowed only between equivalent pages with identical scope and dominant intent.
- Different `country_code`, `visa_type`, or `applicant_profile` values do not qualify as translation-only variants.
- A broader-scope page may not canonicalize a narrower-scope page by default.

---

## 6. Opportunity Prioritization Model

`opportunity_score` is a deterministic weighted score from 0 to 100:

- `search_demand`: 30
- `business_value`: 20
- `truth_coverage_availability`: 20
- `competitive_gap`: 15
- `freshness_volatility`: 5
- `operational_cost_inverse`: 10

Gating before scoring:

- scope must be explicit,
- SERP support must be at least `R2_supported`,
- no unresolved cannibalization blocker may exist,
- minimum truth coverage must exist for draftable page types.

If a page fails gating, it is not ranked for creation.

---

## 7. Internal Linking Contract

### 7.1 Required link families

- `hub -> child`
- `child -> hub`
- `overview -> detail`
- `problem -> solution`
- `profile_specific -> general rule`
- `faq/checklist -> source owner page`

### 7.2 Optional link families

- `sibling -> sibling`
- `editorial -> overview`
- `editorial -> detail`

### 7.3 No-link cases

Links must not be recommended when:

- pages are in unresolved cannibalization conflict,
- target is deprecated or blocked,
- source and target have incompatible scope,
- the link exists only to repeat exact-match anchors,
- the target is below minimum freshness or below minimum QA state.

### 7.4 Link scoring

`link_score` uses:

- semantic adjacency: 30
- scope compatibility: 20
- journey continuity: 20
- business priority: 10
- orphan risk reduction: 10
- anchor diversity need: 10

### 7.5 Anchor policy

- `exact_match` anchors may not exceed 30% of links pointing to the same target within the same scope cluster
- the same source page may not repeat the same anchor text to the same target more than once
- `anchor_strategy` must come from the controlled registry

---

## 8. Cannibalization Resolution Policy

A cannibalization conflict exists when two pages share:

- the same `scope_signature`
- the same or effectively identical `dominant_intent`
- materially overlapping URL or blueprint territory

### 8.1 Resolution outcomes

- `merge`
- `split`
- `hierarchy_fix`
- `scope_fix`
- `deprecate_lower_priority_page`

### 8.2 Deterministic precedence

- `detail` page owns narrow task intent
- `hub` page owns aggregate cluster intent
- `overview` owns general scoped intent only when no narrower page owns it
- if two pages still conflict after hierarchy analysis, the page with higher `truth_coverage_availability` wins
- if still tied, the page with higher `opportunity_score` wins

Unresolved same-scope dominant-intent conflicts block publication.

---

## 9. Content Reuse Boundaries

Allowed reuse:

- verified glossary definitions
- stable factual micro-summaries that remain subordinate on the page
- CTA patterns from registry
- standard disclaimer fragments

Restricted reuse:

- dominant-intent sections across pages with the same scope
- H1/H2 structures copied across competing pages
- FAQ blocks duplicated across same-scope pages without canonical ownership

If reused content would make two pages interchangeable for the same dominant intent, reuse is forbidden.

---

## 10. IA Quality Gates

A page is IA-valid only when:

- `scope_signature` is explicit,
- `dominant_intent` is singular,
- canonical URL is unique within scope,
- required hierarchy links are satisfiable,
- no unresolved cannibalization blocker exists,
- localization policy is not violated.

---

## 11. Deterministic Cannibalization Tie-Break Addendum

If two pages still conflict after the current hierarchy and score rules, the final deterministic tie-break chain is:

1. hierarchy owner wins
2. narrower valid scope wins
3. higher `truth_coverage_availability` wins
4. higher `opportunity_score` wins
5. lower canonical URL depth wins
6. lexicographically lower `page_node_key` wins as final deterministic fallback

If the tie-break chain still cannot be evaluated because an upstream field is missing or invalid, publication must block and route to HITL.

---

## 12. Minimum Site Generation System

This document defines the minimum deterministic site-generation logic required on top of the graph and retrieval layers.

The minimum site-generation system must cover:

- deterministic page taxonomy
- dominant intent assignment
- site structure generation
- page opportunity acceptance
- internal linking
- content gap detection as SEO logic
- cannibalization control as SEO logic

### 12.1 Deterministic page taxonomy

The SEO engine must assign every accepted page candidate to one canonical page type before draft assembly.

Minimum page types for the expert baseline:

- `hub_page`
- `overview_page`
- `detail_page`
- `requirement_page`
- `fee_page`
- `timeline_page`
- `faq_page`
- `troubleshooting_page`
- `comparison_page`
- `checklist_page`

### 12.2 Site structure generation

Site structure must be generated from:

- dominant intent
- scope
- hierarchy role
- truth coverage availability
- supported page blueprint
- unresolved cannibalization blockers

A generated structure is valid only when:

- each important cluster has a hub or justified direct-leaf model
- every leaf has a parent or explicit root exception
- no accepted page duplicates the same dominant intent under the same scope
- required hierarchy links are satisfiable

### 12.3 Page opportunity acceptance

A page opportunity may enter the site plan only when:

- SERP support is sufficient
- deterministic page type is assigned
- dominant intent is singular
- scope is valid
- truth coverage is enough for the intended page type
- no blocking cannibalization conflict exists

### 12.4 Internal linking as primary SEO logic

Internal linking is not a cosmetic post-process. It is a primary SEO logic surface.

The minimum linking system must determine:

- required hub-to-child links
- required child-to-hub links
- sibling links when intent continuity exists
- detail links from overview pages
- support-page links from requirement, fee, or timeline pages

### 12.5 Page opportunities and content gaps

`ContentGap` and opportunity logic must answer two distinct questions:

- what page is missing from the site structure?
- what section or evidence coverage is missing inside an existing page?

Both outputs are required for expert-level site growth.
