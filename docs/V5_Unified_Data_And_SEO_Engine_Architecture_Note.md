# V5 Unified Data And SEO Engine Architecture Note

**Status:** short target-architecture note  
**Purpose:** define how the Data Engine and SEO Engine coexist inside one native platform architecture for the expert super-site.

## 1. Core Decision

The system should be built as **one unified architecture**, not as two disconnected products.

Inside that unified architecture there are two native subsystems:

- `Data Engine`
- `SEO Engine`

They must share one knowledge foundation, but they must not collapse their responsibilities into one undifferentiated layer.

## 2. Target Flow

```text
Sources / SERP / Competitor Sites / Expert Inputs
    -> Data Engine
    -> Truth / Graph / Retrieval Foundation
    -> SEO Engine
    -> CMS / Super-Site
```

Expanded form:

```text
SERP + competitor sites + expert inputs
    -> ingestion / extraction / canonicalization / verification
    -> canonical truth + derived graph + retrieval surfaces
    -> site structure / opportunities / linking / briefs / drafts / QA
    -> CMS publish path
    -> expert super-site
```

## 3. What Data Engine Owns

`Data Engine` owns:

- source ingestion
- extraction
- canonicalization
- evidence binding
- verification
- truth enrichment
- graph and retrieval preparation

It answers:

- what is known?
- what is verified?
- what entities, rules, topics, patterns, and signals exist?

## 4. What SEO Engine Owns

`SEO Engine` owns:

- page opportunities
- deterministic page taxonomy
- site structure
- internal linking
- page briefs
- page blueprints in use
- draft assembly
- QA and publish readiness
- rebuild and refresh decisions for SEO artifacts

It answers:

- what pages should exist?
- how should the site be structured?
- what should link to what?
- what draft should be assembled from verified support?

## 5. What They Share

Both subsystems share one common foundation:

- canonical entities
- verified truth references
- core graph surfaces
- retrieval surfaces
- scope model
- registries
- versioning and audit discipline

This is the native shared layer that lets both systems work together without becoming separate architectures.

## 6. What Must Stay Strictly Separated

### Data Engine must not own

- page ownership decisions
- SEO page taxonomy decisions
- publishable page drafts as canonical outputs
- CMS publication logic

### SEO Engine must not own

- truth creation
- truth mutation
- competitor signals promoted into truth
- evidence verification rules
- canonical entity identity

## 7. Boundary Rule

`SEO Engine` may derive from truth and derived signals.

It may not write back into truth as an authority source.

This is the essential condition that allows the two subsystems to live inside one architecture without corruption of the knowledge base.

## 8. Correct Mental Model

Do not think about this as:

- one app for data
- another app for SEO

The correct model is:

- one platform architecture
- one knowledge foundation
- two native engines with different ownership

In compact form:

```text
Data Engine creates and maintains knowledge.
SEO Engine turns that knowledge into the super-site.
CMS publishes the approved result.
```

## 9. Recommendation For This Project

Build the platform as:

- shared foundation
- Data Engine on the left side of the flow
- SEO Engine on the right side of the flow
- CMS / super-site as the serving endpoint

Do not split them into separate temporary architectures.
Do not merge them into one ownerless logic layer.

That middle path is the correct expert architecture.
