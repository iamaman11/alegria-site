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
Write-Output 'WINDOWS_STACK_LIFECYCLE_CONTRACT=PASS'
