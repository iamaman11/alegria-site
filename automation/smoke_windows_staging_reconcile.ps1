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
$oldCheck = $check.Clone()
$oldCheck.ExpectedImage = $old
$oldCheck.ExpectedCheckout = $oldRoot
$oldCheck.ExpectedBuildId = 'b' * 40
Assert-Equal (Test-Staging-Readback -Observed $readback @oldCheck) $false 'candidate is not rollback'

Write-Output 'WINDOWS_STAGING_ENGINE_RECONCILE=PASS'
