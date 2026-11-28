#requires -Version 7.0
# Download and load only an attested accepted-main GitHub Actions release.
# Never run docker build, cargo build, or modify existing containers here.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourceSha,
    [Parameter(Mandatory)][long]$RunId
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows_site_architect_image_verify.ps1')

$repo = 'iamaman11/alegria-site'
$downloadRoot = 'B:\Alegria\runtime\site-architect-releases'
if ($SourceSha -cnotmatch '^[a-f0-9]{40}$' -or $RunId -le 0) {
    throw 'An exact source SHA and positive Actions run ID are required'
}
foreach ($command in @('gh', 'docker', 'git')) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "Required native installer CLI is missing: $command"
    }
}

# Install scripts themselves must come from this exact accepted source, not
# from an arbitrary local checkout or an edited Windows copy.
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$checkoutSha = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $checkoutSha -cne $SourceSha) {
    throw 'Windows installer source checkout is not the accepted GitHub commit'
}
$installerFiles = @('infra/local/windows_site_architect_images.ps1',
    'infra/local/windows_site_architect_image_verify.ps1')
$modified = & git -C $repoRoot status --porcelain -- @installerFiles
if ($LASTEXITCODE -ne 0 -or ($modified | Out-String).Trim().Length -ne 0) {
    throw 'Windows installer source files are modified or untracked'
}

$rawRun = & gh run view $RunId -R $repo --json 'databaseId,headSha,headBranch,event,conclusion,jobs'
if ($LASTEXITCODE -ne 0) { throw 'Cannot verify canonical GitHub Actions run' }
$run = ($rawRun | Out-String) | ConvertFrom-Json
$packageJob = @($run.jobs | Where-Object {
    $_.name -eq 'package-site-architect' -and $_.conclusion -eq 'success'
})
if ($run.headSha -cne $SourceSha -or $run.headBranch -cne 'main' -or
    $run.event -cne 'workflow_dispatch' -or $run.conclusion -cne 'success' -or
    $packageJob.Count -ne 1) {
    throw 'siteArchitect image release is not a successful accepted-main packaging run'
}

if (-not (Test-Path -LiteralPath 'B:\Alegria\runtime' -PathType Container)) {
    throw 'Required Alegria B: runtime directory is unavailable'
}
$destination = Join-Path $downloadRoot ($SourceSha + '-' + $RunId + '-' + [guid]::NewGuid().ToString('n'))
$null = New-Item -ItemType Directory -Path $destination -Force
$artifactName = 'site-architect-images-' + $SourceSha
& gh run download $RunId -R $repo -n $artifactName -D $destination
if ($LASTEXITCODE -ne 0) { throw 'Cannot download exact GitHub siteArchitect image artifact' }

$receipt = Read-SiteArchitect-Verified-ImageArchive -ArtifactDirectory $destination -ExpectedSha $SourceSha
& gh attestation verify $receipt.Archive -R $repo | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'GitHub image archive provenance attestation failed' }

# Never replace an existing exact-source image tag with a different image ID.
& docker info --format '{{.ServerVersion}}' | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Docker Engine is not ready for image import' }
foreach ($image in $receipt.Images) {
    $known = $null
    try {
        $probe = & docker image inspect $image.tag --format '{{.Id}}' 2>$null
        if ($LASTEXITCODE -eq 0) { $known = ($probe | Out-String).Trim() }
    } catch {
        # A fresh release tag is expected to be absent before docker load.
    }
    if ($known -and $known -cne $image.image_id) {
        throw "Existing immutable tag conflicts with attested release: $($image.owner)"
    }
}

& docker image load --input $receipt.Archive | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Docker failed to import the attested release archive' }

$entrypoints = @{
    pg = 'temporal_pg_effects'
    orchestrator = 'temporal_orchestrator'
    acquisition = 'temporal_acquisition_pure'
    'request-issuer' = 'one_step_request_issuer'
    'source-issuer' = 'acquisition_source_issuer'
}
foreach ($image in $receipt.Images) {
    $actualId = (& docker image inspect $image.tag --format '{{.Id}}' | Out-String).Trim()
    $actualSha = (& docker image inspect $image.tag --format '{{index .Config.Labels "by.alegria.source-head-sha"}}' | Out-String).Trim()
    $entrypointJson = (& docker image inspect $image.tag --format '{{json .Config.Entrypoint}}' | Out-String).Trim()
    $entrypoint = @(ConvertFrom-Json -InputObject $entrypointJson)
    if ($LASTEXITCODE -ne 0 -or $actualId -cne $image.image_id -or $actualSha -cne $SourceSha -or
        @($entrypoint).Count -ne 1 -or $entrypoint[0] -cne ('/usr/local/bin/' + $entrypoints[$image.owner])) {
        throw "Imported siteArchitect image failed immutable ID/source verification: $($image.owner)"
    }
}
Write-Output "SITE_ARCHITECT_IMAGE_INSTALL=PASS SOURCE_SHA=$SourceSha RUN_ID=$RunId IMAGES=$($receipt.Images.Count) PATH=$destination"
