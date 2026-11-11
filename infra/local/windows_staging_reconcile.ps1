# Pure, read-only deployment identity and decision contract.
# Windows PowerShell 7; sourced only by windows_staging.ps1 and its offline tests.
# No Docker invocation, WSL, state mutation, or external service.
function ConvertTo-Staging-EnvironmentMap {
    [CmdletBinding()]
    param([AllowNull()][object]$Environment)
    $map = @{}
    foreach ($entry in @($Environment)) {
        if ($null -eq $entry) { continue }
        $line = [string]$entry
        $split = $line.IndexOf('=')
        if ($split -gt 0) {
            $map[$line.Substring(0, $split)] = $line.Substring($split + 1)
        }
    }
    return $map
}

function Test-Staging-EnvironmentMatches {
    [CmdletBinding()]
    param([AllowNull()][object]$Expected, [AllowNull()][object]$Actual)
    if ($null -eq $Expected -or $null -eq $Actual) { return $false }
    $actualMap = ConvertTo-Staging-EnvironmentMap -Environment $Actual
    $expectedMap = @{}
    if ($Expected -is [System.Collections.IDictionary]) {
        foreach ($key in $Expected.Keys) { $expectedMap[[string]$key] = [string]$Expected[$key] }
    } else {
        foreach ($property in $Expected.PSObject.Properties) {
            $expectedMap[[string]$property.Name] = [string]$property.Value
        }
    }
    foreach ($key in $expectedMap.Keys) {
        if (-not $actualMap.ContainsKey($key) -or -not
            [string]::Equals([string]$actualMap[$key], [string]$expectedMap[$key],
                [System.StringComparison]::Ordinal)) {
            return $false
        }
    }
    return $true
}

function Test-Staging-Readback {
    [CmdletBinding()]
    param(
        [AllowNull()][object]$Observed,
        [string]$ExpectedImage,
        [string]$ExpectedProject,
        [string]$ExpectedCheckout,
        [string]$ExpectedBuildId,
        [bool]$MetricsRequired,
        [bool]$MetricsOk,
        [AllowNull()][object]$ExpectedEnvironment
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
    if ($null -ne $ExpectedEnvironment -and -not
        (Test-Staging-EnvironmentMatches -Expected $ExpectedEnvironment -Actual $Observed.Environment)) {
        return $false
    }
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
