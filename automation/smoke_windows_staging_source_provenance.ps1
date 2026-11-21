#requires -Version 7.0
# Execute the REAL Windows source/CI proof against mocked GitHub responses.
# Neither Docker nor network credentials nor staging volumes are touched.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$operator = [System.IO.File]::ReadAllText((Join-Path $root 'infra/local/windows_staging.ps1'))
$begin = '# A two-parent accepted main merge must have the SAME Git tree'
$end = 'Write-Output "CI=PASS COMMIT=$Commit SOURCE_SHA=$sourceSha"'
$start = $operator.IndexOf($begin, [System.StringComparison]::Ordinal)
$stop = $operator.IndexOf($end, [System.StringComparison]::Ordinal)
if ($start -lt 0 -or $stop -le $start) {
    throw 'Unable to extract canonical Windows signed-source proof'
}
$sourceProof = $operator.Substring($start, $stop - $start)
$script:MergeSha = 'a' * 40
$script:SourceSha = 'b' * 40
$script:ExpectedTree = 'c' * 40
$script:Scenario = 'pass'

function Assert-Exit([string]$Stage) {
    # gh is fully replaced by this deterministic stub, never a native process.
}

function gh {
    $argv = @($args)
    if ($argv.Count -lt 2) { throw 'unexpected gh invocation' }
    if ($argv[0] -eq 'api') {
        switch ([string]$argv[1]) {
            "repos/iamaman11/alegria-site/commits/$script:MergeSha" {
                return (@{ parents = @(@{sha = ('d' * 40)}, @{sha = $script:SourceSha}) } | ConvertTo-Json -Depth 8 -Compress)
            }
            "repos/iamaman11/alegria-site/git/commits/$script:MergeSha" {
                return $script:ExpectedTree
            }
            "repos/iamaman11/alegria-site/git/commits/$script:SourceSha" {
                if ($script:Scenario -eq 'wrong_tree') { return ('e' * 40) }
                return $script:ExpectedTree
            }
            "repos/iamaman11/alegria-site/commits/$script:MergeSha/pulls" {
                if ($script:Scenario -eq 'missing_pr') { return '[]' }
                return (@(@{
                    merged_at = '2026-11-21T19:00:00Z'
                    merge_commit_sha = $script:MergeSha
                    head = @{ sha = $script:SourceSha }
                    base = @{ ref = 'main' }
                }) | ConvertTo-Json -Depth 8 -Compress)
            }
            "repos/iamaman11/alegria-site/actions/runs/42/jobs?per_page=100" {
                $jobs = @(
                    @{ name = 'CI'; conclusion = 'success' },
                    @{ name = 'comprehensive'; conclusion = 'success' }
                )
                if ($script:Scenario -ne 'missing_package') {
                    $jobs += @{ name = 'package'; conclusion = 'success' }
                }
                return (@{ jobs = $jobs } | ConvertTo-Json -Depth 8 -Compress)
            }
        }
    }
    if ($argv[0] -eq 'run' -and $argv[1] -eq 'list') {
        $head = if ($script:Scenario -eq 'wrong_head') { 'f' * 40 } else { $script:SourceSha }
        $conclusion = if ($script:Scenario -eq 'failed_ci') { 'failure' } else { 'success' }
        return (@(@{
            databaseId = 42
            workflowName = 'ci'
            event = 'pull_request'
            status = 'completed'
            conclusion = $conclusion
            headSha = $head
        }) | ConvertTo-Json -Depth 8 -Compress)
    }
    throw "unexpected gh mock invocation: $($argv -join ' ')"
}

function Invoke-Case([string]$Case, [string]$ExpectedError) {
    $script:Scenario = $Case
    $repo = 'iamaman11/alegria-site'
    $Commit = $script:MergeSha
    $Service = 'temporal-worker'
    $sourceSha = ''
    $imageRunId = ''
    $actualError = ''
    try {
        Invoke-Expression $sourceProof
    } catch {
        $actualError = $_.Exception.Message
    }
    if ($ExpectedError) {
        if (-not $actualError.Contains($ExpectedError)) {
            throw "Scenario $Case failed closed incorrectly: $actualError"
        }
    } else {
        if ($actualError) { throw "Valid source rejected: $actualError" }
        if ($sourceSha -cne $script:SourceSha -or $imageRunId -cne '42') {
            throw 'Accepted source or image run was not preserved by real operator code'
        }
    }
}

Invoke-Case 'pass' ''
Invoke-Case 'wrong_tree' 'different source trees'
Invoke-Case 'missing_pr' 'no unique accepted main PR'
Invoke-Case 'missing_package' 'package did not pass'
Invoke-Case 'wrong_head' 'no successful exact-head PR CI'
Invoke-Case 'failed_ci' 'no successful exact-head PR CI'
Write-Output 'WINDOWS_STAGING_SOURCE_PROVENANCE=PASS scenarios=6'
