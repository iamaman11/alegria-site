#requires -Version 7.0
<#
GitHub-only, fail-closed lifecycle for the two independent native Temporal V2
siteArchitect pilot owners. Status/Install/Update/Remove; no Windows build,
provider access, SQL credential, grant issuance, or Workflow Start.
Every owner release gets an immutable, root-owned catalog volume. Remove
preserves volumes, Temporal namespace, History and database backups.
#>
[CmdletBinding()]
param(
    [ValidateSet('Status', 'Install', 'Update', 'Remove')][string]$Action = 'Status',
    [ValidatePattern('^[a-f0-9]{40}$')][string]$Commit,
    [string]$HomeOnB = 'B:\Alegria',
    [switch]$AdoptExistingOwners
)

$ErrorActionPreference = 'Stop'
$docker = Join-Path $HomeOnB 'docker\app\resources\bin\docker.exe'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$compose = Join-Path $root 'infra\site_architect\compose.pure-owners-pilot.yml'
$project = 'alegria-site-architect-owners'
$network = 'alegria-site-clean_temporal_pure_control'
$namespace = 'site-architect-pilot'
$helper = 'temporalio/admin-tools:1.32.2'
$acquisition = 'alegria_site_architect_acquisition_pilot_v1'
$orchestrator = 'alegria_site_architect_orchestrator_pilot_v1'
$composeArgs = @('compose', '-p', $project, '--project-directory', $root, '-f', $compose)

