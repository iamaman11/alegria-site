#requires -Version 7.0
<#
Repeatable, transport-only Windows B siteArchitect pilot lifecycle:
  Status  - read exact two containers, with no changes
  Install - stage three NEW private volumes and bring up GitHub-built ingress/tunnel
  Update  - recreate only these two containers with a verified GitHub image
  Remove  - remove only these two containers; preserve every volume and backup

No Windows builds, SQL grants, Temporal Starts, Access policy changes or secret
values in process arguments. A nonempty intent directory blocks replacement.
Existing non-Compose v2 containers require an explicit one-time adoption switch.
#>
[CmdletBinding()]
param(
    [ValidateSet('Status', 'Install', 'Update', 'Remove')][string]$Action = 'Status',
    [ValidatePattern('^[a-f0-9]{40}$')][string]$Commit,
    [string]$HomeOnB = 'B:\Alegria',
    [ValidatePattern('^[a-zA-Z0-9][a-zA-Z0-9_.-]+$')][string]$SourceKeyVolume,
    [ValidatePattern('^[a-zA-Z0-9][a-zA-Z0-9_.-]+$')][string]$SourceConnectorVolume,
    [switch]$AdoptExistingTransport
)

$ErrorActionPreference = 'Stop'
$docker = Join-Path $HomeOnB 'docker\app\resources\bin\docker.exe'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$compose = Join-Path $root 'infra\site_architect\compose.transport-pilot.yml'
$project = 'alegria-site-architect-transport'
$ingress = 'alegria_site_architect_ingress_pilot_v2'
$connector = 'alegria_site_architect_cloudflared_pilot_v2'
$signedVolume = 'alegria_site_architect_signed_private_pilot_v2'
$connectorVolume = 'alegria_site_architect_connector_private_pilot_v2'
$intentVolume = 'alegria_site_architect_intents_pilot_v2'
$network = 'alegria_site_architect_private_pilot_v2'
$helper = 'temporalio/admin-tools:1.32.2'
$cloudflared = 'cloudflare/cloudflared:2026.11.2'
$composeArgs = @('compose', '-p', $project, '--project-directory', $root, '-f', $compose)

