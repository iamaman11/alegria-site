#!/usr/bin/env python3
"""Guard the Windows staging rollback contract before a physical deployment.

This is a source-boundary regression: the real Docker image and file hashes
are additionally checked by windows_staging.ps1 on the Windows machine.
"""
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "infra/local/windows_staging.ps1"
COMPOSE = ROOT / "docker-compose.yml"


def check(script: str) -> None:
    required = [
        "if ($oldSha -eq $Commit)",
        "if ($count -ne '0')",
        "if ($EnvFile)",
        "if ($Service -ne 'temporal-worker')",
        "& $docker @previousArgs build $Service",
        "& $docker create --network none --entrypoint /bin/true $recoveredImage",
        "& $docker cp ($target + ':/' + $file) $live",
        "& $docker cp ($verifier + ':/' + $file) $rebuilt",
        "Get-FileHash $live -Algorithm SHA256",
        "Get-FileHash $rebuilt -Algorithm SHA256",
        "throw \"Rollback mismatch in $file; keep current worker running\"",
        "& $docker image tag $recoveredImage $rollbackTag",
        "& $docker image inspect $rollbackTag --format '{{.Id}}'",
        "Assert-Exit 'rollback guard: verified image absent'",
        "$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}')",
        "-ExpectedImage $rollbackImage",
        "$observed.Image -ne $newImage -and $observed.Image -ne $rollbackImage",
        "& $docker @newArgs build $Service",
        "& $docker @newArgs up -d --no-deps --no-build --force-recreate $Service",
        "& $docker image tag $rollbackTag $latestTag",
        "& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service",
        ". (Join-Path $PSScriptRoot 'windows_staging_reconcile.ps1')",
        "function Test-Docker-Engine-Ready",
        "function Read-Worker-Observed",
        "function Wait-Worker-Reconciliation",
        "$decision = Wait-Worker-Reconciliation -StartExit $startExit",
        "Test-Staging-Readback -Observed $observed",
        "Resolve-Staging-Decision -EngineReady $true",
        "if ($decision -eq 'DEPLOYED_AFTER_START_ERROR')",
        'Write-Output "DEPLOYED_RECOVERED=$Service',
        "if ($decision -eq 'ENGINE_UNAVAILABLE')",
        "if ($decision -eq 'OLD_RUNNING')",
        "if ($null -ne $observed -and $observed.State -eq 'running')",
        "throw 'RECOVERY_REQUIRED: unverified running container",
        'throw "DEPLOY_REJECTED_ROLLED_BACK:',
        "if ($rollbackDecision -eq 'OLD_RUNNING')",
        "Rollback build identity does not match previous container; refuse recreate",
        "$env:WORKER_BUILD_ID = $oldBuildId",
        "$rollbackConfig = ((& $docker @previousArgs config --format json)",
        "$env:WORKER_BUILD_ID = $previousBuildEnvironment",

    ]
    for snippet in required:
        assert snippet in script, f"missing safety check: {snippet}"
    recovery = script.index("& $docker @previousArgs build $Service")
    compare = script.index("Get-FileHash $live -Algorithm SHA256")
    preserve = script.index("& $docker image tag $recoveredImage $rollbackTag")
    guard = script.index("Assert-Exit 'rollback guard: verified image absent'")
    candidate_build = script.index("& $docker @newArgs build $Service")
    recreate = script.index("& $docker @newArgs up -d --no-deps --no-build --force-recreate $Service")
    rollback_image = script.index("$rollbackImage = ((& $docker image inspect $rollbackTag")
    assert recovery < compare < preserve < rollback_image < guard < candidate_build < recreate
    readback = script.index("$decision = Wait-Worker-Reconciliation -StartExit $startExit")
    rollback_tag = script.index("& $docker image tag $rollbackTag $latestTag")
    assert recreate < readback < rollback_tag, (
        "inspect exact candidate after a Docker start error BEFORE changing rollback tags"
    )
    restored_identity = script.index("$env:WORKER_BUILD_ID = $oldBuildId")
    checked_identity = script.index("Rollback build identity does not match previous container")
    rollback_up = script.index("& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service")
    assert rollback_tag < restored_identity < checked_identity < rollback_up, (
        "restore old build identity and verify config before attempting rollback"
    )
    assert "Assert-Exit \"start $Service\"" not in script, (
        "start exit status must be reconciled against real container identity"
    )
    assert "Assert-Exit 'restore previous worker'" not in script, (
        "a failed rollback start must be read back, not assumed"
    )
    assert "Candidate build did not produce a distinct" not in script, (
        "operator-only commits may legitimately rebuild to the same Docker image ID"
    )
    assert "Remove-Item -LiteralPath $scratch -Force -Recurse" in script
    assert "& $docker rm $verifier" in script
    assert "RETRIEVAL_CAPABILITY_REQUIRED" not in script  # no bypass of publication gates


