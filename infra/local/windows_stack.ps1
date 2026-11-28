#requires -Version 7.0
<#
One entry point for the GitHub-built Alegria Docker Desktop stack on B:.

  Status  : inspect the project without mutation
  Install : create a fresh, isolated Compose project with new named volumes
  Update  : promote both certified GitHub Workers on the existing project
  Remove  : remove project containers/networks, NEVER volumes

Only the initial Install may remove other Alegria containers, with the explicit
-ReplaceOldContainers switch and an operator-provided backup directory.
No action compiles an image on Windows or uses `docker compose down -v`.
#>
[CmdletBinding()]
param(
    [ValidateSet('Status', 'Install', 'Update', 'Remove')][string]$Action = 'Status',
    [ValidatePattern('^[a-f0-9]{40}$')][string]$Commit,
    [ValidatePattern('^[a-z][a-z0-9-]+$')][string]$Project = 'alegria-site-clean',
    [string]$HomeOnB = 'B:\Alegria',
    [string]$EnvFile = 'B:\Alegria\secrets\staging.env',
    [switch]$ReplaceOldContainers,
    [string]$BackupDirectory
)

$ErrorActionPreference = 'Stop'
$docker = Join-Path $HomeOnB 'docker\app\resources\bin\docker.exe'
$repo = 'iamaman11/alegria-site'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$compose = Join-Path $root 'docker-compose.yml'
$services = @('postgres', 'postgres-temporal', 'pgbouncer', 'neo4j', 'qdrant',
    'temporal-schema', 'temporal-server', 'temporal-namespace', 'outbox-worker', 'temporal-worker')
$running = @('alegria_postgres', 'alegria_postgres_temporal', 'alegria_pgbouncer',
    'alegria_neo4j', 'alegria_qdrant', 'alegria_temporal', 'alegria_outbox_worker',
    'alegria_temporal_worker')
$volumes = @('postgres18_data', 'postgres_temporal18_data', 'neo4j_data',
    'qdrant20_data', 'static_site_releases')

function Assert-Exit([string]$Stage) {
    if ($LASTEXITCODE -ne 0) { throw "$Stage failed (exit $LASTEXITCODE)" }
}
function Invoke-Compose([string[]]$Arguments) {
    # Compose prints routine progress and warnings to stderr. Its exit code and
    # independent Docker readback—not text on stderr—decide the result.
    & $docker @script:composeArgs @Arguments 2>$null
    Assert-Exit "docker compose $($Arguments[0])"
}
function Project-Containers {
    $names = @(& $docker ps -a --filter "label=com.docker.compose.project=$Project" --format '{{.Names}}')
    Assert-Exit 'list project containers'
    return $names
}
function All-Volumes {
    $names = @(& $docker volume ls --format '{{.Name}}')
    Assert-Exit 'list Docker volumes'
    return $names
}
function Assert-Previous-Volumes([string[]]$Before) {
    $after = @(All-Volumes)
    foreach ($volume in $Before) {
        if ($volume -notin $after) { throw "Existing Docker volume was lost: $volume" }
    }
}
function Verify-Running-Stack {
    foreach ($name in $running) {
        $raw = & $docker inspect $name --format '{{json .}}'
        Assert-Exit "inspect $name"
        $container = ($raw | Out-String) | ConvertFrom-Json
        if ($container.State.Status -ne 'running' -or
            $container.Config.Labels.'com.docker.compose.project' -cne $Project) {
            throw "$name is not a running container of $Project"
        }
        if ($container.State.Health -and $container.State.Health.Status -ne 'healthy') {
            throw "$name health=$($container.State.Health.Status)"
        }
        Write-Output "SERVICE=PASS NAME=$name"
    }
    foreach ($name in @('alegria_temporal_schema', 'alegria_temporal_namespace')) {
        $raw = & $docker inspect $name --format '{{json .State}}'
        Assert-Exit "inspect $name"
        $state = ($raw | Out-String) | ConvertFrom-Json
        if ($state.Status -ne 'exited' -or $state.ExitCode -ne 0) {
            throw "$name did not finish successfully"
        }
    }
}