function Assert-Exit([string]$Stage) {
    if ($LASTEXITCODE -ne 0) { throw "$Stage failed (exit $LASTEXITCODE)" }
}
function Invoke-Compose([string[]]$Arguments) {
    & $docker @script:composeArgs @Arguments
    Assert-Exit "compose $($Arguments[0])"
}
function Volumes {
    $names = @(& $docker volume ls --format '{{.Name}}')
    Assert-Exit 'list volumes'
    return $names
}
function Assert-Preserved([string[]]$Before) {
    $after = @(Volumes)
    foreach ($name in $Before) {
        if ($name -notin $after) { throw "An existing volume disappeared: $name" }
    }
}
function Container([string]$Name) {
    $raw = & $docker inspect $Name --format '{{json .}}' 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return (($raw | Out-String) | ConvertFrom-Json)
}
function Assert-Isolated([object]$Data, [string]$Name) {
    if ($Data.HostConfig.Privileged -or -not $Data.HostConfig.ReadonlyRootfs -or
        @($Data.HostConfig.CapDrop) -notcontains 'ALL' -or
        @($Data.HostConfig.PortBindings.PSObject.Properties).Count -gt 0) {
        throw "Unsafe container confinement: $Name"
    }
    if ($Name -ceq $script:ingress -and
        $Data.HostConfig.NetworkMode -cne $script:network) {
        throw "Ingress is not on the dedicated private network: $Name"
    }
    if ($Name -ceq $script:connector -and
        -not $Data.HostConfig.NetworkMode.StartsWith('container:')) {
        throw 'cloudflared must share the ingress network namespace'
    }
}
function Assert-Empty-Intents {
    $all = @(Volumes)
    if ($script:intentVolume -notin $all) { throw 'Private intent volume not installed' }
    $result = & $docker run --rm --user '65532:65532' --network none `
        --mount "type=volume,src=$script:intentVolume,dst=/intents,readonly" `
        --entrypoint /bin/sh $script:helper -c `
        'test "$(find /intents -mindepth 1 -maxdepth 1 | wc -l)" -eq 0 && echo EMPTY'
    Assert-Exit 'inspect private intents'
    if (($result | Out-String).Trim() -cne 'EMPTY') {
        throw 'Private intent directory contains grants; refuse automatic cutover'
    }
}
function Assert-Pilot-Images([string]$SourceSha) {
    foreach ($image in @(('alegria-site-architect-pg:' + $SourceSha), $script:cloudflared, $script:helper)) {
        & $docker image inspect $image --format '{{.Id}}' | Out-Null
        Assert-Exit "inspect preinstalled image $image (Windows must not build/pull)"
    }
    $image = 'alegria-site-architect-pg:' + $SourceSha
    $revision = (& $docker image inspect $image `
        --format '{{index .Config.Labels "by.alegria.source-head-sha"}}' | Out-String).Trim()
    Assert-Exit 'inspect native image provenance label'
    if ($revision -cne $SourceSha) { throw 'Native owner image source SHA mismatch' }
    $entry = (& $docker image inspect $image --format '{{json .Config.Entrypoint}}' | Out-String).Trim()
    Assert-Exit 'inspect native owner entrypoint'
    if ($entry -cne '["/usr/local/bin/temporal_pg_effects"]') {
        throw 'Unexpected native ingress owner entrypoint'
    }
}
function Assert-Canonical-Source([string]$SourceSha) {
    foreach ($cli in @('git', 'gh')) {
        if (-not (Get-Command $cli -ErrorAction SilentlyContinue)) { throw "$cli not installed" }
    }
    $head = (& git -C $root rev-parse HEAD | Out-String).Trim()
    Assert-Exit 'read installer source revision'
    $dirty = & git -C $root status --porcelain -- `
        'infra/local/windows_site_architect_transport.ps1' `
        'infra/site_architect/compose.transport-pilot.yml'
    Assert-Exit 'verify installer source integrity'
    if (($dirty | Out-String).Trim()) { throw 'Transport installer is locally modified' }
    $remote = (& gh api 'repos/iamaman11/alegria-site/commits/main' --jq .sha | Out-String).Trim()
    Assert-Exit 'read canonical GitHub main'
    if ($head -cne $remote) { throw 'Use the exact current canonical main checkout' }
    & git -C $root merge-base --is-ancestor $SourceSha $head
    Assert-Exit 'native owner image source must be an ancestor of main'
}
function Install-Private-Volumes {
    $old = @(Volumes)
    $needSigned = $script:signedVolume -notin $old
    $needConnector = $script:connectorVolume -notin $old
    $needIntent = $script:intentVolume -notin $old
    if (($needSigned -or $needConnector) -and
        (-not $SourceKeyVolume -or -not $SourceConnectorVolume)) {
        throw 'Fresh secret volumes require BOTH explicit source-volume names'
    }
    if ($needSigned -and $SourceKeyVolume -notin $old) { throw 'Key source volume does not exist' }
    if ($needConnector -and $SourceConnectorVolume -notin $old) { throw 'Token source volume does not exist' }
    foreach ($item in @(@($script:signedVolume, $needSigned),
            @($script:connectorVolume, $needConnector), @($script:intentVolume, $needIntent))) {
        if ($item[1]) {
            & $docker volume create --name $item[0] | Out-Null
            Assert-Exit "create new private volume $($item[0])"
        }
    }
    if ($needSigned -or $needConnector -or $needIntent) {
        # A one-shot root helper copies only operator-provided secret files into
        # separate new Linux volumes. The bytes never enter PowerShell or logs.
        $mounts = @('--mount', "type=volume,src=$script:signedVolume,dst=/new/signed",
            '--mount', "type=volume,src=$script:connectorVolume,dst=/new/connector",
            '--mount', "type=volume,src=$script:intentVolume,dst=/new/intents")
        $copy = 'set -eu; '
        if ($needSigned) {
            $mounts += @('--mount', "type=volume,src=$SourceKeyVolume,dst=/old/signed,readonly")
            $copy += 'test -s /old/signed/admission.key; cp -p /old/signed/admission.key /new/signed/admission.key; '
        }
        if ($needConnector) {
            $mounts += @('--mount', "type=volume,src=$SourceConnectorVolume,dst=/old/connector,readonly")
            $copy += 'test -s /old/connector/token; cp -p /old/connector/token /new/connector/token; '
        }
        $copy += 'chown 65532:65532 /new/signed /new/connector /new/intents /new/signed/admission.key /new/connector/token; '
        $copy += 'chmod 700 /new/signed /new/connector /new/intents; chmod 600 /new/signed/admission.key /new/connector/token; echo VOLUMES_READY'
        & $docker run --rm --user '0:0' --network none @mounts `
            --entrypoint /bin/sh $script:helper -c $copy
        Assert-Exit 'stage private secret volumes'
    }
    # Read-only preflight; no file content leaves the container.
    & $docker run --rm --user '65532:65532' --network none `
        --mount "type=volume,src=$script:signedVolume,dst=/signed,readonly" `
        --mount "type=volume,src=$script:connectorVolume,dst=/connector,readonly" `
        --entrypoint /bin/sh $script:helper -c `
        'test -s /signed/admission.key && test -s /connector/token && test "$(stat -c %a /signed/admission.key)" = 600 && test "$(stat -c %a /connector/token)" = 600'
    Assert-Exit 'verify private-file access and mode'
    Assert-Empty-Intents
}

if (-not (Test-Path -LiteralPath $docker -PathType Leaf)) { throw 'Docker CLI missing from B:' }
& $docker info --format '{{.ServerVersion}}' | Out-Null
Assert-Exit 'Docker Linux Engine not ready; start Docker Desktop in interactive session'

if ($Action -eq 'Status') {
    foreach ($name in @($ingress, $connector)) {
        $data = Container $name
        if (-not $data) { Write-Output "CONTAINER=$name STATUS=absent"; continue }
        Assert-Isolated $data $name
        $managed = $data.Config.Labels.'com.docker.compose.project' -ceq $project
        Write-Output "CONTAINER=$name STATUS=$($data.State.Status) MANAGED=$managed IMAGE=$($data.Config.Image)"
    }
    Write-Output "VOLUMES_PRESENT=$(@((Volumes) | Where-Object { $_ -in @($signedVolume,$connectorVolume,$intentVolume) }).Count)/3"
    Write-Output 'SQL_GRANTS=NOT_PROVISIONED_BY_TRANSPORT'
    exit 0
}
if ($AdoptExistingTransport -and $Action -ne 'Install') {
    throw '-AdoptExistingTransport is only permitted for Install'
}
$before = @(Volumes)
if ($Action -eq 'Remove') {
    foreach ($name in @($ingress, $connector)) {
        $data = Container $name
        if ($data -and $data.Config.Labels.'com.docker.compose.project' -cne $project) {
            throw "Refuse to remove unmanaged container: $name"
        }
    }
    Invoke-Compose @('down', '--remove-orphans')
    Assert-Preserved $before
    Write-Output "REMOVE=PASS PROJECT=$project VOLUMES_PRESERVED=$($before.Count)"
    exit 0
}

if (-not $Commit) { throw "$Action requires a 40-character GitHub owner image SHA" }
Assert-Canonical-Source $Commit
Assert-Pilot-Images $Commit
$env:SITE_ARCHITECT_PG_IMAGE_SHA = $Commit

if ($Action -eq 'Update') {
    foreach ($name in @($ingress, $connector)) {
        $data = Container $name
        if (-not $data -or $data.Config.Labels.'com.docker.compose.project' -cne $project) {
            throw "Update requires an installed Compose-managed transport: $name"
        }
    }
    Assert-Empty-Intents
} elseif ($Action -eq 'Install') {
    $unmanaged = @()
    foreach ($name in @($ingress, $connector)) {
        $data = Container $name
        if ($data -and $data.Config.Labels.'com.docker.compose.project' -cne $project) {
            Assert-Isolated $data $name
            $unmanaged += $name
        }
    }
    if ($unmanaged.Count -and -not $AdoptExistingTransport) {
        throw 'Unmanaged v2 pilot exists; explicit -AdoptExistingTransport required'
    }
    Install-Private-Volumes
    $nets = @(& $docker network ls --format '{{.Name}}')
    Assert-Exit 'list networks'
    if ($network -notin $nets) {
        & $docker network create --driver bridge $network | Out-Null
        Assert-Exit 'create private connector network'
    }
} else {
    throw 'Unknown transport action'
}

Invoke-Compose @('config', '--quiet')
if ($unmanaged.Count) {
    # Explicit one-time cutover after all prerequisites are checked. Never -v.
    foreach ($name in @($connector, $ingress)) {
        if ($name -in $unmanaged) {
            & $docker rm -f $name | Out-Null
            Assert-Exit "retire exact unmanaged transport container $name"
        }
    }
}
Invoke-Compose @('up', '-d', '--no-build', '--pull', 'never', '--force-recreate', '--remove-orphans')
foreach ($name in @($ingress, $connector)) {
    $data = Container $name
    if (-not $data -or $data.State.Status -cne 'running' -or
        $data.Config.Labels.'com.docker.compose.project' -cne $project) {
        throw "Transport container failed readiness: $name"
    }
    Assert-Isolated $data $name
    Write-Output "SERVICE=PASS NAME=$name"
}
Assert-Preserved $before
Write-Output "$Action=PASS SOURCE_SHA=$Commit PROJECT=$project SQL_GRANTS=NONE"
