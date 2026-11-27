# siteArchitect — authenticated one-step launch acceptance (P0)

This is an acceptance contract for issue #94, not an assertion of production
readiness. The only canonical project source is
`https://github.com/iamaman11/alegria-site`. Keep PR #95 in Draft; do not merge
to `main` or enable production Start until independently witnessed end-to-end
evidence exists.

## Two different trust boundaries

The existing `siteArchitect` **stdio MCP** is an authoring and inspection
adapter. It has no independently authenticated ChatGPT human identity. A
recipe's `scope.tenant_id`, a UUID locator, Unix peer UID, PostgreSQL
`session_user`, or a model-provided `principal_id` is **not** a ChatGPT OAuth
subject. The read-only `site_architect_one_step` tool may inspect a single
recipe, but it never reserves a grant or starts Temporal. The name
`site_architect_run_one` is reserved exclusively for the Access-protected
remote request-ID Start tool, which remains unavailable without the private
admission service and an independently authenticated user.

The native `--operator-start-granted-single-v1` route already supports a
separately authorized, **local operator** pilot through SQL one-use grants and
native Temporal Parent/Child workflows. That success cannot be transferred
into a claim that any ChatGPT user can start an operation.

## Required remote authorization boundary

The following must be implemented and physically tested as a separate remote
MCP path before exposing a mutating Start tool:

1. A dedicated Cloudflare Access application with **Managed OAuth** handles
   the MCP client's real user sign-in, consent, OAuth token lifecycle and
   Access policy. Enforce an exact allowed email, application AUD and resource.
   On every directly invoked Worker request require `ctx.access` and the
   Access-verified `user_uuid`; reject absent, invalid and other-app identity.
   Access is the token validator. The one-tool Access app grants exactly the
   `site_architect.run_one` capability by app policy, **not by a separately
   granted per-tool OAuth scope**. No other mutating tools may share this
   policy without a new authorization design.
2. Resolve verified `(issuer, subject)` to tenant, role, permitted operations
   and context in server-owned policy. Deny absent, stale, revoked, ambiguous,
   mismatched and cross-tenant bindings. Do not accept any of these as tool
   arguments, claims relayed by the model, headers from an untrusted proxy or
   values embedded in a recipe.
3. Canonicalize one exact installed operation/version, parameter DTO, input
   artifact references, source provenance and budget. Perform independent
   owner `ValidateOnly` with durable, inspectable evidence before admission.
4. Issue a short-lived, single-use SQL authorization bound to the exact
   authenticated subject, tenant, context, operation, source, plan/command
   digests, policy and budget. The approval/issuer identity must differ from
   the grant consumer. Never put credentials, raw SQL, privileged endpoints or
   private grant material in MCP tool arguments or model output.
5. Hand off only the minimum opaque request locator to the already isolated
   operator. Preserve atomic SQL reservation and committed
   `start_outcome_unknown` **before** Temporal Start. Do not automatically
   repeat a Start after any ambiguous SQL or Temporal acknowledgment.
6. Read back the exact Temporal **server History** and independently authorized
   SQL grant state. Confirm only evidence matching tenant, run, workflow ID,
   plan/source hashes and principal/policy. A client RPC acknowledgment is not
   proof of execution or result, and provider cost/receipt must be attested
   independently for effectful operations.

The current Cloudflare Static Assets deployment is *not* such a remote MCP
server. Deploying or configuring an Access app/IdP, per-user policy and a
secured private operator handoff requires separate release authorization.

An isolated code-only Cloudflare MCP Worker lives at
`infra/cloudflare/site-architect-mcp/`. Its request handler pins a verified
Access app AUD, allowed email and opaque user UUID. The exact one-tool
capability is signed into a 30-second HMAC proof; the native CLI mode verifies
issuer, resource, single subject and request UUID against a separate immutable
operator-owned intent before the existing SQL grant / Temporal path. The
Workers VPC Service binding pins a fixed private destination, but a real
Tunnel/VPC service, private native HTTP ingress and configured Access
application are **not deployed or accepted**. The default has no public route;
a bound request may report only SQL reservation or an ambiguous outcome.
Fixture-signing tests do not establish an actual user login or Start.

## Distinct statuses; never infer success

| Evidence | Allowed conclusion | Forbidden conclusion |
| --- | --- | --- |
| Installed typed contract | Operation can be described and locally composed | Owner Worker is running or user may Start |
| One-step recipe preview | Locally type-compatible plan | Principal/tenant/source/budget has been authenticated |
| SQL grant `unclaimed` | Approved bounded request exists for the SQL subject | Start was sent |
| SQL grant `reserved` | One-use right was claimed | Temporal accepted it |
| SQL grant `start_outcome_unknown` | At most one uncertain Start attempt; reconcile exact History | Safe to retry or automatically mark success |
| Attested matching server History | Exact execution facts visible in History | Provider spend/source integrity/whole workflow accepted without receipts |
| Owner result and independent receipts | Specific effect can be assessed | All 52 operations are user-ready |

## Acceptance evidence for each operation

The typed catalog and an optional 52-row planning matrix are inventories,
**not** a runtime acceptance count. Promote an individual operation to
`user_start_verified` only with evidence for all of:

- actual ChatGPT Managed OAuth/Access user and tenant authorization, including denied
  cross-tenant, expired, revoked and replayed access;
- installed typed input/output/error contracts, independent source evidence,
  runtime owner route and owner `ValidateOnly`;
- exact immutable one-use authorization and enforced resource limits;
- real native Temporal Parent/Child Histories, unknown-outcome recovery,
  no duplicate Starts, correct final business result;
- bounded, attributable external provider receipts where applicable;
- physical E2E on isolated non-production infrastructure, with evidence
  persisted in the canonical repository/CI and independently reviewed.

Group execution additionally requires proof of dependency ordering, result
handoff, separate authorization and budget per effectful child, partial failure
semantics and cancellation. Single-step success never implies group readiness.

The Acquisition source issuer's current dedicated positive proof covers
`acquisition.classify_source_page`; it does **not** authorize a blanket claim
for `acquisition.evaluate_dom_blocks`, the authenticity of the remote website,
or a source policy that remains valid after the last pre-Start SQL check.

**Release state:** local operator pilot only, production NO-GO, and `0/52`
fully authenticated ChatGPT-user-ready operations until the above acceptance
is independently satisfied. This is not a limitation on local typed authoring
or arbitrary offline selection of installed operation descriptions.
