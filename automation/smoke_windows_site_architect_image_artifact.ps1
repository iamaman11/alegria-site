#requires -Version 7.0
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../infra/local/windows_site_architect_image_verify.ps1')

$source = 'a' * 40
$root = Join-Path ([IO.Path]::GetTempPath()) ('site-architect-artifact-test-' + [guid]::NewGuid().ToString('n'))
$null = New-Item -ItemType Directory -Path $root
try {
    $archive = Join-Path $root 'site-architect-images.tar.gz'
    [IO.File]::WriteAllBytes($archive, [byte[]](1, 2, 3, 4, 5))
    $checksum = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $root 'site-architect-images.tar.gz.sha256'), "$checksum  site-architect-images.tar.gz`n")
    $owners = @('pg', 'orchestrator', 'acquisition', 'request-issuer', 'source-issuer')
    $images = @(for ($i = 0; $i -lt $owners.Count; $i++) {
        $owner = $owners[$i]
        [ordered]@{owner=$owner;tag=('alegria-site-architect-' + $owner + ':' + $source);image_id=('sha256:' + ([string]([char](98 + $i)) * 64))}
    })
    $manifest = [ordered]@{schema_revision=1;source_sha=$source;images=$images}
    $path = Join-Path $root 'manifest.json'
    [IO.File]::WriteAllText($path, ($manifest | ConvertTo-Json -Depth 7))

    $ok = Read-SiteArchitect-Verified-ImageArchive -ArtifactDirectory $root -ExpectedSha $source
    if ($ok.Images.Count -ne 5) { throw 'Positive fixture did not pass' }

    $manifest.images[2].tag = 'wrong:tag'
    [IO.File]::WriteAllText($path, ($manifest | ConvertTo-Json -Depth 7))
    try {
        $null = Read-SiteArchitect-Verified-ImageArchive -ArtifactDirectory $root -ExpectedSha $source
        throw 'Malformed tag was accepted'
    } catch {
        if ($_.Exception.Message -eq 'Malformed tag was accepted') { throw }
    }

    $manifest.images[2].tag = 'alegria-site-architect-acquisition:' + $source
    [IO.File]::WriteAllText($path, ($manifest | ConvertTo-Json -Depth 7))
    [IO.File]::WriteAllBytes($archive, [byte[]](1, 2, 3, 4, 6))
    try {
        $null = Read-SiteArchitect-Verified-ImageArchive -ArtifactDirectory $root -ExpectedSha $source
        throw 'Tampered archive was accepted'
    } catch {
        if ($_.Exception.Message -eq 'Tampered archive was accepted') { throw }
    }
    Write-Output 'SITE_ARCHITECT_IMAGE_ARCHIVE_GUARDS=PASS'
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force
}