def main() -> None:
    source = SCRIPT.read_text(encoding="utf-8")
    check(source)
    # Enforce native Docker restart policies for always-on Alegria services.
    compose = COMPOSE.read_text(encoding='utf-8')
    parts = re.split(r'(?m)^  ([a-z][a-z0-9-]*):\s*
    # Negative mutation cases must fail closed, not silently pass.
    for fragment in (
        "Get-FileHash $live -Algorithm SHA256",
        "& $docker image inspect $rollbackTag --format '{{.Id}}'",
        "throw \"Rollback mismatch in $file; keep current worker running\"",
        "& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service",
        "$decision = Wait-Worker-Reconciliation -StartExit $startExit",
        "if ($decision -eq 'ENGINE_UNAVAILABLE')",
        "if ($decision -eq 'DEPLOYED_AFTER_START_ERROR')",
        "$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}')",
    ):
        mutated = source.replace(fragment, "", 1)
        try:
            check(mutated)
        except AssertionError:
            pass
        else:
            raise AssertionError(f"mutation did not fail: {fragment}")
    print("WINDOWS_STAGING_ROLLBACK=PASS")


if __name__ == "__main__":
    main()
, compose)
    services = dict(zip(parts[1::2], parts[2::2]))
    for name in ('postgres', 'postgres-temporal', 'pgbouncer', 'neo4j',
                 'qdrant', 'temporal-server', 'temporal-worker', 'outbox-worker'):
        assert re.search(r'(?m)^    restart: unless-stopped\s*
    # Negative mutation cases must fail closed, not silently pass.
    for fragment in (
        "Get-FileHash $live -Algorithm SHA256",
        "& $docker image inspect $rollbackTag --format '{{.Id}}'",
        "throw \"Rollback mismatch in $file; keep current worker running\"",
        "& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service",
        "$decision = Wait-Worker-Reconciliation -StartExit $startExit",
        "if ($decision -eq 'ENGINE_UNAVAILABLE')",
        "if ($decision -eq 'DEPLOYED_AFTER_START_ERROR')",
        "$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}')",
    ):
        mutated = source.replace(fragment, "", 1)
        try:
            check(mutated)
        except AssertionError:
            pass
        else:
            raise AssertionError(f"mutation did not fail: {fragment}")
    print("WINDOWS_STAGING_ROLLBACK=PASS")


if __name__ == "__main__":
    main()
, services[name]), name
    for name in ('temporal-schema', 'temporal-namespace'):
        assert re.search(r'(?m)^    restart: "no"\s*
    # Negative mutation cases must fail closed, not silently pass.
    for fragment in (
        "Get-FileHash $live -Algorithm SHA256",
        "& $docker image inspect $rollbackTag --format '{{.Id}}'",
        "throw \"Rollback mismatch in $file; keep current worker running\"",
        "& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service",
        "$decision = Wait-Worker-Reconciliation -StartExit $startExit",
        "if ($decision -eq 'ENGINE_UNAVAILABLE')",
        "if ($decision -eq 'DEPLOYED_AFTER_START_ERROR')",
        "$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}')",
    ):
        mutated = source.replace(fragment, "", 1)
        try:
            check(mutated)
        except AssertionError:
            pass
        else:
            raise AssertionError(f"mutation did not fail: {fragment}")
    print("WINDOWS_STAGING_ROLLBACK=PASS")


if __name__ == "__main__":
    main()
, services[name]), name


    # Negative mutation cases must fail closed, not silently pass.
    for fragment in (
        "Get-FileHash $live -Algorithm SHA256",
        "& $docker image inspect $rollbackTag --format '{{.Id}}'",
        "throw \"Rollback mismatch in $file; keep current worker running\"",
        "& $docker @previousArgs up -d --no-deps --no-build --force-recreate $Service",
        "$decision = Wait-Worker-Reconciliation -StartExit $startExit",
        "if ($decision -eq 'ENGINE_UNAVAILABLE')",
        "if ($decision -eq 'DEPLOYED_AFTER_START_ERROR')",
        "$rollbackImage = ((& $docker image inspect $rollbackTag --format '{{.Id}}')",
    ):
        mutated = source.replace(fragment, "", 1)
        try:
            check(mutated)
        except AssertionError:
            pass
        else:
            raise AssertionError(f"mutation did not fail: {fragment}")
    print("WINDOWS_STAGING_ROLLBACK=PASS")


if __name__ == "__main__":
    main()
