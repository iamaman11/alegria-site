#requires -Version 7.0
# Native Windows release lifecycle contract: offline, no Docker or secrets.
$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot '../infra/local/windows_stack.ps1'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $scriptPath, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0 -or -not $ast) {
    throw "Windows lifecycle script PowerShell parsing failed: $($errors -join '; ')"
}
$source = [System.IO.File]::ReadAllText($scriptPath)
$actions = @('Status', 'Install', 'Update', 'Remove')
foreach ($action in $actions) {
    if ($source -cnotmatch "'${action}'") { throw "Missing lifecycle action: $action" }
}
$required = @('--no-build', '--pull', 'never', 'Read-Staging-Verified-Worker-Image',
    'gh attestation verify', 'docker', 'ReplaceOldContainers', 'BackupDirectory',
    'Assert-Previous-Volumes', 'Verify-Running-Stack',
    'Apply-Fresh-Business-Migrations', 'ALEGRIA_SCHEMA_BASELINE_MIGRATION',
    'Ensure-Pinned-Base-Images')
foreach ($token in $required) {
    if (-not $source.Contains($token)) { throw "Lifecycle safety contract absent: $token" }
}
$withoutBlockComments = [regex]::Replace($source, '(?s)<#.*?#>', '')
$program = ($withoutBlockComments -split "`n" | Where-Object { $_ -notmatch '^\s*#' }) -join "`n"
if ($program -match '(?m)\b(?:docker|\$docker)\s+(?:build|compose\s+build)\b' -or
    $program -match '(?m)\bdown\s+-v\b' -or
    $program -match '(?m)\bvolume\s+(?:rm|prune)\b') {
    throw 'Windows lifecycle must never build locally or delete persistent volumes'
}
# Update delegates into windows_staging.ps1, which must obey the same rule.
# This guards against reintroducing the legacy Windows rebuild fallback.
$stagingPath = Join-Path $PSScriptRoot '../infra/local/windows_staging.ps1'
$stagingTokens = $null
$stagingErrors = $null
$stagingAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $stagingPath, [ref]$stagingTokens, [ref]$stagingErrors)
if ($stagingErrors.Count -ne 0 -or -not $stagingAst) {
    throw "Staging PowerShell parsing failed: $($stagingErrors -join '; ')"
}
$dockerCommands = @($stagingAst.FindAll({ param($node)
    $node -is [System.Management.Automation.Language.CommandAst] -and
    $node.Extent.Text -match '(?m)^\s*&\s*\$docker\b'
}, $true))
foreach ($command in $dockerCommands) {
    if ($command.Extent.Text -match '(?m)\sbuild(?:\s|$)') {
        throw 'Staging must never compile Docker images on Windows'
    }
}
if (-not ([System.IO.File]::ReadAllText($stagingPath)).Contains('ROLLBACK_IMAGE_MISSING')) {
    throw 'Staging must fail closed when the original GitHub image is missing'
}
Write-Output 'WINDOWS_STACK_LIFECYCLE_CONTRACT=PASS'
