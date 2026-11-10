# Alegria SEO staging — native Cloudflare Static Assets

**Canonical source:** `iamaman11/alegria-site`, Draft PR #28. This is only the
isolated *deployment adapter* for the generated site, not a new site builder or
a replacement for the canonical Temporal SEO workflow.

## Isolation

- Dedicated Cloudflare account: `site-alegria` (`78e5fcb3762c1473113d51838556da74`).
- Dedicated Worker: `alegria-seo-staging`.
- Technical staging hostname: `https://alegria-seo-staging.alegria-site.workers.dev`.
- The account subdomain is `alegria-site.workers.dev`; `workersDev: true` is
  the only Worker route. No custom domain or DNS zone is required.
- Never modify the `alegria.by` zone or the CRM, OKX, sing-box accounts.
  No Pages project, nginx, R2, or extra container is required.
- The only deployed input is the complete immutable `*-published` directory
  produced by the canonical Rust renderer **after real human approval**.

## Native CLI / explicit deployment

The supported provider is **Cloudflare Workers Static Assets** using the
native `cf` CLI plus the Cloudflare Vite plugin (cf Build Output). The deployment command uses the existing
canonical public static-output validator and the existing independent HTTPS
byte-for-byte readback. A synthetic fixture can only run in offline CI; it is
**not** production evidence.

Install CLI dependencies once under this directory (Node.js 26; `npm install`);
do not commit any credentials. The deployment adapter prepares an ephemeral
public-only directory, omitting `.alegria_static_site` and
`alegria-static-manifest.json`. `cf build` runs against those assets,
then `cf deploy --prebuilt --mode production` publishes the exact build.
There is no raw directory argument to `cf deploy` and no unsupported
`--no-provision` option. CI performs a real CLI dry run without uploading.
Set `CLOUDFLARE_ACCOUNT_ID` to the account ID from `cloudflare.config.ts`.
Provide a scoped `CLOUDFLARE_API_TOKEN` via runtime secrets. The script
has two explicit modes:

```sh
python3 automation/deploy_seo_static_cloudflare.py \
  /path/to/.alegria-static-releases/<release>-published \
  --release-root /path/to/.alegria-static-releases

python3 automation/deploy_seo_static_cloudflare.py \
  /path/to/.alegria-static-releases/<release>-published \
  --release-root /path/to/.alegria-static-releases \
  --previous-release /path/to/.alegria-static-releases/<previous>-published \
  --deploy --report /path/to/independent-readback.json
```

The **first command never publishes**; it only verifies the release. The
second is an explicit staging deployment, with best-effort redeploy of the old
version if deployment/readback fails. A first publication with no previous
release must be checked independently after any ambiguous failure. The script
is *not* a replacement for CMS finalization.

## Canonical Temporal integration — implemented, not physically certified

The Rust finalizer in PR #28 calls the **same** Cloudflare adapter under the
PostgreSQL site-wide publish lock, after recording a durable journal and
before independent HTTPS readback and atomic CMS/outbox commit. On rollback it
attempts to re-deploy the previous version. After an ambiguous COMMIT or
worker crash, worker startup reconciles **both** the Cloudflare deployment and
the local journal against PostgreSQL. A first failed deployment with no prior
release deliberately stops automatic recovery for human inspection.

The Temporal worker image contains Python 3 + the pinned native `cf` CLI as
part of **the existing container**; no new daemon or runtime process.
Environment/secrets are passed by `docker-compose.yml` with empty defaults.
Missing credentials fail closed before changing the live symlink.

**Production decision remains NO-GO.** This code has not been deployed and
physically tested. The system may not report a successful first published site
until the full CI passes and a true expert-approved, non-empty release is
served on `alegria-seo-staging.alegria-site.workers.dev` and verified. First-publication failures
have no remote predecessor: the journal intentionally requires manual
reconciliation rather than claiming a safe rollback.

Tracking: Issue #3 (hosting), #6 (E2E), #10 (master NO-GO), #19 (evidence).
