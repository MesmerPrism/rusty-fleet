# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$repoRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot "Fleet.ValidationGuardrails.psm1") -Force
$receiptSchemaPath = Join-Path $repoRoot `
    "schemas/rusty.fleet.validation_run_receipt.v1.schema.json"

function Assert-True([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
}

function Write-Json([string] $Path, [object] $Value) {
    $Value | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
}

function Get-TestSha256Text([string] $Value) {
    [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))
    ).ToLowerInvariant()
}

function Get-TestReceiptPath([string] $Root, [object] $Receipt) {
    Join-Path $Root ("artifacts/validation/{0}.json" -f [string]$Receipt.run_id)
}

function Get-TestPendingReceiptPath([string] $Root, [object] $Receipt) {
    Join-Path $Root (
        "artifacts/validation/{0}.in-progress.json" -f [string]$Receipt.run_id
    )
}

$scratch = Join-Path ([IO.Path]::GetTempPath()) ("fleet-validation-{0}" -f [guid]::NewGuid())
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    & git -C $scratch init --quiet
    & git -C $scratch config user.email "validation@example.invalid"
    & git -C $scratch config user.name "Fleet Validation Self-Test"
    New-Item -ItemType Directory -Path (Join-Path $scratch "docs"), (Join-Path $scratch "schemas"), (Join-Path $scratch "tools") | Out-Null
    Set-Content -LiteralPath (Join-Path $scratch "README.md") -Value "baseline" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch ".gitignore") -Value "artifacts/" -Encoding utf8NoBOM
    & git -C $scratch add README.md .gitignore
    & git -C $scratch commit --quiet -m baseline
    $base = (& git -C $scratch rev-parse HEAD).Trim()

    $config = Get-Content -LiteralPath (Join-Path $repoRoot "config/fleet-validation-risk.v1.json") -Raw |
        ConvertFrom-Json -Depth 100
    Assert-True (
        Test-FleetProductionValidationConfig (
            Join-Path $repoRoot "config/fleet-validation-risk.v1.json"
        )
    ) "Canonical production validation configuration was rejected."
    $damagedProductionConfigs = @()
    $loweredAuthority = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    $releaseRule = $loweredAuthority.risk_rules | Where-Object id -EQ "release-authority"
    $releaseRule.patterns = @($releaseRule.patterns | Where-Object { $_ -cne "tools/Test-Repo.ps1" })
    $damagedProductionConfigs += $loweredAuthority
    $narrowedWorkflowAuthority = $config | ConvertTo-Json -Depth 100 |
        ConvertFrom-Json -Depth 100
    $narrowedRule = $narrowedWorkflowAuthority.risk_rules |
        Where-Object id -EQ "release-authority"
    $narrowedRule.patterns = @(
        $narrowedRule.patterns | ForEach-Object {
            if ($_ -ceq ".github/**") { ".github/workflows/**" } else { $_ }
        }
    )
    $damagedProductionConfigs += $narrowedWorkflowAuthority
    foreach ($releaseFloor in @(
        "apps/fleet-setup/**",
        "packaging/windows/**",
        "tools/Test-WindowsDistribution.ps1"
    )) {
        $loweredFloor = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $floorRule = $loweredFloor.risk_rules | Where-Object id -EQ "release-authority"
        $floorRule.patterns = @($floorRule.patterns | Where-Object { $_ -cne $releaseFloor })
        $damagedProductionConfigs += $loweredFloor
    }
    $changedCheck = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($changedCheck.checks | Where-Object minimum_profile -EQ "focused").arguments[1] = "Standard"
    $damagedProductionConfigs += $changedCheck
    $removedDeviceRejection = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    $removedDeviceRejection.forbidden_command_patterns = @(
        $removedDeviceRejection.forbidden_command_patterns |
            Where-Object { $_ -cne "(?i)\b(?:adb|fastboot)\b" }
    )
    $damagedProductionConfigs += $removedDeviceRejection
    for ($index = 0; $index -lt $damagedProductionConfigs.Count; $index++) {
        $damagedPath = Join-Path $scratch "damaged-production-$index.json"
        Write-Json $damagedPath $damagedProductionConfigs[$index]
        try {
            Test-FleetProductionValidationConfig $damagedPath
            throw "Damaged production validation configuration $index was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "production|authority") `
                "Damaged production configuration $index failed incorrectly."
        } finally {
            Remove-Item -LiteralPath $damagedPath -Force -ErrorAction SilentlyContinue
        }
    }
    $config.checks = @(
        [ordered]@{
            id = "synthetic-pass"; minimum_profile = "focused"; file = "tools/pass.ps1"
            arguments = @("alpha value", "", 'quoted"value', 'trailing path\', "-Flag")
            timeout_seconds = 10
        },
        [ordered]@{
            id = "synthetic-standard"; minimum_profile = "standard"; file = "tools/drift.ps1"
            arguments = @(); timeout_seconds = 10
        },
        [ordered]@{
            id = "synthetic-timeout"; minimum_profile = "release"; file = "tools/timeout.ps1"
            arguments = @(); timeout_seconds = 1
        }
    )
    $config.heartbeat_seconds = 1
    $config.receipt_directory = "artifacts/validation"
    $configPath = Join-Path $scratch "risk.json"
    Write-Json $configPath $config
    Set-Content -LiteralPath (Join-Path $scratch "tools/pass.ps1") -Value @'
param(
    [string] $First,
    [AllowEmptyString()][string] $Empty,
    [string] $Quoted,
    [string] $Trailing,
    [switch] $Flag
)
if (
    $First -cne "alpha value" -or
    $Empty -cne "" -or
    $Quoted -cne 'quoted"value' -or
    $Trailing -cne 'trailing path\' -or
    -not $Flag
) {
    throw "Synthetic arguments were not splatted as distinct parameters."
}
exit 0
'@ -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/drift.ps1") `
        -Value 'Set-Content -LiteralPath (Join-Path $PSScriptRoot "../README.md") -Value "drift"' -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/index-drift.ps1") `
        -Value '& git add README.md; exit $LASTEXITCODE' -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/timeout.ps1") -Value @'
Write-Output "timeout stdout survived"
[Console]::Error.WriteLine("timeout stderr survived")
$child = Start-Process -FilePath "pwsh" -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Seconds 30") -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 30
'@ -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/fail-child.ps1") -Value @'
$child = Start-Process -FilePath "pwsh" -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Seconds 30") -PassThru -WindowStyle Hidden
exit 7
'@ -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/transient-child.ps1") -Value @'
$child = Start-Process -FilePath "pwsh" -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Milliseconds 750") -PassThru -WindowStyle Hidden
exit 0
'@ -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/leaking-child.ps1") -Value @'
$child = Start-Process -FilePath "pwsh" -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Seconds 30") -PassThru -WindowStyle Hidden
exit 0
'@ -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/throw.ps1") `
        -Value 'throw "synthetic terminating error"' -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "tools/recovered-native.ps1") -Value @'
