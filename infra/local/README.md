# Local Runtime Credentials

Этот каталог держит **локальные** runtime credentials, которые не должны уходить в git.

Канонический файл для business DB:

- `infra/local/dev_db.env`

Канонический файл для локальных runtime/provider credentials:

- `infra/local/dev_runtime.env`

Минимальный usage:

```bash
cd /home/bose/projects/alegria-site
set -a
. infra/local/dev_db.env
set +a
```

После этого `DATABASE_URL` и `ALEGRIA_DATABASE_URL` указывают на локальную business DB для automation и локальных runtime-команд.

Если нужен полный local runtime env:

```bash
cd /home/bose/projects/alegria-site
set -a
. infra/local/dev_db.env
. infra/local/dev_runtime.env
set +a
```

## Windows staging — existing Docker Desktop Hyper-V on B:

### One-command Docker stack lifecycle (GitHub-only)

`windows_stack.ps1` is the operator's **single entry point** for four routine
actions on Docker Desktop (Hyper-V, B:). It never compiles an image on Windows.
The two application Workers are pulled as exact-source GitHub Actions artifacts,
SHA256 checked, and GitHub-attestation verified **before container deletion**.
The five siteArchitect native-owner release images are imported independently
with `windows_site_architect_images.ps1`; importing them is not permission to
start protected pilot effects or alter Cloudflare policies.

Use a clean `git` checkout of the **current main** on `B:`. Supply the SHA
of the most recent *certified Worker image release* to `-Commit`; it may be an
ancestor of main when later commits only change operator scripts or docs.
Thus lifecycle-only changes do not trigger another Rust image build. For example:

```powershell
$source = '<40-character-current-main-SHA>'
$workerRelease = '<40-character-certified-worker-release-SHA>'
$script = 'B:\Alegria\runtime\alegria-site-' + $source + '\infra\local\windows_stack.ps1'

# Inspect without changes.
pwsh -NoProfile -File $script -Action Status

# One-time clean installation with isolated, new Docker volumes. Existing
# Alegria containers are removed, but all old volumes and DB dumps survive.
pwsh -NoProfile -File $script -Action Install -Commit $workerRelease `
  -ReplaceOldContainers -BackupDirectory 'B:\Alegria\secrets\docker-reinstall-20261128'

# Future updates (no volume recreation; outbox first, then Temporal Worker).
pwsh -NoProfile -File $script -Action Update -Commit $workerRelease

