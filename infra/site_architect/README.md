# siteArchitect native owner images — GitHub-only release

The canonical source is `iamaman11/alegria-site`. Windows is an **installer**,
not a Docker or Rust build host. These five native owners are independent
release images, not an alias for the existing Temporal Worker:

| Owner | Immutable release image |
| --- | --- |
| PostgreSQL effects / signed ingress | `alegria-site-architect-pg:<full-source-SHA>` |
| Temporal DAG orchestrator | `alegria-site-architect-orchestrator:<full-source-SHA>` |
| Acquisition | `alegria-site-architect-acquisition:<full-source-SHA>` |
| One-step request issuer | `alegria-site-architect-request-issuer:<full-source-SHA>` |
| Source issuer | `alegria-site-architect-source-issuer:<full-source-SHA>` |

The single `.github/workflows/ci.yml` provides the explicit
`site-architect-images` operation. It is **main-only**, runs after the existing
accepted-PR provenance gate, compiles with pinned Rust inside GitHub Buildx,
and uploads one multi-image Docker archive with exact source SHA, independent
image IDs, archive SHA256 and GitHub Artifact Attestation. An ordinary PR or
documentation-only change does **not** build these images.

After the source changes have passed PR CI and merged into `main`:

1. Dispatch **ci → Run workflow → operation: site-architect-images** on `main`.
2. Select the successful run ID and its full 40-character source SHA.
3. On Windows, using an exact clean checkout of that accepted SHA, execute:

   ```powershell
   pwsh -NoProfile -File infra/local/windows_site_architect_images.ps1 `
     -SourceSha '<40-character-main-SHA>' -RunId <GitHub-Actions-run-ID>
   ```

The installer writes downloads only beneath `B:\Alegria\runtime`, checks
the exact GitHub run and five owner identities, verifies archive checksum and
GitHub attestation **before** `docker image load`, and verifies Docker image
IDs, SHA labels and entrypoints **after** import. It never compiles code,
creates containers, changes Cloudflare policies or grants a Temporal Start.

Cutover of existing pilot containers is a separate guarded operation: inspect
and preserve their current volumes, networks, identity binding, and secrets;
then replace only exact pilot containers with these attested images. Do not
remove live images or declare the MCP E2E/SQL grant accepted merely because
an image import succeeded.

## Windows B: repeatable **transport-only** pilot lifecycle

Once the attested native image is imported and the operator has verified the
dedicated Cloudflare Access app, Worker, Tunnel and VPC Service, run this from
an **exact clean checkout of current GitHub `main`**. `-Commit` names an
attested owner image release already in the history of `main` (not necessarily
the current documentation commit). It must already exist in Docker.

```powershell
pwsh -NoProfile -File infra/local/windows_site_architect_transport.ps1 -Action Status
pwsh -NoProfile -File infra/local/windows_site_architect_transport.ps1 -Action Install -Commit '<40-hex-owner-image-SHA>'
pwsh -NoProfile -File infra/local/windows_site_architect_transport.ps1 -Action Update -Commit '<40-hex-new-owner-image-SHA>'
pwsh -NoProfile -File infra/local/windows_site_architect_transport.ps1 -Action Remove
```

`Install` requires three existing **new** named volumes (signed key, Tunnel
token, empty intents). When any of these are absent, supply both operator-
approved, existing read-only source volume names using `-SourceKeyVolume` and
`-SourceConnectorVolume`; the script copies only `admission.key` and `token`
into separate owner-0600 protected new volumes. It never prints those bytes.
The source volumes and database backups remain untouched. If transitioning
from the two historical non-Compose v2 containers, `Install` additionally
requires `-AdoptExistingTransport` and verifies confinement and empty intents
before removing those **two exact containers only**.

`Update` refuses to run while any private granted-intent file is present;
coordinate real operator starts separately and never replay unknown outcomes.
`Remove` removes only the Compose-owned ingress and connector **containers**;
it does not delete any Docker volume, private key, database or backup. All
operations use `--no-build --pull never` or read-only Docker queries. Windows
never compiles or pulls a mutable native owner release.

This Compose stack intentionally has no SQL credentials, tenant grants,
Temporal connection or published port. Its sole purpose is exercising
Access → Worker → VPC Service → Tunnel → localhost ingress in **fail-closed**
mode. A healthy Tunnel or HTTP 400 on malformed proof is *not* proof of an
accepted one-use SQL grant or a Temporal Start. The remaining four attested
native owners are not started by this transport installer.
