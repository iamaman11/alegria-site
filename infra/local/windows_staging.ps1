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
. (Join-Path $PSScriptRoot 'windows_staging_reconcile.ps1')

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
$oldEnv = ((& $docker inspect $target --format '{{json .Config.Env}}') | Out-String) | ConvertFrom-Json
Assert-Exit 'inspect previous build identity'
$oldBuildId = [string]($oldEnv | Where-Object { $_ -match '^WORKER_BUILD_ID=' } |
    Select-Object -First 1)
$oldBuildId = $oldBuildId -replace '^WORKER_BUILD_ID=', ''
$rollbackTag = $project + '-' + $Service + ':rollback-' + $oldSha.Substring(0,12)
# Preserve the previous image BEFORE building :latest. A preparatory build may
# already have displaced its tag, leaving only the still-running old container.
# Recover only from its exact checkout and demand byte-identical runtime files.
& $docker image inspect $oldImage --format '{{.Id}}' 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) {
    & $docker image tag $oldImage $rollbackTag
    Assert-Exit 'preserve previous image'
} else {
    if ($Service -ne 'temporal-worker') {
        throw 'Previous image absent: recovery is not certified for this service'
    }
    Write-Output "ROLLBACK_RECOVERY=REBUILD_PREVIOUS_SOURCE PREVIOUS_SHA=$oldSha"
    & $docker @previousArgs config --quiet
    Assert-Exit 'previous Compose config'
    & $docker @previousArgs build $Service
    Assert-Exit 'rebuild previous image'
    $recoveredImage = $project + '-' + $Service + ':latest'
    # Create, but never run, a disposable verifier. No provider secret is copied
    # into the image and no second Compose project or persistent service is made.
    $verifier = (& $docker create --network none --entrypoint /bin/true $recoveredImage | Out-String).Trim()
    Assert-Exit 'create recovered-image verifier'
    $scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("alegria-rollback-" + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory -Path $scratch -Force | Out-Null
    try {
        foreach ($file in @(
            'usr/local/bin/temporal_worker',
            'usr/local/bin/temporal_starter',
            'usr/local/bin/cli_tools',
            'app/automation/deploy_seo_static_cloudflare.py',
            'app/automation/check_first_site_release.py',
            'app/automation/check_public_http_readback.py',
            'app/automation/check_public_static_output.py'
        )) {
            $live = Join-Path $scratch 'live'
            $rebuilt = Join-Path $scratch 'rebuilt'
            & $docker cp ($target + ':/' + $file) $live
            Assert-Exit "read running worker file $file"
            & $docker cp ($verifier + ':/' + $file) $rebuilt
            Assert-Exit "read recovered worker file $file"
            if ((Get-FileHash $live -Algorithm SHA256).Hash -ne
                (Get-FileHash $rebuilt -Algorithm SHA256).Hash) {
                throw "Rollback mismatch in $file; keep current worker running"
            }
            Remove-Item -LiteralPath $live,$rebuilt -Force
        }
        & $docker image tag $recoveredImage $rollbackTag
        Assert-Exit 'preserve verified recovered image'
        Write-Output "ROLLBACK_RECOVERY=PASS CHECKED_FILES=7"
    } finally {
        & $docker rm $verifier | Out-Null
        Remove-Item -LiteralPath $scratch -Force -Recurse
    }
}
# The rollback tag is now immutable across the candidate :latest build.
# The old container's .Image can be absent from the local image store; a verified
# rebuild can have a different ID. Compare post-rollback state to this tag.
$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}') | Out-String).Trim()
Assert-Exit 'rollback guard: verified image absent'
if (-not $rollbackImage) { throw 'Rollback guard returned empty immutable image ID' }
Write-Output "ROLLBACK_IMAGE=$rollbackTag PREVIOUS_SHA=$oldSha"
& $docker @newArgs build $Service
Assert-Exit "build $Service"
$latestTag = $project + '-' + $Service + ':latest'
$newImage = ((& $docker image inspect $latestTag --format '{{.Id}}') | Out-String).Trim()
Assert-Exit 'inspect newly built image'
# An operator-only commit can legitimately rebuild to the SAME immutable image.
# Checkout + WORKER_BUILD_ID must still match the requested commit at readback.
if (-not $newImage) {
    throw 'Candidate build has no verifiable image ID'
}

