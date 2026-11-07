# Alegria SEO staging — native Cloudflare Static Assets

**Canonical source:** `iamaman11/alegria-site`, Draft PR #26. This is only the
isolated *deployment adapter* for the generated site, not a new site builder or
a replacement for the canonical Temporal SEO workflow.

## Isolation

- Cloudflare account: `Pvisakp@gmail.com's Account` (the account owning the
  `alegria.by` zone).
- Dedicated Worker: `alegria-seo-staging`.
- Dedicated hostname: `https://seo-staging.alegria.by`.
- Never modify `staging.alegria.by` (CRM), `mish.alegria.by`, `www.alegria.by`,
  or the zone apex. No Pages project, nginx, R2, or extra container is required.
- The only deployed input is the complete immutable `*-published` directory
  produced by the canonical Rust renderer **after real human approval**.

## Native CLI / explicit deployment

The supported provider is **Cloudflare Workers Static Assets** with the `cf`
CLI pinned in `package.json`. The deployment command uses the existing
canonical public static-output validator and the existing independent HTTPS
byte-for-byte readback. A synthetic fixture can only run in offline CI; it is
**not** production evidence.

Install CLI dependencies once under this directory; do not commit any credentials.
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

## Important integration boundary — remains NO-GO

The existing Rust finalizer in PR #26 currently switches a **local filesystem**
symlink and checks public HTTPS. It does not yet call this Cloudflare deployment
adapter or reconcile a remote Workers deployment on crash/retry.

Therefore **do not** set `published` simply because `cf deploy` returned zero
or because local files changed. The single canonical publish workflow must
still enforce actual approved truth, independent public HTTPS readback, and
replay-safe reconciliation between Workers deployment and PostgreSQL before
production GO.

The native Cloudflare deployment path must be connected to the finalizer and
physically tested with both a first publication and a replacement/rollback.
Tracking: Issue #3 (hosting), #6 (E2E), #10 (master NO-GO), #19 (evidence).
