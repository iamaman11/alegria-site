# V5 SEO Identity And Applicability Hardening Plan

**Status:** current-seo-execution-plan  
**Purpose:** decision-complete hardening plan for separating truth identity, applicant-profile applicability, and publishing/search scope in the live SEO runtime.

---

## 1. Why This Document Exists

The live codebase is directionally correct: it already separates `context_key` from `scope_signature`, keeps verified rules under `verified.*`, and routes pages through the SEO workflow and CMS publish path.

The main remaining weakness is semantic drift around three concepts:

- `context_key`
- `applicant_profile`
- `visa_subtype`

Without hardening, the system risks encoding profile, regulatory subtype, and editorial packaging inside the same field family. That creates rebuild noise, weak traceability, and eventually corrupts truth boundaries.

This document is the live-aligned execution plan for fixing that gap.

Normative ownership still remains with:

- `V5_SEO_Foundation_Contracts.md`
- `V5_SEO_Live_Schema_Design_Spec.md`
- `V5_SEO_Runtime_Step_Contracts_Spec.md`
- `SEO_SUPERSITE_10_10_EXECUTION_PLAN.md`
- `V5_SEO_Implementation_Readiness_Gate.md`

---

## 2. Canonical Identity Model

### 2.1 Truth identity

Truth identity answers:

> Which normative reality do these verified rules belong to?

Source of record:

- normalized truth tuple
- derived `context_key`
- `kb.visa_contexts`
- `verified.rule_instances`

Canonical truth tuple fields:

- `country_code`
- `visa_family`
- `regulatory_track` or temporary `visa_subtype`
- `citizenship_code`
- optional future `filing_jurisdiction`
- optional future `residency_bucket`

Truth identity must not include:

- `locale`
- `market`
- `intent pack`
- editorial audience wording
- search packaging

### 2.2 Applicability/profile layer

Applicability answers:

> Which verified rules apply to which applicant class?

Source of record:

- `kb.applicant_profiles`
- `verified.rule_instance_profiles`
- `verified.rule_exceptions`

Applicability semantics:

- `profile_key` is registry-backed and canonical
- `applicability` is one of `applies|excludes|conditional`
- `condition_json` carries narrow conditional logic only
- exception overrides such as `replace_value`, `waive`, `add_requirement`, and `remove_requirement` must never be silently flattened into SEO-only scope logic

Examples of truth-relevant profiles:

- `standard`
- `minor`
- `student`
- `family`

### 2.3 Publishing/search identity

Publishing/search identity answers:

> Which search slice and page package are we building?

Source of record:

- `SeoScopePayload`
- `site.site_scopes`
- derived `scope_signature`

Scope fields:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile` only as a validated pointer to applicability
- future `audience_lens`
- optional future `intent_pack`

Publishing/search identity must not redefine normative truth.

---

## 3. Current Live Gaps

Before hardening, the live implementation had these issues:

1. `applicant_profile` was accepted as a free-form string in workflow-start paths.
2. Non-`standard` profile values could be implicitly converted into `visa_subtype`.
3. Verified support loading did not filter by profile applicability.
4. `load_verified_support_bundle` used a delimiter-based string request that was unsafe because `context_key` itself contains `|`.
5. `kb.applicant_profiles` existed, but the starter/runtime path did not enforce registry-backed use strongly enough.

---

## 4. Mandatory Invariants

These rules are mandatory for live code and future changes.

1. `locale` stays out of `context_key`.
2. `context_key` changes only when the truth tuple changes.
3. `scope_signature` changes only when the scope tuple changes.
4. `applicant_profile` may be part of `SEO scope`, but only as a validated reference to a registry profile.
5. `applicant_profile` must never be implicitly converted into `visa_subtype`.
6. `visa_subtype` is reserved for regulatory subtype semantics only.
7. Profile-specific truth behavior must be modeled in applicability tables, not hidden in SEO-only scope strings.
8. Support bundles must be loaded from verified truth and then filtered by profile applicability before draft assembly.

---

## 5. Live Runtime And API Changes

### 5.1 Starter and workflow-start boundary

The workflow-start path now requires:

- explicit `applicant_profile`
- optional explicit `visa_subtype`
- no implicit profile-to-subtype derivation

Accepted profile registry for phase 1:

- `standard`
- `minor`
- `student`
- `family`

Behavioral rules:

- unknown profile values must fail fast
- deprecated profile values must fail fast
- registry-backed validation is mandatory before input persistence

### 5.2 Truth identity derivation

The raw truth tuple must be treated as:

- `country_code`
- `visa_family`
- `visa_subtype`
- `citizenship_code`

The system may store this tuple for forensics inside startup payload metadata, but only the truth fields may influence `context_key`.

### 5.3 Scope validation

Every SEO site-build input must validate:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`
- reproducible `scope_signature`