function Test-Docker-Engine-Ready {
    try {
        $version = ((& $docker version --format '{{.Server.Version}}' 2>$null) | Out-String).Trim()
        return ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($version))
    } catch { return $false }
}
function Read-Worker-Observed {
    try {
        $raw = ((& $docker inspect $target --format '{{json .}}' 2>$null) | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $raw) { return $null }
        $info = $raw | ConvertFrom-Json
        $labels = $info.Config.Labels
        $buildEntry = @($info.Config.Env | Where-Object { $_ -match '^WORKER_BUILD_ID=' } |
            Select-Object -First 1)
        $buildId = if ($buildEntry.Count -gt 0) {
            ([string]$buildEntry[0]).Substring('WORKER_BUILD_ID='.Length)
        } else { '' }
        return @{
            State = [string]$info.State.Status
            Image = [string]$info.Image
            Project = [string]$labels.PSObject.Properties['com.docker.compose.project'].Value
            Checkout = [string]$labels.PSObject.Properties['com.docker.compose.project.working_dir'].Value
            BuildId = $buildId
        }
    } catch { return $null }
}
function Test-Worker-Metrics {
    if ($Service -ne 'temporal-worker') { return $true }
    try {
        $response = Invoke-WebRequest 'http://127.0.0.1:9464/metrics' -UseBasicParsing -TimeoutSec 8
        return ($response.StatusCode -eq 200)
    } catch { return $false }
}
function Wait-Worker-Reconciliation([int]$StartExit, [int]$Attempts = 30) {
    # Docker Desktop Hyper-V may restart while Compose is waiting. Command exit
    # -1 is not proof of a failed container start or a successful rollback.
    $lastReady = $false
    for ($i = 0; $i -lt $Attempts; $i++) {
        $lastReady = Test-Docker-Engine-Ready
        if ($lastReady) {
            $observed = Read-Worker-Observed
            $metricsOk = $false
            if ($null -ne $observed -and $observed.State -eq 'running' -and
                ($observed.Image -eq $newImage -or $observed.Image -eq $rollbackImage)) {
                $metricsOk = Test-Worker-Metrics
            }
            $newVerified = Test-Staging-Readback -Observed $observed -ExpectedImage $newImage `
                -ExpectedProject $project -ExpectedCheckout $root `
                -ExpectedBuildId $(if ($Service -eq 'temporal-worker') { $Commit } else { '' }) `
                -MetricsRequired ($Service -eq 'temporal-worker') -MetricsOk $metricsOk
            $oldVerified = Test-Staging-Readback -Observed $observed -ExpectedImage $rollbackImage `
                -ExpectedProject $project -ExpectedCheckout $oldRoot `
                -ExpectedBuildId ([string]$oldBuildId) `
                -MetricsRequired ($Service -eq 'temporal-worker') -MetricsOk $metricsOk
            $decision = Resolve-Staging-Decision -EngineReady $true -NewVerified $newVerified `
                -OldVerified $oldVerified -StartExit $StartExit
            if ($decision -ne 'UNVERIFIED') { return $decision }
            # Never clobber an unknown RUNNING image, even if the Docker CLI
            # returned a failure. It could be a concurrent or external deploy.
            if ($null -ne $observed -and $observed.State -eq 'running' -and
                $observed.Image -ne $newImage -and $observed.Image -ne $rollbackImage) {
                throw 'RECOVERY_REQUIRED: unexpected running image; refuse to overwrite'
            }
        }
        if ($i -lt ($Attempts - 1)) { Start-Sleep -Seconds 2 }
    }
    if (-not $lastReady) { return 'ENGINE_UNAVAILABLE' }
    return 'UNVERIFIED'
}

# Native Docker Desktop can hang indefinitely while Compose waits for its
# own "Starting" progress. Let Compose own creation/configuration, and use
# Docker's standard container start for the actual transition to running.
# The same two-step lifecycle is used for both deploy and rollback.
function Invoke-Compose-Recreate-Start([string[]]$ComposeArgs) {
    $createExit = -1
    try {
        $null = & $docker @ComposeArgs up --no-start --no-deps --no-build --force-recreate $Service
        $createExit = [int]$LASTEXITCODE
    } catch {
        Write-Warning 'Compose create raised an exception; reconciling actual container state'
    }
    if ($createExit -ne 0) { return $createExit }
    try {
        $null = & $docker start $target
        return [int]$LASTEXITCODE
    } catch {
        Write-Warning 'Native Docker start raised an exception; reconciling actual container state'
        return -1
    }
}

$startExit = Invoke-Compose-Recreate-Start -ComposeArgs $newArgs
if ($startExit -ne 0) {
    Write-Warning "Worker create/start returned exit $startExit; checking Docker engine and exact deployed identity before rollback"
}
$decision = Wait-Worker-Reconciliation -StartExit $startExit
if ($decision -eq 'DEPLOYED') {
    Write-Output "DEPLOYED=$Service COMMIT=$Commit PROJECT=$project IMAGE=$newImage"
    exit 0
}
if ($decision -eq 'DEPLOYED_AFTER_START_ERROR') {
    # This is NOT a clean Compose success: preserve the anomaly in the audit
    # and confirm exact image/build/source identity + real metrics first.
    Write-Warning "DEPLOYED_RECOVERED: Compose start failed but identical candidate is running and verified"
    Write-Output "DEPLOYED_RECOVERED=$Service COMMIT=$Commit PROJECT=$project IMAGE=$newImage COMPOSE_EXIT=$startExit"
    exit 0
}
if ($decision -eq 'ENGINE_UNAVAILABLE') {
    throw "RECOVERY_REQUIRED: Docker engine unreachable; preserved rollback image $rollbackTag. No destructive retry."
}
if ($decision -eq 'OLD_RUNNING') {
    # The old container is already healthy. Restore only its :latest alias,
    # not the container itself; a later --no-build must not pick the candidate.
    & $docker image tag $rollbackTag $latestTag
    Assert-Exit 'restore previous latest tag (no container recreate)'
    throw "DEPLOY_REJECTED: previous worker $oldSha is running and verified; candidate not deployed"
}

# Docker is available, but the candidate is not verified. A recoverable
# stopped/created container may be restored only from the immutable old image.
$observed = Read-Worker-Observed
if ($null -ne $observed -and $observed.State -eq 'running') {
    throw 'RECOVERY_REQUIRED: unverified running container; refuse automatic destructive rollback'
}
Write-Warning "Candidate failed verification; restoring immutable old image $oldSha"
& $docker image tag $rollbackTag $latestTag
Assert-Exit 'retag preserved image for rollback'
$rollbackExit = -1
$previousBuildEnvironment = [Environment]::GetEnvironmentVariable('WORKER_BUILD_ID', 'Process')
try {
    # Rollback must not use the NEW commit's WORKER_BUILD_ID from -EnvFile.
    # The original image and the worker identity must remain consistent.
    if ($oldBuildId) {
        $env:WORKER_BUILD_ID = $oldBuildId
    } else {
        Remove-Item Env:WORKER_BUILD_ID -ErrorAction SilentlyContinue
    }
    $rollbackConfig = ((& $docker @previousArgs config --format json) | Out-String) | ConvertFrom-Json
    Assert-Exit 'rollback effective Compose config'
    if ($oldBuildId -and $rollbackConfig.services.$Service.environment.WORKER_BUILD_ID -ne $oldBuildId) {
        throw 'Rollback build identity does not match previous container; refuse recreate'
    }
    $rollbackExit = Invoke-Compose-Recreate-Start -ComposeArgs $previousArgs
} finally {
    if ($null -eq $previousBuildEnvironment) {
        Remove-Item Env:WORKER_BUILD_ID -ErrorAction SilentlyContinue
    } else {
        $env:WORKER_BUILD_ID = $previousBuildEnvironment
    }
}
$rollbackDecision = Wait-Worker-Reconciliation -StartExit $rollbackExit
if ($rollbackDecision -eq 'OLD_RUNNING') {
    throw "DEPLOY_REJECTED_ROLLED_BACK: old worker $oldSha is verified; candidate not deployed"
}
if ($rollbackDecision -eq 'ENGINE_UNAVAILABLE') {
    throw "RECOVERY_REQUIRED: Docker engine unavailable after rollback request; preserved $rollbackTag"
}
throw "RECOVERY_REQUIRED: rollback not verified ($rollbackDecision); preserve $rollbackTag and inspect container before retry"