function Apply-Fresh-Business-Migrations {
    # The initdb schema contains an inclusive set of historical migrations.
    # Replaying those SQL files would duplicate tenant constraints. This is
    # the same baseline selection as automation/bootstrap_local_runtime_baseline.py.
    $schemaPath = Join-Path $root 'app\db\schema.sql'
    $matches = [regex]::Matches([IO.File]::ReadAllText($schemaPath),
        '(?m)^-- ALEGRIA_SCHEMA_BASELINE_MIGRATION: ([0-9]{8}_[0-9]{4}_[a-z0-9_]+\.sql)$')
    if ($matches.Count -ne 1) { throw 'No unique fresh schema migration baseline marker' }
    $baseline = $matches[0].Groups[1].Value
    $all = @(Get-ChildItem (Join-Path $root 'app\db\migrations') -Filter '*.sql' -File |
        Sort-Object Name)
    if ($baseline -notin @($all | ForEach-Object Name)) { throw 'Schema baseline migration is missing' }
    $delta = @($all | Where-Object { $_.Name -cgt $baseline })
    if (-not $delta.Count) { Write-Output 'POST_BASELINE_MIGRATIONS=NONE'; return }

    $folder = '/tmp/alegria-migrations-' + [guid]::NewGuid().ToString('n')
    $created = $false
    try {
        & $docker exec alegria_postgres mkdir -p $folder 2>$null
        Assert-Exit 'create temporary migration folder'
        $created = $true
        & $docker cp (Join-Path $root 'app\db\migrations') ('alegria_postgres:' + $folder + '/migrations') 2>$null
        Assert-Exit 'copy canonical SQL migrations into PostgreSQL container'
        $paths = @($delta | ForEach-Object { $folder + '/migrations/' + $_.Name })
        $shell = 'set -eu; for f in ' + ($paths -join ' ') +
            '; do echo "MIGRATION=${f##*/}"; psql -X -q -v ON_ERROR_STOP=1 ' +
            '-U postgres -d alegria -f "$f" >/dev/null 2>' + $folder +
            '/last-error.log; done; echo POST_BASELINE_MIGRATIONS=PASS'
        & $docker exec alegria_postgres sh -ec $shell 2>$null
        $status = $LASTEXITCODE
        if ($status -ne 0) {
            & $docker exec alegria_postgres tail -n 12 ($folder + '/last-error.log') 2>$null
            throw "Post-baseline migration chain failed: exit $status"
        }
        Write-Output "POST_BASELINE_COUNT=$($delta.Count)"
    } finally {
        if ($created) {
            & $docker exec alegria_postgres rm -r $folder 2>$null | Out-Null
            Assert-Exit 'remove exact temporary migration files'
        }
    }
}

if (-not (Test-Path -LiteralPath $docker -PathType Leaf)) { throw 'B: Docker CLI missing' }
& $docker info --format '{{.ServerVersion}}' | Out-Null
Assert-Exit 'Docker Engine readiness'
$script:composeArgs = @('compose', '--env-file', $EnvFile, '-p', $Project,
    '--project-directory', $root, '-f', $compose)

