#requires -Version 7.0
# Native Windows, no Docker calls. Regression for the decisions that follow
# a broken Docker Compose start and subsequent Docker Desktop engine recovery.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\infra\local\windows_staging_reconcile.ps1')

function Assert-Equal([object]$Got, [object]$Expected, [string]$Scenario) {
    if ($Got -cne $Expected) {
        throw "$Scenario expected $Expected, observed $Got"
    }
}
$project = 'alegria-site-test'
$new = 'sha256:new'
$old = 'sha256:old'
$sha = 'a' * 40
$newRoot = 'B:\Alegria\runtime\alegria-site-' + $sha
$oldRoot = 'B:\Alegria\runtime\alegria-site-' + ('b' * 40)
$readback = @{
    State = 'running'
    Image = $new
    Project = $project
    Checkout = $newRoot
    BuildId = $sha
}
$check = @{
    ExpectedImage = $new
    ExpectedProject = $project
    ExpectedCheckout = $newRoot
    ExpectedBuildId = $sha
    MetricsRequired = $true
    MetricsOk = $true
}

Assert-Equal (Test-Staging-Readback -Observed $readback @check) $true 'verified candidate'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $true -OldVerified $false -StartExit 0) 'DEPLOYED' 'normal start'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $true -OldVerified $false -StartExit -1) 'DEPLOYED_AFTER_START_ERROR' 'Docker -1 but exact candidate observed'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $true -OldVerified $false -StartExit 125) 'DEPLOYED_AFTER_START_ERROR' 'Docker nonzero but exact candidate observed'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $false -OldVerified $true -StartExit -1) 'OLD_RUNNING' 'old image survives start failure'
Assert-Equal (Resolve-Staging-Decision -EngineReady $false -NewVerified $true -OldVerified $false -StartExit -1) 'ENGINE_UNAVAILABLE' 'engine unavailable cannot claim success'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $false -OldVerified $false -StartExit 0) 'UNVERIFIED' 'Docker zero but wrong image'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $false -OldVerified $false -StartExit -1) 'UNVERIFIED' 'Docker failure and no proven image'

foreach ($scenario in @(
    @{ Name='not running'; Observed=@{State='created'}; Change=@{} },
    @{ Name='wrong image'; Observed=@{Image=$old}; Change=@{} },
    @{ Name='wrong project'; Observed=@{Project='other'}; Change=@{} },
    @{ Name='wrong checkout'; Observed=@{Checkout=$oldRoot}; Change=@{} },
    @{ Name='wrong build id'; Observed=@{BuildId=('c'*40)}; Change=@{} }
)) {
    $test = $readback.Clone()
    foreach ($key in $scenario.Observed.Keys) { $test[$key] = $scenario.Observed[$key] }
    Assert-Equal (Test-Staging-Readback -Observed $test @check) $false $scenario.Name
}
$noMetrics = $check.Clone()
$noMetrics.MetricsOk = $false
Assert-Equal (Test-Staging-Readback -Observed $readback @noMetrics) $false 'not ready metrics'
Assert-Equal (Test-Staging-Readback -Observed $null @check) $false 'no engine readback'

# Same source/version must be recreated when credentials change. No real secrets.
$wanted = [pscustomobject]@{
    DATAFORSEO_LOGIN = 'example-login'
    CLOUDFLARE_API_TOKEN = 'new-example-token'
}
$withEnv = $readback.Clone()
$withEnv.Environment = @('DATAFORSEO_LOGIN=example-login', 'CLOUDFLARE_API_TOKEN=new-example-token')
$withEnvCheck = $check.Clone()
$withEnvCheck.ExpectedEnvironment = $wanted
Assert-Equal (Test-Staging-Readback -Observed $withEnv @withEnvCheck) $true 'desired config readback'
Assert-Equal (Test-Staging-EnvironmentMatches -Expected $wanted -Actual $withEnv.Environment) $true 'same SHA same environment noop'
$withEnv.Environment = @('DATAFORSEO_LOGIN=example-login', 'CLOUDFLARE_API_TOKEN=old-example-token')
Assert-Equal (Test-Staging-Readback -Observed $withEnv @withEnvCheck) $false 'stale token rejected'
Assert-Equal (Test-Staging-EnvironmentMatches -Expected $wanted -Actual $withEnv.Environment) $false 'same SHA different environment requires recreate'
$wanted.CLOUDFLARE_API_TOKEN = ''
$withEnv.Environment = @('DATAFORSEO_LOGIN=example-login', 'CLOUDFLARE_API_TOKEN=')
Assert-Equal (Test-Staging-EnvironmentMatches -Expected $wanted -Actual $withEnv.Environment) $true 'blank previous secret preserved'
$withEnv.Environment = @('DATAFORSEO_LOGIN=example-login')
Assert-Equal (Test-Staging-EnvironmentMatches -Expected $wanted -Actual $withEnv.Environment) $false 'missing required env key'
$oldCheck = $check.Clone()
$oldCheck.ExpectedImage = $old
$oldCheck.ExpectedCheckout = $oldRoot
$oldCheck.ExpectedBuildId = 'b' * 40
Assert-Equal (Test-Staging-Readback -Observed $readback @oldCheck) $false 'candidate is not rollback'

