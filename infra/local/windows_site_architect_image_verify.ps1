#requires -Version 7.0
# Data-only validation; safe to dot-source in CI without Docker or gh.
function Read-SiteArchitect-Verified-ImageArchive {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$ArtifactDirectory,
        [Parameter(Mandatory)][string]$ExpectedSha
    )

    if ($ExpectedSha -cnotmatch '^[a-f0-9]{40}$') {
        throw 'siteArchitect image installation requires a full lowercase source SHA'
    }
    $archive = Join-Path $ArtifactDirectory 'site-architect-images.tar.gz'
    $checksumPath = Join-Path $ArtifactDirectory 'site-architect-images.tar.gz.sha256'
    $manifestPath = Join-Path $ArtifactDirectory 'manifest.json'
    foreach ($file in @($archive, $checksumPath, $manifestPath)) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "siteArchitect release archive missing $([IO.Path]::GetFileName($file))"
        }
    }

    $line = [IO.File]::ReadAllText($checksumPath).Trim()
    if ($line -cnotmatch '^([a-f0-9]{64})  site-architect-images\.tar\.gz$') {
        throw 'siteArchitect release checksum format is invalid'
    }
    $expectedDigest = $Matches[1]
    $actualDigest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -cne $expectedDigest) {
        throw 'siteArchitect release archive SHA256 mismatch'
    }

    $manifest = [IO.File]::ReadAllText($manifestPath) | ConvertFrom-Json
    $owners = @('pg', 'orchestrator', 'acquisition', 'request-issuer', 'source-issuer')
    $revisionNumeric = $manifest.schema_revision -is [int] -or $manifest.schema_revision -is [long]
    if (-not $revisionNumeric -or $manifest.schema_revision -ne 1 -or
        $manifest.source_sha -cne $ExpectedSha -or $manifest.images -isnot [array] -or
        @($manifest.images).Count -ne $owners.Count -or
        (@($manifest.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'images,schema_revision,source_sha') {
        throw 'siteArchitect image manifest schema, source or owner count mismatch'
    }
    for ($i = 0; $i -lt $owners.Count; $i++) {
        $owner = $owners[$i]
        $image = $manifest.images[$i]
        $requiredTag = 'alegria-site-architect-' + $owner + ':' + $ExpectedSha
        if ((@($image.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'image_id,owner,tag' -or
            $image.owner -cne $owner -or $image.tag -cne $requiredTag -or
            $image.image_id -cnotmatch '^sha256:[a-f0-9]{64}$') {
            throw "siteArchitect release image identity mismatch for $owner"
        }
    }
    if (@($manifest.images | ForEach-Object { $_.image_id } | Sort-Object -Unique).Count -ne $owners.Count) {
        throw 'siteArchitect owners must have five distinct native image IDs'
    }
    return [pscustomobject]@{
        Archive = $archive
        SourceSha = $ExpectedSha
        Images = @($manifest.images)
    }
}
