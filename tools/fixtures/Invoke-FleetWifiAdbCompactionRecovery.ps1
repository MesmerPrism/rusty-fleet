# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $ModulePath,
    [Parameter(Mandatory)][string] $RunConfig,
    [Parameter(Mandatory)][string] $OwnerStatePath,
    [ValidateSet(
        "none",
        "owner-leaf-substitution",
        "owner-ancestor-substitution",
        "state-leaf-substitution"
    )]
    [string] $TestRaceCase = "none"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($PSVersionTable.PSEdition -ne "Core" -or
    $PSVersionTable.PSVersion -lt [version]"7.6") {
    throw "The compaction-recovery fixture requires PowerShell 7.6 Core or newer."
}

$expectedModulePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot `
    "..\FleetWifiAdbTwoQuestAcceptance.psm1"))
$resolvedModulePath = [IO.Path]::GetFullPath($ModulePath)
if ($resolvedModulePath -cne $expectedModulePath) {
    throw "The compaction-recovery fixture requires its exact tracked module."
}

Import-Module -Name $resolvedModulePath -Force -DisableNameChecking
$context = Read-ValidatedRunConfig -RunConfig $RunConfig
$fixtureBinding = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerContext, $InnerOwnerStatePath)
    Get-ModeledRecoveryFixtureBinding `
        -Context $InnerContext -OwnerStatePath $InnerOwnerStatePath
} $context $OwnerStatePath

$ownerLease = $fixtureBinding.OwnerLease
$finalStateLease = $null
try {
$state = Read-SanitizedState -Context $context
$preRecovery = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerState, $InnerBinding)
    Assert-ModeledRecoveryFixtureState `
        -State $InnerState -Binding $InnerBinding
} $state $fixtureBinding

if ($TestRaceCase -ceq "owner-leaf-substitution") {
    $replacement = Join-Path $fixtureBinding.TestRoot `
        "owner-leaf-race-replacement.json"
    if (-not (Test-Path -LiteralPath $replacement -PathType Leaf)) {
        throw "The owner leaf race fixture is missing."
    }
    try {
        Move-Item -LiteralPath $replacement `
            -Destination $fixtureBinding.OwnerPath -Force -ErrorAction Stop
    } catch {
        # The retained writable owner lease must deny path replacement.
    }
    try {
        $ownerLease.VerifyAfter()
    } catch {
        throw "modeled_recovery_owner_race_detected"
    }
    throw "modeled_recovery_owner_race_detected"
}
if ($TestRaceCase -ceq "owner-ancestor-substitution") {
    $ownerDirectory = Split-Path -Parent $fixtureBinding.OwnerPath
    $movedDirectory =
        Join-Path $fixtureBinding.TestRoot "compaction-owner-moved"
    $replacementDirectory =
        Join-Path $fixtureBinding.TestRoot "owner-ancestor-race-target"
    if (-not (Test-Path -LiteralPath $replacementDirectory `
            -PathType Container)) {
        throw "The owner ancestor race fixture is missing."
    }
    try {
        Move-Item -LiteralPath $ownerDirectory `
            -Destination $movedDirectory -ErrorAction Stop
        New-Item -ItemType Junction -Path $ownerDirectory `
            -Target $replacementDirectory -ErrorAction Stop | Out-Null
    } catch {
        # Every held path component denies ancestor rename/substitution.
    }
    try {
        $ownerLease.VerifyAfter()
    } catch {
        throw "modeled_recovery_owner_race_detected"
    }
    throw "modeled_recovery_owner_race_detected"
}

# No reservation state is read and no Agent Board process is contacted until
# the complete synthetic context, owner binding, and compacted recovery state
# above have all passed.
[void](Assert-AgentBoardReservation -Context $context -State $state)

$checks = [ordered]@{}
foreach ($entry in $state.cleanup.checks.GetEnumerator()) {
    $checks[[string]$entry.Key] = $entry.Value
}