# A source/ops-only commit may produce an identical immutable Docker image.
# Different checkout and build identity still keep the old and new states apart.
$sameImageOld = $readback.Clone()
$sameImageOld.Checkout = $oldRoot
$sameImageOld.BuildId = 'b' * 40
$sameImageCheck = $check.Clone()
$sameImageCheck.ExpectedCheckout = $oldRoot
$sameImageCheck.ExpectedBuildId = 'b' * 40
Assert-Equal (Test-Staging-Readback -Observed $sameImageOld @sameImageCheck) $true 'old image with same ID still verified by source'
Assert-Equal (Test-Staging-Readback -Observed $sameImageOld @check) $false 'same binary does not impersonate new source'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $false -OldVerified $true -StartExit -1) 'OLD_RUNNING' 'same image ID, old checkout survives'

# Missing original image metadata: verified rebuild may have another immutable ID.
$unavailableOriginal = 'sha256:missing-original'
$verifiedRebuild = 'sha256:verified-rebuild'
$recoveredRunning = $readback.Clone()
$recoveredRunning.Image = $verifiedRebuild
$recoveredRunning.Checkout = $oldRoot
$recoveredRunning.BuildId = 'b' * 40
$verifiedRollbackCheck = $check.Clone()
$verifiedRollbackCheck.ExpectedImage = $verifiedRebuild
$verifiedRollbackCheck.ExpectedCheckout = $oldRoot
$verifiedRollbackCheck.ExpectedBuildId = 'b' * 40
Assert-Equal (Test-Staging-Readback -Observed $recoveredRunning @verifiedRollbackCheck) $true 'rebuilt rollback matches verified tag'
$staleImageCheck = $verifiedRollbackCheck.Clone()
$staleImageCheck.ExpectedImage = $unavailableOriginal
Assert-Equal (Test-Staging-Readback -Observed $recoveredRunning @staleImageCheck) $false 'old missing ID cannot identify rebuilt rollback'
Assert-Equal (Test-Staging-Readback -Observed $recoveredRunning @check) $false 'rebuilt previous is not candidate'
Assert-Equal (Resolve-Staging-Decision -EngineReady $true -NewVerified $false -OldVerified $true -StartExit -1) 'OLD_RUNNING' 'rebuilt old runs after rollback Compose error'
$wrongRollbackProject = $recoveredRunning.Clone()
$wrongRollbackProject.Project = 'foreign-project'
Assert-Equal (Test-Staging-Readback -Observed $wrongRollbackProject @verifiedRollbackCheck) $false 'rebuilt previous foreign project'
$unhealthyRollback = $verifiedRollbackCheck.Clone()
$unhealthyRollback.MetricsOk = $false
Assert-Equal (Test-Staging-Readback -Observed $recoveredRunning @unhealthyRollback) $false 'rebuilt previous missing metrics'

# Parse the actual deployment entrypoint on Windows without running Docker.
$tokens = $null
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot '..\infra\local\windows_staging.ps1'),
    [ref]$tokens, [ref]$parseErrors
) | Out-Null
if (@($parseErrors).Count -gt 0) {
    throw 'Windows staging script contains PowerShell syntax errors'
}

# Removing an inherited variable must remove it, not leave an empty value:
# Docker Compose otherwise treats empty as higher priority than --env-file.
$env:ALEGRIA_STAGING_ENV_TEST = 'inherited-placeholder'
try {
    Remove-Item -LiteralPath 'Env:ALEGRIA_STAGING_ENV_TEST' -ErrorAction Stop
    Assert-Equal (Test-Path -LiteralPath 'Env:ALEGRIA_STAGING_ENV_TEST') $false 'provider variable removed'
} finally {
    Remove-Item -LiteralPath 'Env:ALEGRIA_STAGING_ENV_TEST' -ErrorAction SilentlyContinue
}

Write-Output 'WINDOWS_STAGING_ENGINE_RECONCILE=PASS'