& (Join-Path $PSHOME "pwsh.exe") -NoProfile -NonInteractive -Command "exit 9"
if ($LASTEXITCODE -ne 9) { throw "Synthetic native failure did not execute." }
Write-Output "native failure handled by the check"
'@ -Encoding utf8NoBOM
    & git -C $scratch add .
    & git -C $scratch commit --quiet -m harness

    Set-Content -LiteralPath (Join-Path $scratch "docs/note.md") -Value "focused" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "schemas/new.json") -Value "{}" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $scratch "Cargo.toml") -Value "[workspace]" -Encoding utf8NoBOM
    & git -C $scratch add docs/note.md schemas/new.json Cargo.toml
    & git -C $scratch commit --quiet -m risk-fixtures
    $first = Get-FleetChangeRiskManifest $scratch $configPath $base
    $second = Get-FleetChangeRiskManifest $scratch $configPath $base
    Assert-True ($first.required_profile -eq "release") "Highest-risk-wins classification failed."
    Assert-True (
        ($first | ConvertTo-Json -Depth 100 -Compress) -ceq
        ($second | ConvertTo-Json -Depth 100 -Compress)
    ) "Classification is not deterministic."
    Assert-True (
        (($first.paths -join ",") -ceq (($first.paths | Sort-Object -CaseSensitive) -join ","))
    ) `
        "Changed paths are not deterministically sorted."

    Set-Content -LiteralPath (Join-Path $scratch "docs/path with space.md") -Value "dirty" -Encoding utf8NoBOM
    $mixed = Get-FleetChangeRiskManifest $scratch $configPath $base
    Assert-True ($mixed.paths -ccontains "docs/path with space.md") `
        "Base-commit classification omitted an uncommitted path or damaged a spaced path."
    Remove-Item -LiteralPath (Join-Path $scratch "docs/path with space.md") -Force

    & git -C $scratch mv schemas/new.json docs/renamed.json
    $renamed = Get-FleetChangeRiskManifest $scratch $configPath $base
    Assert-True (
        $renamed.paths -ccontains "schemas/new.json" -and
        $renamed.paths -ccontains "docs/renamed.json"
    ) "Rename classification did not retain both source and target paths."
    & git -C $scratch reset --quiet --hard HEAD

    New-Item -ItemType Directory -Path (Join-Path $scratch "config") | Out-Null
    Set-Content -LiteralPath (Join-Path $scratch "config/policy.json") -Value "{}" -Encoding utf8NoBOM
    $authorityChange = Get-FleetChangeRiskManifest $scratch $configPath $null
    Assert-True ($authorityChange.required_profile -eq "release") `
        "Validation-authority changes did not require release validation."
    Remove-Item -LiteralPath (Join-Path $scratch "config") -Recurse -Force

    foreach ($releasePath in @(
        ".github/CODEOWNERS",
        "apps/fleet-setup/RustyFleet.Setup.csproj",
        "config/fleet-pull-request-authority.v1.json",
        "packaging/windows/Publish-WindowsRelease.ps1",
        "schemas/rusty.fleet.pull_request_authority_assessment.v1.schema.json",
        "tools/Test-FleetPullRequestAuthority.ps1",
        "tools/Test-WindowsDistribution.ps1"
    )) {
        $fullReleasePath = Join-Path $scratch $releasePath
        New-Item -ItemType Directory -Path (Split-Path -Parent $fullReleasePath) -Force |
            Out-Null
        Set-Content -LiteralPath $fullReleasePath -Value "release authority" -Encoding utf8NoBOM
        $releaseManifest = Get-FleetChangeRiskManifest $scratch $configPath $null
        Assert-True ($releaseManifest.required_profile -eq "release") `
            "Release authority path '$releasePath' did not select release validation."
        Remove-Item -LiteralPath $fullReleasePath -Force
    }

    New-Item -ItemType Directory -Path (Join-Path $scratch "crates/example/src") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $scratch "crates/example/src/lib.rs") -Value "pub fn example() {}" -Encoding utf8NoBOM
    $ordinaryCode = Get-FleetChangeRiskManifest $scratch $configPath $null
    Assert-True ($ordinaryCode.required_profile -eq "standard") `
        "Ordinary product code did not select standard validation."
    Remove-Item -LiteralPath (Join-Path $scratch "crates") -Recurse -Force

    New-Item -ItemType Directory -Path (Join-Path $scratch "morphospace/iteration-units") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $scratch "morphospace/iteration-units/example.json") -Value "{}" -Encoding utf8NoBOM
    $ordinaryUnit = Get-FleetChangeRiskManifest $scratch $configPath $null
    Assert-True ($ordinaryUnit.required_profile -eq "standard") `
        "Normal Morphospace unit evidence did not select standard validation."
    Remove-Item -LiteralPath (Join-Path $scratch "morphospace") -Recurse -Force

    Set-Content -LiteralPath (Join-Path $scratch "README.md") -Value "focused documentation" -Encoding utf8NoBOM
    $ordinaryDocs = Get-FleetChangeRiskManifest $scratch $configPath $null
    Assert-True ($ordinaryDocs.required_profile -eq "focused") `
        "Ordinary documentation did not select focused validation."
    & git -C $scratch checkout --quiet -- README.md

    Assert-True ((Resolve-FleetProfile $config "Quick" "focused") -eq "focused") "Quick alias failed."
    Assert-True ((Resolve-FleetProfile $config "Standard" "focused") -eq "standard") "Standard alias failed."
    Assert-True ((Resolve-FleetProfile $config "Deep" "focused") -eq "release") "Deep alias failed."
    try { Resolve-FleetProfile $config "unknown" "focused"; throw "Unknown profile was accepted." } catch {
        Assert-True ($_.Exception.Message -match "Unknown validation profile") "Unknown profile failed incorrectly."
    }
    foreach ($wrongCase in @("quick", "FOCUSED")) {
        try {
            Resolve-FleetProfile $config $wrongCase "focused"
            throw "Wrong-case profile '$wrongCase' was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "Unknown validation profile") `
                "Wrong-case profile '$wrongCase' failed incorrectly."
        }
    }
    try { Resolve-FleetProfile $config "focused" "standard"; throw "Downgrade was accepted." } catch {
        Assert-True ($_.Exception.Message -match "downgrade") "Downgrade failed incorrectly."
    }

    & git -C $scratch reset --quiet --hard HEAD^
    try {
        Invoke-FleetValidationGuardrail $scratch $configPath $null $null
        throw "Clean no-base automatic plan was accepted."
    } catch {
        Assert-True ($_.Exception.Message -match "requires -BaseCommit") `
            "Clean no-base automatic plan failed incorrectly."
    }
    $plan = Invoke-FleetValidationGuardrail $scratch $configPath "Quick" $null
    Assert-True ($plan.result -eq "planned" -and $plan.mode -eq "plan") "Default mode was not read-only plan."
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $scratch "artifacts"))) `
        "Read-only planning wrote an evidence artifact."
    $executePlanned = $plan | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    $executePlanned.mode = "execute"
    try {
        Test-FleetValidationRunReceipt -Receipt $executePlanned `
            -RepositoryRoot $scratch -ConfigPath $configPath
        throw "Execute-mode planned aggregate was accepted."
    } catch {
        Assert-True ($_.Exception.Message -match "planned aggregate") `
            "Execute-mode planned aggregate failed incorrectly."
    }
    $inProgress = $plan | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    $inProgress.mode = "execute"
    $inProgress.result = "in_progress"
    Assert-True (
        Test-FleetValidationRunReceipt -Receipt $inProgress `
            -RepositoryRoot $scratch -ConfigPath $configPath
    ) "Owner receipt validation rejected a valid in-progress receipt."
    Assert-True (
        Test-Json -Json ($inProgress | ConvertTo-Json -Depth 100) `
            -SchemaFile $receiptSchemaPath -ErrorAction Stop
    ) "Receipt schema rejected a valid in-progress receipt."
    $cleanPreRejection = $inProgress | ConvertTo-Json -Depth 100 |
        ConvertFrom-Json -Depth 100
    $cleanPreRejection.result = "rejected"
    try {
        Test-FleetValidationRunReceipt -Receipt $cleanPreRejection `
            -RepositoryRoot $scratch -ConfigPath $configPath
        throw "Clean pre-execution rejection was accepted."
    } catch {
        Assert-True ($_.Exception.Message -match "pre-execution rejection") `
            "Clean pre-execution rejection failed for the wrong reason."
    }
    Assert-True (
        -not (Test-Json -Json ($cleanPreRejection | ConvertTo-Json -Depth 100) `
            -SchemaFile $receiptSchemaPath -ErrorAction SilentlyContinue)
    ) "Receipt schema accepted a clean pre-execution rejection."

    Set-Content -LiteralPath (Join-Path $scratch "README.md") -Value "explicit dirty source" -Encoding utf8NoBOM
    $dirtyRejectPath = $null
    try {
        $dirtyReject = Invoke-FleetValidationGuardrail $scratch $configPath "focused" $null `
            -Execute
        $dirtyRejectPath = Get-TestReceiptPath $scratch $dirtyReject
        Assert-True (
            $dirtyReject.result -eq "rejected" -and
            $dirtyReject.commands[0].status -eq "planned" -and
            -not $dirtyReject.allow_dirty_source
        ) "Dirty execution did not reject before running a command."
        Assert-True (
            Test-FleetValidationRunReceipt -Receipt $dirtyReject `
                -RepositoryRoot $scratch -ConfigPath $configPath
        ) "Owner receipt validation rejected a valid pre-execution rejection."
        $wrongRule = $dirtyReject | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $wrongRule.risk_manifest.classifications[0].rule_id = "release-authority"
        try {
            Test-FleetValidationRunReceipt -Receipt $wrongRule `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Receipt classification differing from the bound configuration was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "bound configuration") `
                "Wrong-rule receipt failed for the wrong reason."
        }
    } finally {
        & git -C $scratch checkout --quiet -- README.md
        if ($dirtyRejectPath) {
            Remove-Item -LiteralPath $dirtyRejectPath -Force -ErrorAction SilentlyContinue
        }
    }

    $executePath = $null
    try {
        $execute = Invoke-FleetValidationGuardrail $scratch $configPath "focused" $null -Execute
        $executePath = Get-TestReceiptPath $scratch $execute
        if ($execute.result -ne "passed") {
            Write-Host ($execute | ConvertTo-Json -Depth 100)
        }
        Assert-True ($execute.result -eq "passed") "Synthetic passing execution failed."
        Assert-True (
            $execute.git.before.commit -eq $execute.git.after.commit -and
            $execute.git.before.tree -eq $execute.git.after.tree -and
            -not $execute.git.drift_detected
        ) "Exact Git drift evidence failed."
        $typed = Get-Content -LiteralPath $executePath -Raw | ConvertFrom-Json -Depth 100
        Assert-True (
            Test-FleetValidationRunReceipt -Receipt $typed `
                -RepositoryRoot $scratch -ConfigPath $configPath
        ) "Owner receipt validation rejected a valid receipt."
        foreach ($field in @(
            "requested_profile",
            "effective_profile",
            "allow_dirty_source",
            "config_sha256",
            "risk_manifest",
            "commands",
            "result",
            "limitations"
        )) {
            Assert-True ($typed.PSObject.Properties.Name -contains $field) "Receipt field missing: $field"
        }
        $damaged = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $damaged.commands[0].status = "failed"
        try {
            Test-FleetValidationRunReceipt -Receipt $damaged `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Damaged passing receipt was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "passing receipt|failed command receipt") `
                "Damaged receipt failed for the wrong reason."
        }
        $falseFailure = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $falseFailure.result = "failed"
        try {
            Test-FleetValidationRunReceipt -Receipt $falseFailure `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Failure without failing command evidence was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "failed receipt") `
                "False failure receipt failed incorrectly."
        }
        $falseRejection = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $falseRejection.result = "rejected"
        try {
            Test-FleetValidationRunReceipt -Receipt $falseRejection `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Post-execution rejection without drift was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "lacks (?:exact )?drift") `
                "False rejection receipt failed incorrectly."
        }
        Assert-True (
            -not (Test-Json -Json ($falseRejection | ConvertTo-Json -Depth 100) `
                -SchemaFile $receiptSchemaPath -ErrorAction SilentlyContinue)
        ) "Receipt schema accepted a post-execution rejection without drift."
        $profileBypass = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $profileBypass.requested_profile = "release"
        $profileBypass.effective_profile = "focused"
        try {
            Test-FleetValidationRunReceipt -Receipt $profileBypass `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Requested release profile was allowed to resolve to focused."
        } catch {
            Assert-True ($_.Exception.Message -match "upward-only requested decision") `
                "Profile-bypass receipt failed for the wrong reason."
        }
        Assert-True (
            -not (Test-Json -Json ($profileBypass | ConvertTo-Json -Depth 100) `
                -SchemaFile $receiptSchemaPath -ErrorAction SilentlyContinue)
        ) "Receipt schema accepted a requested/effective profile bypass."
        $hiddenDrift = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $hiddenDrift.git.after.commit = "0000000000000000000000000000000000000000"
        try {
            Test-FleetValidationRunReceipt -Receipt $hiddenDrift `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Passing receipt with concealed Git drift was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "drift flag") `
                "Concealed-drift receipt failed for the wrong reason."
        }
        $manifestMismatch = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $manifestMismatch.risk_manifest.git.commit = "0000000000000000000000000000000000000000"
        try {
            Test-FleetValidationRunReceipt -Receipt $manifestMismatch `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Receipt snapshot differing from manifest Git fields was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "before-snapshot 'commit'") `
                "Manifest-mismatch receipt failed for the wrong reason."
        }
        $dirtyWithoutAuthority = $typed | ConvertTo-Json -Depth 100 |
            ConvertFrom-Json -Depth 100
        $dirtyWithoutAuthority.git.before.clean = $false
        $dirtyWithoutAuthority.git.after.clean = $false
        $dirtyWithoutAuthority.risk_manifest.git.clean = $false
        $dirtyWithoutAuthority.risk_manifest.git.snapshot_sha256 = Get-TestSha256Text (
            $dirtyWithoutAuthority.git.before | ConvertTo-Json -Depth 100 -Compress
        )
        try {
            Test-FleetValidationRunReceipt -Receipt $dirtyWithoutAuthority `
                -RepositoryRoot $scratch -ConfigPath $configPath
            throw "Passing dirty-source receipt without explicit authority was accepted."
        } catch {
            Assert-True ($_.Exception.Message -match "dirty-source authority") `
                "Dirty-source receipt failed for the wrong reason."
        }
        Assert-True (
            -not (Test-Json -Json ($dirtyWithoutAuthority | ConvertTo-Json -Depth 100) `
                -SchemaFile $receiptSchemaPath -ErrorAction SilentlyContinue)
        ) "Receipt schema accepted dirty execution without explicit authority."
        $commandDamages = @()
        $passedTimedOut = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $passedTimedOut.commands[0].timed_out = $true
        $commandDamages += $passedTimedOut
        $passedNullExit = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $passedNullExit.commands[0].exit_code = $null
        $commandDamages += $passedNullExit
        $passedFailureReason = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $passedFailureReason.commands[0].failure_reason = "nonzero-exit"
        $commandDamages += $passedFailureReason
        $passedCleanup = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $passedCleanup.commands[0].cleanup.attempted = $true
        $commandDamages += $passedCleanup
        $passedDuration = $typed | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $passedDuration.commands[0].duration_ms = [int64]$passedDuration.commands[0].duration_ms + 1
        $commandDamages += $passedDuration
        for ($damageIndex = 0; $damageIndex -lt $commandDamages.Count; $damageIndex++) {
            try {
                Test-FleetValidationRunReceipt -Receipt $commandDamages[$damageIndex] `
                    -RepositoryRoot $scratch -ConfigPath $configPath
                throw "Incoherent passing command receipt $damageIndex was accepted."
            } catch {
                Assert-True (
                    $_.Exception.Message -match "command receipt|duration"
                ) "Command damage $damageIndex failed for the wrong reason."
            }
            if ($damageIndex -lt 4) {
                Assert-True (
                    -not (Test-Json -Json (
                        $commandDamages[$damageIndex] | ConvertTo-Json -Depth 100
                    ) -SchemaFile $receiptSchemaPath -ErrorAction SilentlyContinue)
                ) "Receipt schema accepted structural command damage $damageIndex."
            }
        }
    } finally {
        if ($executePath) {
            Remove-Item -LiteralPath $executePath -Force -ErrorAction SilentlyContinue
        }
    }

    $defaultExecute = Invoke-FleetValidationGuardrail $scratch $configPath "focused" $null -Execute
    $defaultExecutePath = Get-TestReceiptPath $scratch $defaultExecute
    Assert-True (Test-Path -LiteralPath $defaultExecutePath) `
        "Default execution receipt was not written to its unique run-ID path."
    Assert-True (-not (Test-Path -LiteralPath (
        Get-TestPendingReceiptPath $scratch $defaultExecute
    ))) "Completed execution retained its in-progress receipt."
    Remove-Item -LiteralPath $defaultExecutePath -Force

    $transientConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($transientConfig.checks | Where-Object minimum_profile -EQ "focused").file = `
        "tools/transient-child.ps1"
    $transientConfigPath = Join-Path $scratch "transient-child-risk.json"
    Write-Json $transientConfigPath $transientConfig
    $transientPath = $null
    try {
        $transient = Invoke-FleetValidationGuardrail $scratch $transientConfigPath `
            "focused" $null -Execute -AllowDirtySource
        $transientPath = Get-TestReceiptPath $scratch $transient
        Assert-True (
            $transient.result -eq "passed" -and
            $transient.commands[0].status -eq "passed" -and
            $transient.commands[0].exit_code -eq 0 -and
            -not $transient.commands[0].cleanup.attempted
        ) "A naturally draining owned child produced a false leak failure."
    } finally {
        Remove-Item -LiteralPath $transientConfigPath -Force -ErrorAction SilentlyContinue
        if ($transientPath) {
            Remove-Item -LiteralPath $transientPath -Force -ErrorAction SilentlyContinue
        }
    }

    $leakingConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($leakingConfig.checks | Where-Object minimum_profile -EQ "focused").file = `
        "tools/leaking-child.ps1"
    $leakingConfigPath = Join-Path $scratch "leaking-child-risk.json"
    Write-Json $leakingConfigPath $leakingConfig
    $leakingPath = $null
    try {
        $leaking = Invoke-FleetValidationGuardrail $scratch $leakingConfigPath `
            "focused" $null -Execute -AllowDirtySource
        $leakingPath = Get-TestReceiptPath $scratch $leaking
        Assert-True (
            $leaking.result -eq "failed" -and
            $leaking.commands[0].status -eq "failed" -and
            $leaking.commands[0].exit_code -eq 0 -and
            $leaking.commands[0].failure_reason -eq "owned-child-leak" -and
            $leaking.commands[0].cleanup.attempted -and
            $leaking.commands[0].cleanup.completed -and
            $leaking.commands[0].cleanup.survivors.Count -eq 0
        ) "A persistent owned child did not fail closed after the drain grace."
    } finally {
        Remove-Item -LiteralPath $leakingConfigPath -Force -ErrorAction SilentlyContinue
        if ($leakingPath) {
            Remove-Item -LiteralPath $leakingPath -Force -ErrorAction SilentlyContinue
        }
    }

    $recoveredConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($recoveredConfig.checks | Where-Object minimum_profile -EQ "standard").file = `
        "tools/recovered-native.ps1"
    $recoveredConfigPath = Join-Path $scratch "recovered-risk.json"
    Write-Json $recoveredConfigPath $recoveredConfig
    $recoveredPath = $null
    try {
        $recovered = Invoke-FleetValidationGuardrail $scratch $recoveredConfigPath `
            "standard" $null -Execute -AllowDirtySource
        $recoveredPath = Get-TestReceiptPath $scratch $recovered
        Assert-True (
            $recovered.result -eq "passed" -and
            $recovered.commands[0].status -eq "passed" -and
            $recovered.commands[0].exit_code -eq 0
        ) "A handled native failure leaked stale LASTEXITCODE into a false failure."
    } finally {
        Remove-Item -LiteralPath $recoveredConfigPath -Force -ErrorAction SilentlyContinue
        if ($recoveredPath) {
            Remove-Item -LiteralPath $recoveredPath -Force -ErrorAction SilentlyContinue
        }
    }

    $driftPath = $null
    try {
        $drift = Invoke-FleetValidationGuardrail $scratch $configPath "standard" $null `
            -Execute -AllowDirtySource
        $driftPath = Get-TestReceiptPath $scratch $drift
        Assert-True ($drift.result -eq "rejected" -and $drift.git.drift_detected) `
            "Repository drift was not rejected."
    } finally {
        & git -C $scratch checkout --quiet -- README.md
        if ($driftPath) {
            Remove-Item -LiteralPath $driftPath -Force -ErrorAction SilentlyContinue
        }
    }

    Set-Content -LiteralPath (Join-Path $scratch "README.md") -Value "index-only drift" -Encoding utf8NoBOM
    $indexConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($indexConfig.checks | Where-Object minimum_profile -EQ "standard").file = "tools/index-drift.ps1"
    $indexConfigPath = Join-Path $scratch "index-risk.json"
    Write-Json $indexConfigPath $indexConfig
    $indexReceiptPath = $null
    try {
        $indexDrift = Invoke-FleetValidationGuardrail $scratch $indexConfigPath "standard" $null `
            -Execute -AllowDirtySource
        $indexReceiptPath = Get-TestReceiptPath $scratch $indexDrift
        Assert-True ($indexDrift.result -eq "rejected" -and $indexDrift.git.drift_detected) `
            "Index-only drift was not rejected."
    } finally {
        & git -C $scratch reset --quiet HEAD -- README.md
        & git -C $scratch checkout --quiet -- README.md
        Remove-Item -LiteralPath $indexConfigPath -Force -ErrorAction SilentlyContinue
        if ($indexReceiptPath) {
            Remove-Item -LiteralPath $indexReceiptPath -Force -ErrorAction SilentlyContinue
        }
    }

    $failureConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($failureConfig.checks | Where-Object minimum_profile -EQ "standard").file = "tools/fail-child.ps1"
    $failureConfigPath = Join-Path $scratch "failure-risk.json"
    Write-Json $failureConfigPath $failureConfig
    $failurePath = $null
    try {
        $failure = Invoke-FleetValidationGuardrail $scratch $failureConfigPath "standard" $null `
            -Execute -AllowDirtySource
        $failurePath = Get-TestReceiptPath $scratch $failure
        if (
            $failure.result -ne "failed" -or
            $failure.commands[0].status -ne "failed" -or
            $failure.commands[0].exit_code -ne 7 -or
            -not $failure.commands[0].cleanup.attempted -or
            -not $failure.commands[0].cleanup.completed -or
            $failure.commands[0].cleanup.survivors.Count -ne 0
        ) {
            Write-Host ($failure | ConvertTo-Json -Depth 100)
        }
        Assert-True (
            $failure.result -eq "failed" -and
            $failure.commands[0].status -eq "failed" -and
            $failure.commands[0].exit_code -eq 7 -and
            $failure.commands[0].cleanup.attempted -and
            $failure.commands[0].cleanup.completed -and
            $failure.commands[0].cleanup.survivors.Count -eq 0
        ) "Nonzero child failure did not clean its owned process tree."
    } finally {
        Remove-Item -LiteralPath $failureConfigPath -Force -ErrorAction SilentlyContinue
        if ($failurePath) {
            Remove-Item -LiteralPath $failurePath -Force -ErrorAction SilentlyContinue
        }
    }

    $throwConfig = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    ($throwConfig.checks | Where-Object minimum_profile -EQ "standard").file = "tools/throw.ps1"
    $throwConfigPath = Join-Path $scratch "throw-risk.json"
    Write-Json $throwConfigPath $throwConfig
    $throwPath = $null
    try {
        $throwFailure = Invoke-FleetValidationGuardrail $scratch $throwConfigPath `
            "standard" $null -Execute -AllowDirtySource
        $throwPath = Get-TestReceiptPath $scratch $throwFailure
        Assert-True (
            $throwFailure.result -eq "failed" -and
            $throwFailure.commands[0].status -eq "failed" -and
            $throwFailure.commands[0].exit_code -eq 1 -and
            $throwFailure.commands[0].failure_reason -eq "nonzero-exit"
        ) "Terminating PowerShell error produced a false passing receipt."
    } finally {
        Remove-Item -LiteralPath $throwConfigPath -Force -ErrorAction SilentlyContinue
        if ($throwPath) {
            Remove-Item -LiteralPath $throwPath -Force -ErrorAction SilentlyContinue
        }
    }

    $timeoutPath = $null
    try {
        $timeoutInformation = @()
        $timeoutWarnings = @()
        $timeout = Invoke-FleetValidationGuardrail $scratch $configPath "release" $null `
            -Execute -InformationVariable timeoutInformation -WarningVariable timeoutWarnings
        $timeoutPath = Get-TestReceiptPath $scratch $timeout
        Assert-True ($timeout.result -eq "failed") "Timeout did not fail aggregate."
        Assert-True (
            $timeout.commands[0].timed_out -and
            $timeout.commands[0].cleanup.attempted -and
            $timeout.commands[0].cleanup.completed -and
            $timeout.commands[0].cleanup.survivors.Count -eq 0
        ) "Timeout cleanup receipt is incomplete."
        Assert-True (
            (($timeoutInformation | Out-String) -match "timeout stdout survived") -and
            (($timeoutWarnings | Out-String) -match "timeout stderr survived")
        ) "Partial stdout/stderr produced before timeout did not survive Job Object cleanup."
    } finally {
        if ($timeoutPath) {
            Remove-Item -LiteralPath $timeoutPath -Force -ErrorAction SilentlyContinue
        }
    }

    $existingPath = Join-Path $scratch "artifacts/validation/existing.json"
    New-Item -ItemType Directory -Path (Split-Path -Parent $existingPath) -Force | Out-Null
    Set-Content -LiteralPath $existingPath -Value "do-not-overwrite" -Encoding utf8NoBOM
    $uniqueReceiptPath = $null
    try {
        $uniqueReceipt = Invoke-FleetValidationGuardrail $scratch $configPath "focused" $null `
            -Execute
        $uniqueReceiptPath = Get-TestReceiptPath $scratch $uniqueReceipt
        Assert-True (Test-Path -LiteralPath $uniqueReceiptPath) `
            "Execution did not choose its run-ID receipt path."
        Assert-True ((Get-Content -LiteralPath $existingPath -Raw).Trim() -eq "do-not-overwrite") `
            "Run-ID receipt creation overwrote an unrelated existing file."
    } finally {
        Remove-Item -LiteralPath $existingPath -Force
        if ($uniqueReceiptPath) {
            Remove-Item -LiteralPath $uniqueReceiptPath -Force -ErrorAction SilentlyContinue
        }
    }

    $validationRoot = Join-Path $scratch "artifacts/validation"
    Assert-True (
        -not (Test-Path -LiteralPath $validationRoot) -or
        @(Get-ChildItem -Force -LiteralPath $validationRoot).Count -eq 0
    ) "Receipt reparse test requires an empty validation directory."
    if (Test-Path -LiteralPath $validationRoot) {
        Remove-Item -LiteralPath $validationRoot -Force
    }
    $junctionTarget = Join-Path ([IO.Path]::GetTempPath()) (
        "fleet-receipt-junction-{0}" -f [guid]::NewGuid()
    )
    New-Item -ItemType Directory -Path $junctionTarget | Out-Null
    New-Item -ItemType Junction -Path $validationRoot -Target $junctionTarget | Out-Null
    try {
        Invoke-FleetValidationGuardrail $scratch $configPath "focused" $null -Execute
        throw "Receipt-directory reparse point was accepted."
    } catch {
        Assert-True ($_.Exception.Message -match "reparse point") `
            "Receipt-directory reparse point failed incorrectly."
        Assert-True (@(Get-ChildItem -Force -LiteralPath $junctionTarget).Count -eq 0) `
            "Receipt bytes escaped through a rejected reparse point."
    } finally {
        Remove-Item -LiteralPath $validationRoot -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $junctionTarget) {
            Remove-Item -LiteralPath $junctionTarget -Force
        }
    }

    $forbidden = $config | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
    $forbidden.checks[0].file = "adb"
    $forbiddenPath = Join-Path $scratch "forbidden.json"
    Write-Json $forbiddenPath $forbidden
    try {
        Invoke-FleetValidationGuardrail $scratch $forbiddenPath "focused" $null
        throw "Device command was accepted."
    } catch {
        Assert-True ($_.Exception.Message -match "Forbidden device command") "Device command rejection failed incorrectly."
    }

    Write-Host "Fleet validation guardrail self-tests passed."
} finally {
    if (Test-Path -LiteralPath $scratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
}
