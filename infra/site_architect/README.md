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
