#requires -Version 7.0
# Validate the exact-SHA, CI-produced Docker image before allowing a tag or
# container transition. This helper is deliberately testable without Docker.
function Read-Staging-Verified-Worker-Image {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$ArtifactDirectory,
        [Parameter(Mandatory)][string]$ExpectedSha
    )
    if ($ExpectedSha -cnotmatch '^[a-f0-9]{40}$') {
        throw 'CI Worker artifact requires a full lowercase source SHA'
    }
    $files = @('temporal-worker.tar.gz', 'temporal-worker.tar.gz.sha256',
        'image-id.txt', 'source-sha.txt')
    foreach ($file in $files) {
        if (-not (Test-Path -LiteralPath (Join-Path $ArtifactDirectory $file) -PathType Leaf)) {
            throw "CI Worker artifact missing $file"
        }
    }
    $sourceSha = [System.IO.File]::ReadAllText((Join-Path $ArtifactDirectory 'source-sha.txt')).Trim()
    if ($sourceSha -cne $ExpectedSha) { throw 'CI Worker artifact has a different source SHA' }
    $imageId = [System.IO.File]::ReadAllText((Join-Path $ArtifactDirectory 'image-id.txt')).Trim()
    if ($imageId -cnotmatch '^sha256:[a-f0-9]{64}$') {
        throw 'CI Worker artifact has no immutable Docker image ID'
    }
    $checksum = [System.IO.File]::ReadAllText((Join-Path $ArtifactDirectory 'temporal-worker.tar.gz.sha256')).Trim()
    if ($checksum -cnotmatch '^([a-f0-9]{64})  temporal-worker\.tar\.gz$') {
        throw 'CI Worker artifact checksum manifest is malformed'
    }
    $expectedDigest = $Matches[1]
    $archive = Join-Path $ArtifactDirectory 'temporal-worker.tar.gz'
    $actualDigest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -cne $expectedDigest) {
        throw 'CI Worker artifact SHA256 mismatch: refusing image load'
    }
    return [pscustomobject]@{ Archive = $archive; ImageId = $imageId; SourceSha = $sourceSha }
}
