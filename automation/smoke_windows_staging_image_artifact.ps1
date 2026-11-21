#requires -Version 7.0
# Runs on a disposable native Windows runner, never requires Docker or GitHub auth.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\infra\local\windows_staging_image.ps1')

function Assert-Reject([string]$Scenario, [scriptblock]$Check) {
    $rejected = $false
    try { & $Check | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "CI artifact verification accepted $Scenario" }
}

$sha = 'a' * 40
$otherSha = 'b' * 40
$imageId = 'sha256:' + ('c' * 64)
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ('alegria-ci-image-' + [guid]::NewGuid().ToString('n'))
New-Item -ItemType Directory -Path $temp -Force | Out-Null
try {
    $archive = Join-Path $temp 'temporal-worker.tar.gz'
    [System.IO.File]::WriteAllBytes($archive, [byte[]](1, 2, 3, 4, 5))
    $digest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $temp 'temporal-worker.tar.gz.sha256') -Value "$digest  temporal-worker.tar.gz"
    Set-Content -LiteralPath (Join-Path $temp 'image-id.txt') -Value $imageId
    Set-Content -LiteralPath (Join-Path $temp 'source-sha.txt') -Value $sha

    $accepted = Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha
    if ($accepted.SourceSha -cne $sha -or $accepted.ImageId -cne $imageId -or
        $accepted.Archive -cne $archive) { throw 'Known-good CI artifact metadata failed' }
    Assert-Reject 'different requested SHA' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $otherSha }

    Set-Content -LiteralPath (Join-Path $temp 'source-sha.txt') -Value $otherSha
    Assert-Reject 'wrong embedded source' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha }
    Set-Content -LiteralPath (Join-Path $temp 'source-sha.txt') -Value $sha

    Set-Content -LiteralPath (Join-Path $temp 'image-id.txt') -Value 'latest'
    Assert-Reject 'mutable Docker image reference' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha }
    Set-Content -LiteralPath (Join-Path $temp 'image-id.txt') -Value $imageId

    [System.IO.File]::WriteAllBytes($archive, [byte[]](1, 2, 3, 4, 6))
    Assert-Reject 'tampered image bytes' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha }
    Set-Content -LiteralPath (Join-Path $temp 'temporal-worker.tar.gz.sha256') -Value "${digest}  ../temporal-worker.tar.gz"
    Assert-Reject 'malformed checksum manifest' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha }
    Remove-Item -LiteralPath (Join-Path $temp 'temporal-worker.tar.gz.sha256') -Force
    Assert-Reject 'missing checksum manifest' { Read-Staging-Verified-Worker-Image -ArtifactDirectory $temp -ExpectedSha $sha }
} finally {
    Remove-Item -LiteralPath $temp -Force -Recurse
}

Write-Output 'WINDOWS_CI_WORKER_ARTIFACT=PASS'