$recoveryEvidence = [ordered]@{
    completed = $false
    owner_before_sha256 = [string]$fixtureBinding.OwnerSha256
    owner_after_sha256 = ""
    dispatch_before = [long]$preRecovery.DispatchCount
    dispatch_after = 0L
}
$ownerOperation = {
    $ownerRead = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($InnerLease)
        Read-ModeledRecoveryFixtureOwnerFromLease -Lease $InnerLease
    } $ownerLease
    $owner = $ownerRead.Value
    if (
        [long]$owner.dispatch_count -ne
            [long]$recoveryEvidence.dispatch_before -or
        [bool]$owner.effect_applied -ne $true -or
        [long]$owner.unsafe_effect_count -ne 1L -or
        [bool]$owner.recovered -ne $true
    ) {
        throw "The modeled cleanup owner did not match its bound recovery pre-state."
    }

    $owner.dispatch_count = [long]$owner.dispatch_count + 1L
    $serializedOwner =
        ($owner | ConvertTo-Json -Depth 8) + [Environment]::NewLine
    $afterSha256 = $ownerLease.RewriteUtf8Text(
        $serializedOwner,
        65536)
    $ownerAfter = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($InnerLease)
        Read-ModeledRecoveryFixtureOwnerFromLease -Lease $InnerLease
    } $ownerLease
    if (
        [long]$ownerAfter.Value.dispatch_count -ne
            [long]$recoveryEvidence.dispatch_before + 1L -or
        [bool]$ownerAfter.Value.effect_applied -ne $true -or
        [long]$ownerAfter.Value.unsafe_effect_count -ne 1L -or
        [bool]$ownerAfter.Value.recovered -ne $true -or
        [string]$ownerAfter.Sha256 -ceq
            [string]$recoveryEvidence.owner_before_sha256
    ) {
        throw "The modeled cleanup owner post-operation readback is invalid."
    }
    $recoveryEvidence.owner_after_sha256 = [string]$ownerAfter.Sha256
    $recoveryEvidence.dispatch_after =
        [long]$ownerAfter.Value.dispatch_count
    $recoveryEvidence.completed = $true
    return $true
}.GetNewClosure()

