# Pure, read-only deployment identity and decision contract.
# Windows PowerShell 7; sourced only by windows_staging.ps1 and its offline tests.
# No Docker invocation, WSL, state mutation, or external service.
function Test-Staging-Readback {
    [CmdletBinding()]
    param(
        [AllowNull()][object]$Observed,
        [string]$ExpectedImage,
        [string]$ExpectedProject,
        [string]$ExpectedCheckout,
        [string]$ExpectedBuildId,
        [bool]$MetricsRequired,
        [bool]$MetricsOk
    )
    if ($null -eq $Observed) { return $false }
    if ($Observed.State -ne 'running') { return $false }
    if (-not $ExpectedImage -or $Observed.Image -ne $ExpectedImage) { return $false }
    if (-not $ExpectedProject -or $Observed.Project -ne $ExpectedProject) { return $false }
    if (-not $ExpectedCheckout -or
        -not [string]::Equals([string]$Observed.Checkout, $ExpectedCheckout,
            [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }
    if ($ExpectedBuildId -and $Observed.BuildId -ne $ExpectedBuildId) {
        return $false
    }
    if ($MetricsRequired -and -not $MetricsOk) { return $false }
    return $true
}

function Resolve-Staging-Decision {
    [CmdletBinding()]
    param(
        [bool]$EngineReady,
        [bool]$NewVerified,
        [bool]$OldVerified,
        [int]$StartExit
    )
    if (-not $EngineReady) { return 'ENGINE_UNAVAILABLE' }
    if ($NewVerified) {
        if ($StartExit -eq 0) { return 'DEPLOYED' }
        return 'DEPLOYED_AFTER_START_ERROR'
    }
    if ($OldVerified) { return 'OLD_RUNNING' }
    return 'UNVERIFIED'
}