function Assert-Exit([string]$Step) {
    if ($LASTEXITCODE -ne 0) { throw "$Step failed (exit $LASTEXITCODE)" }
}
function Invoke-Compose([string[]]$Arguments) {
    # Routine Docker Compose progress is stderr, not a failed PowerShell job.
    & $docker @script:composeArgs @Arguments 2>$null
    Assert-Exit "compose $($Arguments[0])"
}
function Container([string]$Name) {
    $json = & $docker inspect $Name --format '{{json .}}' 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return (($json | Out-String) | ConvertFrom-Json)
}
function Volumes {
    $all = @(& $docker volume ls --format '{{.Name}}')
    Assert-Exit 'list volumes'
    return $all
}
function Assert-Preserved([string[]]$Before) {
    $current = @(Volumes)
    foreach ($volume in $Before) {
        if ($volume -notin $current) { throw "Existing volume disappeared: $volume" }
    }
}
function Assert-Confined([object]$Data, [string]$Name) {
    if ($Data.HostConfig.NetworkMode -cne $script:network -or
        $Data.HostConfig.Privileged -or -not $Data.HostConfig.ReadonlyRootfs -or
        @($Data.HostConfig.CapDrop) -notcontains 'ALL' -or
        @($Data.HostConfig.PortBindings.PSObject.Properties).Count -gt 0) {
        throw "Unexpected native owner confinement: $Name"
    }
}
function Assert-Pilot-Namespace {
    $raw = @(& $docker run --rm --network $script:network --entrypoint temporal `
        $script:helper operator namespace describe --address temporal-server:7233 `
        --namespace $script:namespace --output json 2>$null)
    Assert-Exit 'inspect dedicated Temporal namespace'
    $response = (($raw -join [Environment]::NewLine) | ConvertFrom-Json)
    if ($response.namespaceInfo.name -cne $script:namespace -or
        $response.namespaceInfo.state -cne 'NAMESPACE_STATE_REGISTERED' -or
        $response.config.workflowExecutionRetentionTtl -cne '86400s') {
        throw 'Dedicated pilot Temporal namespace is missing or changed'
    }
}
function Assert-No-Open-Workflows {
    $raw = @(& $docker run --rm --network $script:network --entrypoint temporal `
        $script:helper workflow list --address temporal-server:7233 `
        --namespace $script:namespace --query 'ExecutionStatus="Running"' `
        --output json --limit 100 2>$null)
    Assert-Exit 'inspect live pilot Temporal workflows'
    $found = @(($raw -join [Environment]::NewLine) | ConvertFrom-Json)
    if ($found.Count) { throw 'Pilot Temporal has running Workflows; refuse cutover' }
}
function Assert-Canonical-Source([string]$SourceSha) {
    foreach ($tool in @('git','gh')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "$tool must be installed for GitHub-only promotion"
        }
    }
    $head = (& git -C $root rev-parse HEAD | Out-String).Trim()
    Assert-Exit 'local installer revision'
    $dirty = & git -C $root status --porcelain -- `
        'infra/local/windows_site_architect_pure_owners.ps1' `
        'infra/site_architect/compose.pure-owners-pilot.yml'
    Assert-Exit 'installer source status'
    if (($dirty | Out-String).Trim()) { throw 'Owner installer has uncommitted edits' }
    $remote = (& gh api 'repos/iamaman11/alegria-site/commits/main' --jq .sha | Out-String).Trim()
    Assert-Exit 'canonical main revision'
    if ($head -cne $remote) { throw 'Install/Update require the current exact GitHub main' }
    & git -C $root merge-base --is-ancestor $SourceSha $head
    Assert-Exit 'native owner release must be in accepted main history'
}
function Assert-Owner-Images([string]$SourceSha) {
    foreach ($owner in @('acquisition','orchestrator')) {
        $image = 'alegria-site-architect-' + $owner + ':' + $SourceSha
        $label = (& $docker image inspect $image `
            --format '{{index .Config.Labels "by.alegria.source-head-sha"}}' | Out-String).Trim()
        Assert-Exit "inspect imported $owner image"
        if ($label -cne $SourceSha) { throw "Wrong attested source for $owner" }
        $entry = (& $docker image inspect $image --format '{{json .Config.Entrypoint}}' | Out-String).Trim()
        Assert-Exit "inspect $owner entrypoint"
        $expected = if ($owner -eq 'acquisition') { '["/usr/local/bin/temporal_acquisition_pure"]' }
            else { '["/usr/local/bin/temporal_orchestrator"]' }
        if ($entry -cne $expected) { throw "Unexpected owner binary $owner" }
    }
    & $docker image inspect $script:helper --format '{{.Id}}' | Out-Null
    Assert-Exit 'pinned Temporal administrative helper must already be installed'
}
function Assert-Trusted-Catalog([string]$SourceSha) {
    $script:catalogVolume = 'alegria_site_architect_catalog_pilot_' + $SourceSha
    $image = 'alegria-site-architect-acquisition:' + $SourceSha
    $export = @(& $docker run --rm --network none --read-only --cap-drop ALL `
        --security-opt no-new-privileges --env ('WORKER_BUILD_ID=' + $SourceSha) `
        $image --export-trusted-catalog-v3)
    Assert-Exit 'export compiled native Acquisition catalog'
    $json = $export -join [Environment]::NewLine
    $document = $json | ConvertFrom-Json
    $expectedIds = @('acquisition.classify_source_page', 'acquisition.evaluate_dom_blocks',
        'acquisition.prepare_entity_span_input')
    $ids = @($document.entries | ForEach-Object { $_.contract.reference.id } | Sort-Object)
    if ($document.schema_revision -ne 3 -or $ids.Count -ne 3 -or
        ($ids -join ',') -cne (($expectedIds | Sort-Object) -join ',')) {
        throw 'Owner catalog must contain exactly the three approved Acquisition contracts'
    }
    foreach ($entry in $document.entries) {
        if ($entry.route.build_id -cne $SourceSha -or
            $entry.route.task_queue -cne 'alegria-acquisition-pure-owner-v1' -or
            $entry.contract.reference.revision -ne 1) {
            throw 'Owner route is not pinned to the exact GitHub binary'
        }
    }
    $expectedHash = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            [Text.Encoding]::UTF8.GetBytes($json.Trim()))).ToLowerInvariant()
    $existing = @(Volumes)
    if ($script:catalogVolume -notin $existing) {
        & $docker volume create --name $script:catalogVolume | Out-Null
        Assert-Exit 'create versioned immutable catalog volume'
        $export | & $docker run --rm -i --user '0:0' --network none `
            --mount "type=volume,src=$script:catalogVolume,dst=/etc/alegria" `
            --entrypoint /bin/sh $script:helper -c `
            'set -eu; umask 077; test ! -e /etc/alegria/acquisition-catalog.json; cat > /etc/alegria/acquisition-catalog.json; chown 0:0 /etc/alegria /etc/alegria/acquisition-catalog.json; chmod 755 /etc/alegria; chmod 444 /etc/alegria/acquisition-catalog.json'
        Assert-Exit 'install root-owned immutable catalog'
    }
    $onDisk = (& $docker run --rm --user '0:0' --network none `
        --mount "type=volume,src=$script:catalogVolume,dst=/etc/alegria,readonly" `
        --entrypoint /bin/sh $script:helper -c `
        'set -eu; test "$(stat -c "%a|%u:%g" /etc/alegria)" = "755|0:0"; test "$(stat -c "%a|%u:%g" /etc/alegria/acquisition-catalog.json)" = "444|0:0"; tr -d "\r\n" < /etc/alegria/acquisition-catalog.json | sha256sum | cut -d " " -f1' | Out-String).Trim()
    Assert-Exit 'verify immutable catalog ownership and digest'
    if ($onDisk -cne $expectedHash) { throw 'Installed trusted catalog differs from GitHub native export' }
    Write-Output "CATALOG=PASS SOURCE_SHA=$SourceSha ENTRIES=3"
}