& (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param(
        $InnerContext,
        $InnerState,
        $InnerChecks,
        $InnerName,
        $InnerOperation
    )
    Invoke-JournaledCleanupStep `
        -Context $InnerContext -State $InnerState `
        -Checks $InnerChecks -Name $InnerName `
        -ModeledNoDeviceProjection -Operation $InnerOperation
} $context $state $checks "owner-compaction-retry" $ownerOperation

$truth = Get-CleanupTruth -Checks $checks
$postRecoveryRecords =
    [long]$state.mutation_history_summary.compacted_count +
    [long]@($state.mutation_history).Count
$lastMutation = @($state.mutation_history)[-1]
if (
    $recoveryEvidence.completed -ne $true -or
    [long]$recoveryEvidence.dispatch_after -ne
        [long]$preRecovery.DispatchCount + 1L -or
    [string]$recoveryEvidence.owner_after_sha256 -notmatch
        '^[0-9a-f]{64}$' -or
    $checks["owner-compaction-retry"] -ne $true -or
    [string]$truth.Status -cne "complete" -or
    $postRecoveryRecords -ne [long]$preRecovery.TotalRecords + 1L -or
    [string]$state.journal_head_sha256 -ceq
        [string]$preRecovery.JournalHeadSha256 -or
    [string]$lastMutation.action_id -cne
        "acceptance.cleanup.owner-compaction-retry" -or
    [string]$lastMutation.stage -cne "confirmed" -or
    [string]$lastMutation.reconciliation_code -cne
        "cleanup_exact_readback_confirmed"
) {
    throw "The fresh-process cleanup recovery did not become complete."
}
$state.cleanup.checks = $checks
$state.cleanup.status = $truth.Status
$state.status = "complete"
$state.phase = "cleanup"
Write-SanitizedState -Context $context -State $state

$finalStateBinding = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerContext, $InnerState, $InnerPreRecovery)
    Get-ModeledRecoveryFinalStateBinding `
        -Context $InnerContext -State $InnerState `
        -PreRecovery $InnerPreRecovery
} $context $state $preRecovery
$finalStateLease = $finalStateBinding.Lease
$ownerPostReadback = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerLease)
    Read-ModeledRecoveryFixtureOwnerFromLease -Lease $InnerLease
} $ownerLease
if (
    [string]$finalStateBinding.State.cleanup.status -cne "complete" -or
    [string]$finalStateBinding.State.status -cne "complete" -or
    $finalStateBinding.State.cleanup.checks[
        "owner-compaction-retry"] -ne $true -or
    [long]$ownerPostReadback.Value.dispatch_count -ne
        [long]$recoveryEvidence.dispatch_after -or
    [string]$ownerPostReadback.Sha256 -cne
        [string]$recoveryEvidence.owner_after_sha256
) {
    throw "The fresh recovery evidence did not survive final durable readback."
}

if ($TestRaceCase -ceq "state-leaf-substitution") {
    $replacement = Join-Path $fixtureBinding.TestRoot `
        "state-leaf-race-replacement.json"
    if (-not (Test-Path -LiteralPath $replacement -PathType Leaf)) {
        throw "The final state leaf race fixture is missing."
    }
    try {
        Move-Item -LiteralPath $replacement `
            -Destination $finalStateBinding.Path -Force -ErrorAction Stop
    } catch {
        # The retained final-state lease must deny leaf substitution.
    }
    try {
        $finalStateLease.VerifyAfter()
        $ownerLease.VerifyAfter()
    } catch {
        throw "modeled_recovery_state_race_detected"
    }
    throw "modeled_recovery_state_race_detected"
}

$releaseReceipt = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param(
        $InnerContext,
        $InnerState,
        $InnerOwnerLease,
        $InnerStateLease
    )
    Invoke-AgentBoardReservationRelease `
        -Context $InnerContext -State $InnerState `
        -DeferStateProjection `
        -RetainedOwnerLease $InnerOwnerLease `
        -RetainedStateLease $InnerStateLease
} $context $state $ownerLease $finalStateLease

# The irreversible external release decision is accepted only while both the
# exact post-operation owner and exact recovered state handles/path chains are
# still held and hash-verified.
$finalStateLease.VerifyAfter()
$ownerLease.VerifyAfter()
$stateTextAfterRelease =
    $finalStateLease.ReadUtf8Text(1048576)
$ownerAfterRelease = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerLease)
    Read-ModeledRecoveryFixtureOwnerFromLease -Lease $InnerLease
} $ownerLease
if (
    $stateTextAfterRelease -cne $finalStateBinding.Text -or
    [string]$finalStateLease.Sha256 -cne
        [string]$finalStateBinding.Sha256 -or
    [string]$ownerAfterRelease.Sha256 -cne
        [string]$recoveryEvidence.owner_after_sha256 -or
    [string]$releaseReceipt.state -cne "released"
) {
    throw "The pinned recovery evidence changed during reservation release."
}

$finalStateLease.Dispose()
$finalStateLease = $null
$ownerLease.Dispose()
$ownerLease = $null
Write-SanitizedState -Context $context -State $state
$state = Read-SanitizedState -Context $context
$statePath = Join-Path (
    [string]$context.Config.private_state_root
) "acceptance-state.json"
$totalRecords =
    [long]$state.mutation_history_summary.compacted_count +
    [long]@($state.mutation_history).Count

[ordered]@{
    schema = "rusty.fleet.wifi_adb_compaction_recovery_fixture.v1"
    result = "pass"
    status = [string]$state.status
    cleanup_status = [string]$state.cleanup.status
    agent_board_reservation = [string]$state.agent_board_reservation
    total_records = $totalRecords
    recent_records = [long]@($state.mutation_history).Count
    compacted_records =
        [long]$state.mutation_history_summary.compacted_count
    state_size_bytes = [long](Get-Item -LiteralPath $statePath).Length
    recovery_evidence_sha256 =
        [string]$recoveryEvidence.owner_after_sha256
    recovered_state_sha256 =
        [string]$finalStateBinding.Sha256
} | ConvertTo-Json -Compress
} finally {
    if ($null -ne $finalStateLease) {
        $finalStateLease.Dispose()
    }
    if ($null -ne $ownerLease) {
        $ownerLease.Dispose()
    }
}