if ($Action -eq 'Status') {
    Write-Output "PROJECT=$Project"
    & $docker ps -a --filter "label=com.docker.compose.project=$Project" `
        --format '{{.Names}} | {{.Status}} | {{.Image}}'
    Assert-Exit 'project status'
    Write-Output "PROJECT_VOLUMES=$(@((All-Volumes) | Where-Object { $_ -like ($Project + '_*') }).Count)"
    exit 0
}
if ($Action -eq 'Remove') {
    if ($ReplaceOldContainers -or $BackupDirectory) {
        throw 'Remove is project-scoped; it does not accept old-container flags'
    }
    $before = @(All-Volumes)
    Invoke-Compose @('down', '--remove-orphans')
    if (@(Project-Containers).Count) { throw 'Project containers remain after Remove' }
    Assert-Previous-Volumes $before
    Write-Output "REMOVE=PASS PROJECT=$Project VOLUMES_PRESERVED=$($before.Count)"
    exit 0
}
if (-not $Commit) { throw "$Action requires -Commit <full GitHub SHA>" }
if (-not (Test-Path -LiteralPath $EnvFile -PathType Leaf)) { throw 'Private EnvFile missing' }
$EnvFile = (Resolve-Path -LiteralPath $EnvFile).Path
if (-not (Get-Command gh -ErrorAction SilentlyContinue) -or
    -not (Get-Command git -ErrorAction SilentlyContinue)) { throw 'GitHub CLI and git are required' }
$installerSha = (& git -C $root rev-parse HEAD | Out-String).Trim()
Assert-Exit 'source checkout SHA'
$modified = & git -C $root status --porcelain -- 'infra/local/windows_stack.ps1' `
    'infra/local/windows_staging.ps1' 'infra/local/windows_staging_image.ps1'
Assert-Exit 'installer source status'
if (($modified | Out-String).Trim()) { throw 'Installer sources have local modifications' }
$remoteSha = (& gh api "repos/$repo/commits/main" --jq .sha | Out-String).Trim()
Assert-Exit 'canonical main SHA'
if ($remoteSha -cne $installerSha) { throw 'Installer must run from current canonical main' }
# Operator-only merges must not require a fresh Rust image build. A previously
# packaged Worker release is valid while it remains in main's ancestry. The
# release artifact is still verified against its own exact source SHA.
& git -C $root merge-base --is-ancestor $Commit $installerSha
if ($LASTEXITCODE -ne 0) { throw 'Worker release SHA is not an ancestor of canonical main' }

# File variables take precedence only when no same-named process environment
# variable is set. Remove inherited shadows; keep the exact deployment ID.
$env:WORKER_BUILD_ID = $Commit
foreach ($line in [System.IO.File]::ReadLines($EnvFile)) {
    if ($line -match '^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=') {
        if ($Matches[1] -ne 'WORKER_BUILD_ID') {
            Remove-Item -LiteralPath ('Env:' + $Matches[1]) -ErrorAction SilentlyContinue
        }
    }
}
Invoke-Compose @('config', '--quiet')
$configuration = ((& $docker @script:composeArgs config --format json 2>$null | Out-String) | ConvertFrom-Json)
Assert-Exit 'parse effective Compose configuration'
foreach ($service in $services) {
    if (-not $configuration.services.PSObject.Properties[$service]) { throw "Missing service: $service" }
    foreach ($port in @($configuration.services.$service.ports)) {
        if ($port -and $port.host_ip -ne '127.0.0.1') { throw "Unsafe host port: $service" }
    }
}

if ($Action -eq 'Update') {
    if ($ReplaceOldContainers -or $BackupDirectory) { throw 'Update never performs a clean install' }
    if (-not @(Project-Containers).Count) { throw 'No installed stack: use Install first' }
    $actualProject = ((& $docker inspect alegria_postgres `
        --format '{{index .Config.Labels "com.docker.compose.project"}}' | Out-String).Trim())
    Assert-Exit 'read installed Compose project'
    if ($actualProject -cne $Project) { throw 'Target project mismatch' }
    foreach ($service in @('outbox-worker', 'temporal-worker')) {
        & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'windows_staging.ps1') `
            -Action Deploy -Commit $Commit -Service $service -HomeOnB $HomeOnB -EnvFile $EnvFile
        Assert-Exit "promote $service"
    }
    Verify-Running-Stack
    Write-Output "UPDATE=PASS SOURCE_SHA=$Commit PROJECT=$Project"
    exit 0
}

# Clean Install. Load both signed image archives BEFORE removing any container.
if ($Action -ne 'Install') { throw 'Unknown operation' }
$existing = @(Project-Containers)
if ($existing.Count) { throw 'Project already installed: use Update or Status' }
foreach ($volume in $volumes) {
    if ((($Project + '_' + $volume)) -in @(All-Volumes)) {
        throw "Project volume already exists: $volume; choose a new project name for a fresh install"
    }
}
$otherNames = @(& $docker ps -a --format '{{.Names}}')
Assert-Exit 'enumerate existing containers'
if ($otherNames.Count -and -not $ReplaceOldContainers) {
    throw 'Other containers exist; explicit -ReplaceOldContainers and backups are required'
}
if ($ReplaceOldContainers) {
    if (-not $BackupDirectory -or -not (Test-Path -LiteralPath $BackupDirectory -PathType Container)) {
        throw 'Existing-container replacement requires a verified backup directory'
    }
    $dumps = @(Get-ChildItem (Join-Path $BackupDirectory 'pg_dumps') -File -Filter '*.dump')
    if ($dumps.Count -ne 3) { throw 'Expected all three PostgreSQL backup dumps' }
    if (@($otherNames | Where-Object { $_ -cnotmatch '^alegria_' }).Count) {
        throw 'Unrelated Docker container exists: refuse engine-wide cleanup'
    }
}

. (Join-Path $PSScriptRoot 'windows_staging_image.ps1')
$runsRaw = & gh run list -R $repo --commit $Commit `
    --json databaseId,workflowName,conclusion,status,headSha --limit 100
Assert-Exit 'list accepted release runs'
$accepted = @((($runsRaw | Out-String) | ConvertFrom-Json) | Where-Object {
    $_.workflowName -eq 'ci' -and $_.status -eq 'completed' -and
    $_.conclusion -eq 'success' -and $_.headSha -ceq $Commit
} | Sort-Object databaseId -Descending)
$releaseRun = $null
foreach ($run in $accepted) {
    $raw = & gh run view ([string]$run.databaseId) -R $repo --json jobs,headBranch
    Assert-Exit 'inspect release packaging jobs'
    $details = ($raw | Out-String) | ConvertFrom-Json
    if ($details.headBranch -ne 'main') { continue }
    if (@($details.jobs | Where-Object { $_.name -eq 'package' -and $_.conclusion -eq 'success' }).Count -eq 1 -and
        @($details.jobs | Where-Object { $_.name -eq 'package-outbox' -and $_.conclusion -eq 'success' }).Count -eq 1) {
        $releaseRun = [string]$run.databaseId
        break
    }
}
if (-not $releaseRun) { throw 'No successful main CI run containing both certified Worker images' }
$downloadRoot = Join-Path $HomeOnB 'downloads'
New-Item -ItemType Directory -Path $downloadRoot -Force | Out-Null
foreach ($service in @('outbox-worker', 'temporal-worker')) {
    $folder = Join-Path $downloadRoot ($service + '-install-' + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    try {
        & gh run download $releaseRun -R $repo -n ($service + '-image-' + $Commit) -D $folder
        Assert-Exit "download certified $service image"
        $image = Read-Staging-Verified-Worker-Image -ArtifactDirectory $folder `
            -ExpectedSha $Commit -Service $service
        $attestation = & gh attestation verify $image.Archive -R $repo `
            --signer-workflow "$repo/.github/workflows/ci.yml" `
            --deny-self-hosted-runners --format json
        Assert-Exit "verify GitHub $service attestation"
        if (-not @((($attestation | Out-String) | ConvertFrom-Json)).Count) {
            throw "$service has no valid GitHub attestation"
        }
        & $docker image load --input $image.Archive | Out-Null
        Assert-Exit "load $service image"
        $nativeTag = if ($service -eq 'outbox-worker') { 'alegria-outbox:ci' } else { 'alegria-seo-temporal:ci' }
        $loadedId = (& $docker image inspect $nativeTag --format '{{.Id}}' | Out-String).Trim()
        Assert-Exit 'inspect loaded Worker ID'
        $label = (& $docker image inspect $nativeTag `
            --format '{{index .Config.Labels "by.alegria.source-head-sha"}}' | Out-String).Trim()
        Assert-Exit 'inspect loaded Worker source label'
        if ($loadedId -cne $image.ImageId -or $label -cne $Commit) {
            throw "$service image ID or canonical source label mismatch"
        }
        & $docker image tag $image.ImageId ($Project + '-' + $service + ':latest')
        Assert-Exit "tag exact $service image"
    } finally {
        Remove-Item -LiteralPath $folder -Recurse -Force
    }
}

$before = @(All-Volumes)
if ($otherNames.Count) {
    $ids = @(& $docker ps -aq)
    Assert-Exit 'list old container IDs'
    $namesNow = @(& $docker ps -a --format '{{.Names}}')
    Assert-Exit 'recheck container identities'
    if (@(Compare-Object $otherNames $namesNow).Count -or $ids.Count -ne $otherNames.Count) {
        throw 'Concurrent container change: refuse cleanup'
    }
    & $docker rm -f @ids | Out-Null
    Assert-Exit 'remove previous Alegria containers without volumes'
    if (@(& $docker ps -aq).Count) { throw 'Container cleanup incomplete' }
    Assert-Previous-Volumes $before
    Write-Output "OLD_CONTAINERS_REMOVED=$($ids.Count) VOLUMES_PRESERVED=$($before.Count)"
}

Invoke-Compose (@('up', '--no-build', '--pull', 'never', '-d') +
    @('postgres', 'postgres-temporal', 'neo4j', 'qdrant'))
Invoke-Compose (@('up', '--no-build', '--pull', 'never', '-d') +
    @('pgbouncer', 'temporal-schema', 'temporal-server', 'temporal-namespace'))
Apply-Fresh-Business-Migrations
Invoke-Compose (@('up', '--no-build', '--pull', 'never', '-d') +
    @('outbox-worker', 'temporal-worker'))

# Wait a bounded interval for real Docker health/readback after Compose exits.
$verified = $false
for ($attempt = 1; $attempt -le 24; $attempt++) {
    try {
        Verify-Running-Stack
        $verified = $true
        break
    } catch {
        if ($attempt -eq 24) { throw }
        Start-Sleep -Seconds 5
    }
}
if (-not $verified) { throw 'Runtime health was not verified' }
Assert-Previous-Volumes $before
Write-Output "INSTALL=PASS PROJECT=$Project SOURCE_SHA=$Commit WORKERS=GITHUB_ONLY OLD_VOLUMES_PRESERVED=$($before.Count)"