if (-not (Test-Path -LiteralPath $docker -PathType Leaf)) { throw 'Docker CLI absent from B:' }
& $docker info --format '{{.ServerVersion}}' | Out-Null
Assert-Exit 'Docker Engine readiness'

if ($Action -eq 'Status') {
    foreach ($name in @($acquisition, $orchestrator)) {
        $data = Container $name
        if (-not $data) { Write-Output "CONTAINER=$name STATUS=absent"; continue }
        Assert-Confined $data $name
        $managed = $data.Config.Labels.'com.docker.compose.project' -ceq $project
        Write-Output "CONTAINER=$name STATUS=$($data.State.Status) MANAGED=$managed IMAGE=$($data.Config.Image)"
    }
    Write-Output 'SQL_GRANTS=NOT_PROVISIONED_BY_PURE_OWNERS'
    exit 0
}
if ($AdoptExistingOwners -and $Action -ne 'Install') {
    throw '-AdoptExistingOwners applies only to Install'
}
$before = @(Volumes)
if ($Action -eq 'Remove') {
    $installed = $null
    foreach ($name in @($acquisition, $orchestrator)) {
        $data = Container $name
        if ($data -and $data.Config.Labels.'com.docker.compose.project' -cne $project) {
            throw "Refuse to remove an unmanaged native owner: $name"
        }
        if ($data) { $installed = $data }
    }
    if (-not $installed) {
        Write-Output "REMOVE=ALREADY_ABSENT PROJECT=$project VOLUMES_PRESERVED=$($before.Count)"
        exit 0
    }
    if ($installed.Config.Image -cnotmatch '^alegria-site-architect-(?:acquisition|orchestrator):([a-f0-9]{40})$') {
        throw 'Installed managed owner image is not an exact GitHub revision'
    }
    $env:SITE_ARCHITECT_OWNER_IMAGE_SHA = $Matches[1]
    $env:SITE_ARCHITECT_OWNER_CATALOG_VOLUME = 'alegria_site_architect_catalog_pilot_' + $Matches[1]
    Assert-Pilot-Namespace
    Assert-No-Open-Workflows
    Invoke-Compose @('down', '--remove-orphans')
    Assert-Preserved $before
    Write-Output "REMOVE=PASS PROJECT=$project VOLUMES_PRESERVED=$($before.Count)"
    exit 0
}

if (-not $Commit) { throw "$Action requires an attested 40-character image source SHA" }
Assert-Canonical-Source $Commit
Assert-Owner-Images $Commit
$env:SITE_ARCHITECT_OWNER_IMAGE_SHA = $Commit
$env:SITE_ARCHITECT_OWNER_CATALOG_VOLUME = 'alegria_site_architect_catalog_pilot_' + $Commit

Assert-Pilot-Namespace
$net = ((& $docker network inspect $network --format '{{.Internal}}' | Out-String).Trim())
Assert-Exit 'inspect isolated Temporal network'
if ($net -cne 'true') { throw 'Owner network must be internal and not exposed to providers' }

$unmanaged = @()
foreach ($name in @($acquisition, $orchestrator)) {
    $data = Container $name
    if ($Action -eq 'Update' -and
        (-not $data -or $data.Config.Labels.'com.docker.compose.project' -cne $project)) {
        throw "Update requires a fully Compose-managed native owner: $name"
    }
    if ($data -and $data.Config.Labels.'com.docker.compose.project' -cne $project) {
        Assert-Confined $data $name
        $unmanaged += $name
    }
}
if ($unmanaged.Count -and -not $AdoptExistingOwners) {
    throw 'Existing unmanaged native owners require explicit -AdoptExistingOwners'
}
if ($Action -eq 'Update' -or $unmanaged.Count) { Assert-No-Open-Workflows }
Assert-Trusted-Catalog $Commit
Invoke-Compose @('config', '--quiet')
foreach ($name in @($orchestrator, $acquisition)) {
    if ($name -in $unmanaged) {
        & $docker rm -f $name | Out-Null
        Assert-Exit "remove exact previously unlabelled pilot owner $name"
    }
}
$up = @('up','-d','--no-build','--pull','never','--remove-orphans')
if ($Action -eq 'Update') { $up += '--force-recreate' }
Invoke-Compose $up
foreach ($name in @($acquisition, $orchestrator)) {
    $data = Container $name
    if (-not $data -or $data.State.Status -cne 'running' -or
        $data.Config.Labels.'com.docker.compose.project' -cne $project) {
        throw "Native owner failed to start as managed Compose service: $name"
    }
    Assert-Confined $data $name
    Write-Output "SERVICE=PASS NAME=$name"
}
Assert-Preserved $before
Write-Output "$Action=PASS PROJECT=$project SOURCE_SHA=$Commit SQL_GRANTS=NONE"
