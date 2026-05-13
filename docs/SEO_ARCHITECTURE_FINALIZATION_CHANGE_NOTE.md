# SEO Architecture Finalization Change Note

## Summary

This change finalizes the Rust SEO architecture migration to the 7-layer model:

- `primitives`
- `seo_domain`
- `seo_steps`
- `seo_ports`
- `seo_application`
- `infrastructure`
- `services/*`

The main outcome is that SEO semantics, orchestration, and adapters now have one stable ownership model across runtime code, automation checks, and documentation.

## What Changed

- Renamed `seo_core` to `seo_domain` and fixed downstream dependencies.
- Renamed `use_cases` to `seo_steps` and kept it as a pure deterministic step layer.
- Introduced `seo_ports` as the application-to-infrastructure contract surface.
- Introduced `seo_application` as the single SEO orchestration layer.
- Moved Temporal SEO workflow paths to `seo_application + seo_ports`.
- Moved CLI SEO mutation paths to `seo_application`.
- Removed obsolete direct business paths from activities and stale runtime wrappers.
- Updated CI and architecture guards to enforce the new layer graph.
- Reworked stale SEO verification checks to validate the new owner chain instead of legacy direct adapter calls.

## Architectural Decisions

- `seo_domain` is the only owner of canonical SEO identity, applicability, and rebuild semantics.
- `seo_steps` may implement deterministic transforms, but not orchestration, ports, persistence, or service entrypoints.
- `seo_application` is the only orchestration layer for SEO runtime flows.
- `seo_ports` is the only allowed bridge from application logic into infrastructure implementations.
- `services/*` act as composition roots and runtime boundaries, not semantic owners.

## Verification

The final state was accepted only after:

- `automation/ci_verify.sh` passed end to end
- `SQLX_OFFLINE=true cargo check -q` passed in `app/rust`
- targeted Rust tests for `seo_application`, `seo_steps`, `temporal_worker`, and `cli_tools` passed

## Team Impact

- New SEO runtime logic should be added to `seo_application`, not to Temporal activities or CLI binaries.
- New deterministic SEO transforms should go to `seo_steps`.
- New SEO semantics should go to `seo_domain`.
- New infrastructure integrations should be exposed through `seo_ports` and implemented in `infrastructure`.

## ADR Follow-up

If the team wants a formal ADR, this change note can be promoted into an ADR describing:

- the 7-layer Rust architecture,
- semantic ownership boundaries,
- the runtime orchestration contract `activities -> seo_application -> seo_ports -> infrastructure`,
- and the CI rules that prevent regression to the old mixed model.
