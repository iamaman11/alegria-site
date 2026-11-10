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

`windows_staging.ps1` is the single chat-callable operator entry point for the
**existing** Compose deployment. It uses the Windows Docker CLI and `gh`
(the canonical remote `iamaman11/alegria-site`), no WSL or second service.
Run it from any exact-commit GitHub checkout on `B:`:

```powershell
pwsh -NoProfile -File infra/local/windows_staging.ps1
pwsh -NoProfile -File infra/local/windows_staging.ps1 -Action Deploy -Commit <CI-green-full-40-character-GitHub-SHA> -Service temporal-worker
pwsh -NoProfile -File infra/local/windows_staging.ps1 -Action Deploy -Commit <CI-green-full-40-character-GitHub-SHA> -Service outbox-worker
```

**Rollback uses the same Deploy command** with the previously installed
CI-green SHA shown by Status, rather than an additional deployment mode.
The script checks both exact-commit GitHub workflows (`quality-gate` and
`automation-verify`) before changing anything; it detects the live project
from PostgreSQL's Compose label to preserve existing named volumes,
snapshots source only from canonical GitHub, and rebuilds/recreates only the
chosen Rust worker. It saves the previous image under a rollback tag before
building and restores the previous Compose file and image on a failed start.
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