# Remove only this project's containers and networks; all volumes survive.
pwsh -NoProfile -File $script -Action Remove
```

`Install` uses only an explicit ten-service Compose allowlist, including the
two Temporal initialization one-shots, rather than activating every service in
`docker-compose.yml`. Every `up` uses `--no-build --pull never`; optional
analytics, Grafana, evaluation profiles and release owners cannot be compiled
or launched accidentally. It checks the installer against current GitHub main,
the Worker release as an ancestor of main, and both signed
Worker releases, binds host ports to loopback, preserves an inventory of old
volumes, then applies **only post-baseline SQL migrations** selected by the
inclusive marker in `app/db/schema.sql`, before starting either Worker. This
avoids replaying historical tenant constraints already folded into initdb.
It verifies the resulting eight runtime containers. If the old
project's volumes or project name already exist, use `Update` or a **new**
Compose project name for another clean installation; a repeat of `Install`
never silently reuses a previous database.

`Update` delegates to the existing guarded `windows_staging.ps1` release
promotions and does not run database migrations. Incompatible schema changes
still require a separately tested migration or an explicit new clean install.

`Remove` uses `docker compose down --remove-orphans` only for the exact project
name—never `-v` or `docker volume prune`. It does not touch other Docker
projects. `Status` reads project containers and volume count only.

### Per-Worker guarded staging

`windows_staging.ps1` is the lower-level, guarded **single-Worker promotion**
called by `windows_stack.ps1 -Action Update`. It uses the Windows Docker CLI and `gh`
(the canonical remote `iamaman11/alegria-site`), no WSL or second service.
Run it from any exact-commit GitHub checkout on `B:`:

```powershell
pwsh -NoProfile -File infra/local/windows_staging.ps1
pwsh -NoProfile -File infra/local/windows_staging.ps1 -Action Deploy -Commit <CI-green-full-40-character-GitHub-SHA> -Service temporal-worker
pwsh -NoProfile -File infra/local/windows_staging.ps1 -Action Deploy -Commit <CI-green-full-40-character-GitHub-SHA> -Service outbox-worker
```

**Rollback uses the same Deploy command** with the previously installed
CI-green SHA shown by Status, rather than an additional deployment mode.
The script requires the single exact-commit `ci` workflow's `CI` gate to
succeed, detects the live project from PostgreSQL's Compose label, preserves
existing named volumes, and retrieves source only from canonical GitHub.

For `temporal-worker`, CI builds and smoke-tests the real Docker image once
for the **accepted PR HEAD**, publishes an immutable per-run artifact for
that exact source SHA, and Windows **downloads, SHA256-checks, loads, verifies the immutable
image ID and source labels, then retags it** for the existing Compose project.
Before Docker load, the installer also requires a valid GitHub Sigstore/SLSA
attestation for the **archive's bytes**, signed by exactly this repository's
`.github/workflows/ci.yml` workflow; checksum metadata alone is not trusted.
No new Windows Rust/Docker compilation occurs on the normal candidate path.
After merging the PR, deploy by its accepted **PR-head SHA**, not the distinct
two-parent merge SHA. The `main` gate checks provenance without compiling or
pretending that the PR image was built from the merge commit.
A missing, expired or mismatched image artifact is a hard NO-GO: never fall back
to an uncertified local candidate build. The previous worker image is preserved
under its rollback tag before retagging the candidate. The previous-image
recovery path can still rebuild **only the previously deployed source** if its
image metadata has disappeared; its seven required runtime files must match
the running container before it can become a rollback candidate.

`outbox-worker` now has a separate accepted-main `package-outbox` release owner
inside the **same** `ci.yml`. Dispatch `worker-image` on `main` to build both
Worker images independently on GitHub; each emits its own immutable archive,
SHA256 manifest and signed GitHub provenance. The Windows installer refuses
to compile either Worker when its release artifact is missing. Each deployment
uses `docker compose ... up --no-build` for container recreation and verifies
the selected image, source checkout, environment and runtime readback.
**Never** run `docker compose down -v` during update/rollback.

This is intentionally a **no-active-runs staging** procedure: if the
`pipeline.execution_runs` registry is nonempty, Deploy refuses to restart
a worker. Genuine production runs require a separate supervised drain
and backup/restore acceptance, not weakening this restriction. CI alone
does not prove a live content release or approved HITL.

For future real providers, copy the committed **template only**
`infra/local/dev_runtime.env.example` to a private file **outside the repo**,
for example `B:\Alegria\secrets\staging.env`, populate values through an
approved secret channel, restrict file ACLs to the administrator, and pass
`-EnvFile B:\Alegria\secrets\staging.env` to Deploy. Compose resolves this
env-file; the script never prints its contents. Without secrets, provider
preflight must remain `NOT_READY` (not faked `PASS`).

Do not confuse the long-running Docker containers with Git HEAD: Status
prints each container's actual source checkout; different historical SHAs
may coexist for unchanged infrastructure services without a second project.
An old source SHA whose full CI was cancelled is **not** considered green;
it cannot be redeployed using this guarded process until properly certified.

### Restore testing without risking existing data

`infra/backups/restore_drill.sh` restores real business and Temporal backups
to **temporary databases in the same existing PostgreSQL containers**.
It compares table inventories and selected row counts, then drops only
those temporary databases. It never drops schemas in the running
`alegria` or `temporal` databases, never starts a second Docker project,
and does not run fixture-seeding production gates. Separate
`restore_business_pg.sh` and `restore_temporal_pg.sh` remain destructive
recovery tools and are **not** a routine staging diagnostic.

**Destructive disaster recovery** is deliberately separate from the isolated
drill. The two `restore_*_pg.sh` commands reject invocation without
`ALEGRIA_ALLOW_DESTRUCTIVE_RESTORE=YES` and a nonempty dump file.
Only use that explicit confirmation in a supervised maintenance/incident
procedure with a separate verified backup, never in a routine chat health check.
