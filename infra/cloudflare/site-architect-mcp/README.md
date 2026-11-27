# siteArchitect remote OAuth MCP resource Worker — code-only release gate

This isolated Cloudflare Worker is **not deployed or user Start-ready**. The
canonical repository is `iamaman11/alegria-site`, Draft PR #95; `main` and
`alegria-seo-staging` must remain untouched during this pilot.

## Boundary

- Native `createMcpHandler` supplies the stateless `/mcp` transport.
- `OAuthResourceServer` publishes protected-resource metadata and delegates
  token validation to a separately deployed OAuth 2.1 Authorization Server
  over the `AUTH_SERVER` Service Binding. The authorization server must
  authenticate a **real user** with PKCE S256 and consent, validate the exact
  resource audience, and provision its `props.userId` from a real IdP.
- Required exact scope `site_architect.run_one` is **also enforced** on each
  handler request. Neither a tool argument nor a model-provided bearer,
  tenant, principal, policy, operation, source, grant, endpoint or budget may
  establish authority.
- The only mutating tool accepts an opaque `request_id` UUID. `ONE_STEP_ADMISSION`
  is an optional **private service binding**; no binding means no Start.
  It receives only a 30-second HMAC-SHA256 proof over the OAuth issuer,
  resource, verified subject, exact scope, request UUID and fresh nonce.
  The signing key is a Cloudflare **secret**, never JSON arguments, a model
  response, config text or source control. Missing key blocks the handoff.
- Native `--operator-start-signed-granted-single-v1` independently checks the
  signature, expiry, one configured subject and exact private intent, then
  reuses the existing one-use SQL grant and Temporal operator. The native
  verifier uses a separate, private copy of the 32-byte key; the OAuth
  subject is pinned by the operator, never selected from request JSON.
- A live HTTP/VPC/Tunnel transport from the private Service Binding to this
  native operator is **not implemented here**. An eventual bridge must
  validate its trusted private caller and securely deliver the signed proof
  to the correct single-subject operator, where SQL checks the configured
  tenant/principal/ACL against the exact one-use grant, source,
  plan, operation, context and budget; atomically reserve and record the
  ambiguous-outcome state; and then hand off to the **existing native Temporal
  operator**. Binding alone is not authorization. Never expose the backend as
  a publicly callable proxy or trust unsigned identity headers.
- `request_reserved` means only that a bounded request was reserved, **not**
  that Temporal Start was accepted or work completed. Any transport error or
  malformed admission reply is `start_outcome_unknown`; **never auto-retry**.
  Only independent SQL and Temporal server History inspection can reconcile.

## Build / tests (no Cloudflare release)

```sh
npm ci
npm run check
npm test
npm run build
```

`npm run build` calls native `cf deploy --dry-run` and must not deploy. The
Worker uses `cloudflare.config.ts` (no parallel Wrangler config).

The default config has `workersDev: false`, no routes, no OAuth endpoint
variables and no admission binding. Deploying that default **does not expose
a public MCP endpoint**. To prepare a separate authorized deployment the
operator must explicitly set:

| Environment configuration | Trust requirement |
| --- | --- |
| `SITE_ARCHITECT_ENABLE_PUBLIC_MCP=yes` | Explicit public release switch, not a grant to Start |
| `SITE_ARCHITECT_MCP_RESOURCE_URL` | Exact externally reachable HTTPS URL ending `/mcp` |
| `SITE_ARCHITECT_OAUTH_ISSUER_URL` | HTTPS issuer that signed in and consented a real user |
| `SITE_ARCHITECT_AUTH_WORKER` | Name of the separately verified auth server Worker providing `validateToken` |
| `SITE_ARCHITECT_PRIVATE_ADMISSION_WORKER` | Separately vetted private admission Worker; required to enable the public route, otherwise Start blocked |

The resource Worker also requires secret `ADMISSION_SIGNING_KEY_HEX`: exactly
64 lowercase hex digits (32 cryptographically random bytes). Provision it
using Cloudflare secret management, never as a `SITE_ARCHITECT_*` public
config variable. The operator must receive the same secret in a separate
private, permission-restricted file via
`SITE_ARCHITECT_SIGNED_ADMISSION_V1_KEY_FILE`, plus operator-owned
`SITE_ARCHITECT_SIGNED_ADMISSION_V1_ISSUER`,
`SITE_ARCHITECT_SIGNED_ADMISSION_V1_RESOURCE`, and
`SITE_ARCHITECT_SIGNED_ADMISSION_V1_SUBJECT`. These settings bind **one**
authenticated user to one independently credentialed SQL Start principal;
general multi-user ACL selection is not implemented and remains a gate.

These are not interchangeable with a tunnel, SQL connection string, raw token,
or public HTTP URL. An operator on a private Windows host will need an
independently secured private route (Tunnel, VPC or another approved bridge)
**after** this resource Worker. That path and the real authorization server do
not exist in this PR. Do not set the release switch just to see MCP tools.

## Mandatory acceptance before user launch

1. Provision and inspect IdP, real sign-in + consent, issuer, exact resource,
   PKCE S256, token expiry, revocation, scope and per-user ACL.
2. Implement a private only admission service and independently test a stolen,
   forged, cross-tenant, revoked, repeated and expired subject/grant.
   The CI fixture signer for the private native operator is *not* a real
   OAuth login or a production identity-provider acceptance test.
3. Physical isolated PG18 + Temporal 1.32 trial: source + owner ValidateOnly,
   real one-use grant, no duplicate Start, exact History, budget/receipts.
4. Verify Chrome/ChatGPT remote MCP user connection, including a negative
   access test; promote **individual** operations only with canonical CI
   evidence. Arbitrary groups have an additional dependency/partial-failure
   acceptance and are **not** enabled by this resource server.

The existing Rust 12-tool stdio MCP catalog and 5 installed contract owners
remain the only authoring/source-of-truth implementations. This Worker does
not fork that catalog, create another scheduler, or claim 52 operations ready.
