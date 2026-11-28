# Experimental admitted Qdrant keyword projection V1 — NO-GO

This is a **second real provider-mutation pilot with a new V7 native Temporal
child route**, not an accepted generalized business-operation executor and
**not a production cutover**. Legacy V3/V4/V5 Workflow Histories and the
canonical Outbox dispatcher remain unchanged.

## Precisely what is implemented

- `20261125_0010_qdrant_projection_operator_ledger.sql` plus forward-only
  `20261125_0012_qdrant_projection_immutable_receipts_and_revocation.sql`
  and the matching fresh-install `schema.sql` define three PostgreSQL
  relations: immutable operator command authority, durable RPC-attempt
  uncertainty barriers, and append-only receipts.
- Each admission binds exact serialized command bytes (SHA-256), a real run,
  immutable tenant/context ownership, keyword and parent/child Temporal
  identities. The opt-in V7 operator now independently retrieves **both exact
  server Histories** and proves the reciprocal parent initiation/start pair,
  child server-origin parent identity, task queues and typed commands before
  inserting this immutable row. The PostgreSQL LOGIN must be provisioned for
  that tenant; no untrusted tenant GUC is accepted.
- The typed Rust boundary admits only a single operation:
  `seo_keyword_clusters_4`/`keyword_cluster` with a 1024-dimensional finite,
  nonzero cosine vector and fixed tenant provenance fields. Neither endpoint,
  arbitrary collection, SQL statement nor Workflow Type is caller-supplied.
- Before the first HTTP/gRPC Qdrant write, PostgreSQL durably inserts an
  `uncertain` attempt. The Qdrant point UUID equals the immutable admission
  UUID. The `(tenant, context, keyword)` key admits **one** immutable effect,
  preventing later revisions from silently overwriting earlier effects.
- An attempt remains permanently `uncertain`: SQL no longer mutates its state
  to `confirmed`. Confirmation is **derived from an independently persisted,
  append-only receipt**. This applies to newly written attempts without
  rewriting or discarding historical V6 `confirmed` rows. Even an
  accidentally overgranted worker cannot `UPDATE` or `DELETE` attempts or
  receipts to reopen a provider RPC or hide its previous outcome.
- Before the first attempt, the *same authenticated, tenant-bound issuer LOGIN*
  may revoke an admission exactly once. The database enforces a monotonic
  NULL-to-server-timestamp `revoked_at` transition and rejects all other
  authority-field changes, wrong issuer, and repeated revocations. An
  operator must receive specifically audited `SELECT` and `UPDATE` rights
  to revoke; the migration grants neither production role automatically.
  Admission-row locking serializes revocation against `begin`: a revocation
  that wins prevents the RPC, but **revocation after `begin` cannot cancel an
  already-started provider effect**. Such an effect must still be reconciled
  with an immutable receipt after exact provider readback.
- After the real provider upsert, a separate Qdrant REST retrieval compares
  **all 1024 cosine-normalized float coordinates** within documented Qdrant
  f32 arithmetic tolerance (absolute `1e-5`) and the **exact provider
  payload**, including the approved original-command SHA-256. Qdrant's Cosine
  distance normalizes the stored vector; raw input float equality would
  produce false mismatches. Only then does
  the dedicated PostgreSQL function insert a durable receipt. A successful
  write followed by a failed receipt stays uncertain; a later call may
  **read back and reconcile**, but cannot automatically issue another write.
- A container-backed PG + Qdrant E2E exercises SQL receipt failure *after*
  real Qdrant success, independently checks the Qdrant REST point and gRPC
  vector search, repairs the SQL receipt without replay, rejects cross-tenant
  access, rejects direct wrong-tenant/same-issuer SECURITY DEFINER calls,
  tests revoke-before-begin, and checks an intentionally overgranted worker
  cannot erase or rewrite the receipt/attempt ledger. An `uncertain` write
  with a missing remote point is never retried automatically.
- V7 adds `AlegriaQdrantProjectionDagV1Workflow` on the provider-free parent
  queue `alegria-qdrant-dag-v1`, and a separate
  `temporal_qdrant_effects` executable registering
  `QdrantKeywordProjectionV1Workflow` on `alegria-qdrant-command-v1`
  plus the actual Qdrant Activity on `alegria-qdrant-effects-v1`. A dedicated
  operator CLI performs the two-History proof and signals the child only
  after committing admission. A new isolated four-container E2E covers
  Temporal + PostgreSQL + Qdrant source, provider readback, status-race
  denial and an independently authenticated operator. The E2E is **authored
  but must pass authoritative Rust CI** before this route counts as proven.
- The native Qdrant process now links the narrow `qdrant_effects_core` crate,
  not graph/Voyage-linked legacy `projection_delivery`. The latter reexports
  the old public path for compatibility.

## Explicit production blockers

1. **Native V7 Temporal code is not yet independently accepted.** The new
   parent/child server-History operator, replay routes and full-stack E2E
   require clean locked Cargo builds, genuine live integration and offline
   History replay. The V7 typed parent sequences **Qdrant commands only**;
   it does not yet compose arbitrary PG V5 and Qdrant owners in the same DAG.
   The earlier V6 direct-SQL pilot remains test-only and makes no independent
   claim of Temporal provenance.
2. **Trust boundary of `confirm_qdrant_projection_v1`:** PostgreSQL cannot
   independently query Qdrant. A trusted dedicated worker must perform exact
   provider readback and only then invoke `confirm`. The `TRUE` SQL value is
   **not** independent provider attestation. A compromised role able to call
   `confirm` could lie. This migration deliberately grants **no** `EXECUTE`
   privilege to any application role.
3. A possible late Qdrant write after a timeout makes a missing readback
   ambiguous. The attempt must remain `uncertain` and requires manual
   resolution; there is intentionally no automatic resend or silent expiry.
   The one-shot business key cannot be reused while this ambiguity remains.
4. The vector is operator-supplied and not yet bound to an independently
   attested embedding/Truth revision. No durable source snapshot or
   compensating delete for provider corruption has been proven.
5. PostgreSQL superusers and table owners can bypass tenant RLS, as with the
   existing V5 rollout. Dedicated least-privilege LOGINs, operational audit,
   historical migration and physical service isolation are not complete.

## Opt-in / fail-closed controls

- The executor refuses operation unless
  `ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1=1`.
- The native Worker additionally requires
  `ALEGRIA_ENABLE_QDRANT_EFFECT_V1=true`, a dedicated tenant-bound PostgreSQL
  secret, and two distinct pinned Temporal Task Queues. Independent operator
  issuance additionally requires `ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1=true`
  and a **separate** tenant-bound operator PostgreSQL secret.
- Newly created admission tables and both SQL functions revoke PUBLIC access.
  There is **no** production worker grant, production cutover or ingress route.
  The new Workflow registrations exist only on explicitly opt-in separate
  queues. A DBA can provision **separate** tenant-bound operator and executor
  LOGINs in a disposable environment for the pilot.
- Never infer that a `confirmed` SQL row alone proves live Qdrant state. The
  Rust executor rechecks the provider's exact current bytes on every return,
  including repeated completed calls.

## Next required P0 step

Pass the new real Temporal + PostgreSQL + Qdrant E2E and SDK History replay in
CI, independently verify the operator's History proof, then introduce a
new versioned **mixed PG/Qdrant operation DAG** with proven ordered and
concurrent branches, Truth revision fencing, and tenant-denied cases. Until
those checks pass #94 remains **NO-GO** and PR #95 remains **Draft**.
