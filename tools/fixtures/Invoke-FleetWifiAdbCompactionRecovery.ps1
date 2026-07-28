# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $ModulePath,
    [Parameter(Mandatory)][string] $RunConfig,
    [Parameter(Mandatory)][string] $OwnerStatePath
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

$state = Read-SanitizedState -Context $context
$preRecovery = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerState, $InnerBinding)
    Assert-ModeledRecoveryFixtureState `
        -State $InnerState -Binding $InnerBinding
} $state $fixtureBinding

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
        param($InnerBinding)
        Read-PinnedModeledRecoveryFixtureOwner `
            -Path $InnerBinding.OwnerPath `
            -ExpectedSha256 $InnerBinding.OwnerSha256 `
            -ExpectedChainIdentity $InnerBinding.OwnerChainIdentity
    } $fixtureBinding
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
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($serializedOwner)
    $stream = [IO.File]::Open(
        [string]$fixtureBinding.OwnerPath,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Write,
        [IO.FileShare]::Read)
    try {
        $stream.SetLength(0)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }

    $afterSha256 = (
        Get-FileHash -LiteralPath $fixtureBinding.OwnerPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $ownerAfter = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($InnerBinding, $InnerAfterSha256)
        Read-PinnedModeledRecoveryFixtureOwner `
            -Path $InnerBinding.OwnerPath `
            -ExpectedSha256 $InnerAfterSha256 `
            -ExpectedChainIdentity $InnerBinding.OwnerChainIdentity
    } $fixtureBinding $afterSha256
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

$state = Read-SanitizedState -Context $context
$ownerPostReadback = & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
    param($InnerBinding, $InnerAfterSha256)
    Read-PinnedModeledRecoveryFixtureOwner `
        -Path $InnerBinding.OwnerPath `
        -ExpectedSha256 $InnerAfterSha256 `
        -ExpectedChainIdentity $InnerBinding.OwnerChainIdentity
} $fixtureBinding $recoveryEvidence.owner_after_sha256
if (
    [string]$state.cleanup.status -cne "complete" -or
    [string]$state.status -cne "complete" -or
    $state.cleanup.checks["owner-compaction-retry"] -ne $true -or
    [long]$ownerPostReadback.Value.dispatch_count -ne
        [long]$recoveryEvidence.dispatch_after
) {
    throw "The fresh recovery evidence did not survive final durable readback."
}
[void](Release-AgentBoardReservation -Context $context -State $state)

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
} | ConvertTo-Json -Compress