The live runtime must reject any mismatch between:

- stored scope fields
- recomputed normalized `scope_signature`

### 5.4 Support bundle resolution

The support bundle loader must:

1. load verified truth by `context_key`
2. filter by profile applicability
3. honor exclusion and waiver exceptions
4. persist the resolved support bundle as a runtime artifact

This means draft, QA, and publish stages consume already-resolved support, not raw unfiltered truth.

---

## 6. Schema And Persistence Impacts

Existing schema objects already support the target model:

- `kb.visa_contexts`
- `kb.applicant_profiles`
- `verified.rule_instance_profiles`
- `verified.rule_exceptions`
- `site.site_scopes`

No immediate schema rewrite is required for phase 1 hardening.

Phase 1 relies on:

- strict runtime validation
- seeded applicant profile registry entries
- scope-signature reproducibility checks
- profile-aware support loading

Future schema work may add:

- explicit `regulatory_track` naming if the rename is worth the migration cost
- explicit `audience_lens`
- optional truth tuple dimensions such as `filing_jurisdiction` or `residency_bucket`

---

## 7. Migration Strategy

### Phase 1: freeze and gate

- restrict accepted `applicant_profile` values
- block arbitrary strings
- remove implicit `profile -> subtype` behavior
- keep `visa_subtype` explicit and optional

### Phase 2: profile-aware truth consumption

- seed profile registry entries
- enforce registry-backed profile validation
- route support loading through `verified.rule_instance_profiles`
- honor waiver/removal exceptions in support selection

### Phase 3: future semantics cleanup

- reserve `audience_lens` for editorial packaging
- consider renaming `visa_subtype` to `regulatory_track`
- extend truth identity only when business rules prove the need

---

## 8. Rebuild Semantics

The system must treat rebuild triggers by identity layer.

- `truth_change`
  - rebuild all impacted pages that depend on changed support
- `profile_applicability_change`
  - rebuild only pages whose support bundle differs for that profile
- `scope_change`
  - rebuild SEO artifacts only
- `locale_change`
  - new scope, no truth identity change
- `regulatory_track change`
  - new truth identity
- future `audience_lens change`
  - new scope only

---

## 9. Test And Acceptance Plan

The hardening is only accepted when tests cover:

- valid `standard` profile normalization
- rejection of unknown profile values
- same truth tuple plus different locale yields same truth identity and different scope
- same truth tuple plus different allowed profile yields same truth identity and different scope
- explicit regulatory subtype changes `context_key`
- starter/runtime no longer derive subtype from profile implicitly

Operational acceptance criteria:

- no workflow-start path accepts arbitrary profile strings
- no workflow-start path derives truth subtype from profile implicitly
- `context_key` remains reproducible from truth-only fields
- `scope_signature` remains reproducible from scope-only fields
- support bundles are resolved through applicability-aware filtering

---

## 10. Rollout Order

1. Freeze accepted `applicant_profile` values.
2. Remove implicit `visa_subtype_from_profile()` behavior.
3. Require explicit truth-identity vs scope-identity validation at starter/bootstrap boundaries.
4. Resolve support bundles through profile applicability.
5. Add identity-invariant tests.
6. Update readiness and execution docs after code and tests pass.
7. Treat `audience_lens` as a separate follow-up, not part of phase 1.

---

## 11. Current Phase-1 Defaults

- default pilot profile: `standard`
- default allowed profile registry: `standard|minor|student|family`
- `locale` remains SEO-only unless future business rules prove otherwise
- `visa_subtype` remains the live field name, but must be interpreted as regulatory subtype only
- `audience_lens` is postponed until profile/applicability semantics are fully stable
