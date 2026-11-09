#requires -Version 7.0
<#
Windows staging for the existing Alegria Docker Desktop Hyper-V stack on B:.
Status: pwsh -File infra/local/windows_staging.ps1
Deploy or revert: pwsh -File infra/local/windows_staging.ps1 -Action Deploy -Commit <full SHA>
No parallel project, WSL, docker down, volume deletion, or automatic CMS approval.
#>
[CmdletBinding()]
param(
    [ValidateSet('Status', 'Deploy')][string]$Action = 'Status',
    [ValidatePattern('^[a-f0-9]{40}$')][string]$Commit,
    [ValidateSet('temporal-worker', 'outbox-worker')][string]$Service = 'temporal-worker',
    [string]$HomeOnB = 'B:\Alegria',
    [string]$EnvFile
)
$ErrorActionPreference = 'Stop'
$docker = Join-Path $HomeOnB 'docker\app\resources\bin\docker.exe'
$runtime = Join-Path $HomeOnB 'runtime'
$downloads = Join-Path $HomeOnB 'downloads'
$repo = 'iamaman11/alegria-site'

function Assert-Exit([string]$Stage) {
    if ($LASTEXITCODE -ne 0) { throw "$Stage failed: exit $LASTEXITCODE" }
}
function Container-Label([string]$Name, [string]$Label) {
    $raw = & $docker inspect $Name --format '{{json .Config.Labels}}'
    Assert-Exit "inspect $Name labels"
    $labels = ($raw | Out-String) | ConvertFrom-Json
    return [string]$labels.PSObject.Properties[$Label].Value
}
function Container-State([string]$Name) {
    $raw = & $docker inspect $Name --format '{{.State.Status}}'
    Assert-Exit "inspect $Name state"
    return ($raw | Out-String).Trim()
}
if (-not (Test-Path -LiteralPath $docker)) { throw "Docker CLI absent at $docker" }
$project = Container-Label 'alegria_postgres' 'com.docker.compose.project'
if (-not $project) { throw 'No existing Alegria Compose project: refuse a second database' }
if ((Container-State 'alegria_postgres') -ne 'running') { throw 'Business PostgreSQL is not running' }
if ($Action -eq 'Status') {
    Write-Output "PROJECT=$project"
    foreach ($name in @('alegria_postgres','alegria_postgres_temporal','alegria_pgbouncer',
        'alegria_temporal','alegria_neo4j','alegria_qdrant',
        'alegria_outbox_worker','alegria_temporal_worker')) {
        if ((Container-Label $name 'com.docker.compose.project') -ne $project) {
            throw "Different Compose project detected for $name"
        }
        $state = Container-State $name
        $workdir = Container-Label $name 'com.docker.compose.project.working_dir'
        $source = Split-Path $workdir -Leaf
        Write-Output "$name=$state SOURCE=$source"
    }
    exit 0
}
if (-not $Commit) { throw 'Deploy needs -Commit <40-character GitHub SHA>' }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw 'GitHub CLI is required for exact-SHA CI gate' }
$remote = (& gh api "repos/$repo/commits/$Commit" --jq .sha | Out-String).Trim()
Assert-Exit 'GitHub commit lookup'
if ($remote -ne $Commit) { throw 'GitHub commit SHA does not match' }
$runJson = & gh run list -R $repo --commit $Commit --json databaseId,workflowName,conclusion,status,headSha --limit 100
Assert-Exit 'GitHub Actions lookup'
$runs = $runJson | ConvertFrom-Json
foreach ($name in @('quality-gate', 'automation-verify')) {
    $latest = @($runs | Where-Object { $_.workflowName -eq $name -and $_.headSha -eq $Commit } | Sort-Object databaseId -Descending | Select-Object -First 1)
    if ($latest.Count -ne 1 -or $latest[0].status -ne 'completed' -or $latest[0].conclusion -ne 'success') {
        throw "CI NO-GO: $name not SUCCESS at $Commit"
    }
}
Write-Output "CI=PASS COMMIT=$Commit"
$target = if ($Service -eq 'temporal-worker') {'alegria_temporal_worker'} else {'alegria_outbox_worker'}
if ((Container-State $target) -ne 'running') { throw "$target not running" }
if ((Container-Label $target 'com.docker.compose.project') -ne $project) { throw 'Worker project mismatch' }
$oldRoot = Container-Label $target 'com.docker.compose.project.working_dir'
$oldSha = (Split-Path $oldRoot -Leaf) -replace '^alegria-site-', ''
if ($oldSha -notmatch '^[a-f0-9]{40}$' -or -not (Test-Path (Join-Path $oldRoot 'docker-compose.yml'))) {
    throw 'Previous canonical checkout absent: cannot guarantee rollback'
}
if ($oldSha -eq $Commit) { Write-Output "NOOP: $Service already at $Commit"; exit 0 }
# Staging only: do not drain an actual business run implicitly.
$count = (& $docker exec alegria_postgres psql -U postgres -d alegria -t -A -c 'SELECT count(*) FROM pipeline.execution_runs;' | Out-String).Trim()
Assert-Exit 'business-run safety check'
if ($count -ne '0') { throw "Run history is nonempty ($count): require explicit drain plan" }
if ($EnvFile) {
    $EnvFile = (Resolve-Path -LiteralPath $EnvFile).Path
    if ($EnvFile.StartsWith($runtime, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Provider secrets must live outside immutable Git checkouts'
    }
} else {
    $existing = ((& $docker inspect $target --format '{{json .Config.Env}}') | Out-String) | ConvertFrom-Json
    Assert-Exit 'existing secret-presence check'
    foreach ($key in @('DATAFORSEO_PASSWORD','VOYAGE_API_KEY','OPENAI_API_KEY','ANTHROPIC_API_KEY',
        'GEMINI_API_KEY','GOOGLE_API_KEY','CLOUDFLARE_API_TOKEN')) {
        if (@($existing | Where-Object { $_ -match ('^' + $key + '=.+' ) }).Count) {
            throw 'Provider credentials already in use: specify a private -EnvFile'
        }
    }
}
New-Item -ItemType Directory -Path $runtime,$downloads -Force | Out-Null
$root = Join-Path $runtime "alegria-site-$Commit"
$compose = Join-Path $root 'docker-compose.yml'
if (-not (Test-Path -LiteralPath $compose)) {
    $archive = Join-Path $downloads "alegria-site-$Commit.zip"
    if (-not (Test-Path -LiteralPath $archive)) {
        & "$env:SystemRoot\System32\curl.exe" --fail --location --silent --show-error --connect-timeout 10 --max-time 120 --output $archive "https://codeload.github.com/$repo/zip/$Commit"
        Assert-Exit 'download canonical GitHub archive'
    }
    & "$env:SystemRoot\System32\tar.exe" -xf $archive -C $runtime
    Assert-Exit 'unpack canonical GitHub archive'
}
if (-not (Test-Path -LiteralPath $compose)) { throw 'Missing exact-source Compose file' }
function Compose-Args([string]$Checkout) {
    $list = @('compose')
    if ($EnvFile) { $list += @('--env-file',$EnvFile) }
    return $list + @('-p',$project,'--project-directory',$Checkout,'-f',(Join-Path $Checkout 'docker-compose.yml'))
}
$newArgs = Compose-Args $root
$previousArgs = Compose-Args $oldRoot
& $docker @newArgs config --quiet
Assert-Exit 'Compose config'
$config = ((& $docker @newArgs config --format json) | Out-String) | ConvertFrom-Json
Assert-Exit 'effective Compose config'
foreach ($port in @($config.services.$Service.ports)) {
    if ($port -and $port.host_ip -ne '127.0.0.1') { throw "Unsafe host port on $Service" }
}
if ($Service -eq 'temporal-worker') {
    $nativeMount = @($config.services.$Service.volumes | Where-Object {
        $_.type -eq 'volume' -and $_.target -eq '/app/app/rust/dist'
    })
    if ($nativeMount.Count -ne 1) { throw 'Native persistent release volume is required' }
}
$oldImage = ((& $docker inspect $target --format '{{.Image}}') | Out-String).Trim()
Assert-Exit 'inspect previous image'
& $docker image inspect $oldImage --format '{{.Id}}' | Out-Null
Assert-Exit 'rollback guard: previous image absent'
$rollbackTag = $project + '-' + $Service + ':rollback-' + $oldSha.Substring(0,12)
& $docker image tag $oldImage $rollbackTag
Assert-Exit 'preserve prior image'
Write-Output "ROLLBACK_IMAGE=$rollbackTag PREVIOUS_SHA=$oldSha"
& $docker @newArgs build $Service
Assert-Exit "build $Service"
try {
    & $docker @newArgs up -d --no-deps --no-build --force-recreate $Service
    Assert-Exit "start $Service"
    $ok = $false
    for ($i = 0; $i -lt 45; $i++) {
        if ((Container-State $target) -eq 'running') { $ok = $true; break }
        Start-Sleep -Seconds 2
    }
    if (-not $ok) { throw "$target never became running" }
    if ((Container-Label $target 'com.docker.compose.project') -ne $project) { throw 'Project changed' }
    if ($Service -eq 'temporal-worker') {
        $status = (Invoke-WebRequest 'http://127.0.0.1:9464/metrics' -UseBasicParsing -TimeoutSec 8).StatusCode
        if ($status -ne 200) { throw 'Metrics check failed' }
    }
    Write-Output "DEPLOYED=$Service COMMIT=$Commit PROJECT=$project"
} catch {
    $message = $_.Exception.Message
    Write-Warning "New deploy failed ($message); restoring prior image and config"
    $latestTag = $project + '-' + $Service + ':latest'
    & $docker image tag $rollbackTag $latestTag
    Assert-Exit 'restore previous image'
    & $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service
    Assert-Exit 'restore previous worker'
    if ((Container-State $target) -ne 'running') { throw 'Rollback worker did not start' }
    throw "Deploy rejected; restored $oldSha"
}
