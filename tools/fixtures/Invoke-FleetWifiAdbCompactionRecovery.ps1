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

Import-Module -Name $ModulePath -Force -DisableNameChecking
$context = Read-ValidatedRunConfig -RunConfig $RunConfig
$state = Read-SanitizedState -Context $context
[void](Assert-AgentBoardReservation -Context $context -State $state)

$checks = [ordered]@{}
foreach ($entry in $state.cleanup.checks.GetEnumerator()) {
    $checks[[string]$entry.Key] = $entry.Value
}

$ownerOperation = {
    $owner = Get-Content -LiteralPath $OwnerStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 8
    $exact = @($owner.Keys | Sort-Object)
    if ((
            Compare-Object $exact @(
                "dispatch_count",
                "effect_applied",
                "recovered",
                "schema",
                "unsafe_effect_count"
            )
        ).Count -ne 0 -or
        [string]$owner.schema -cne
            "rusty.fleet.wifi_adb_compaction_test_owner.v1") {
        throw "The modeled cleanup owner state is invalid."
    }
    $owner.dispatch_count = [int]$owner.dispatch_count + 1
    if (-not [bool]$owner.effect_applied) {
        $owner.effect_applied = $true
        $owner.unsafe_effect_count =
            [int]$owner.unsafe_effect_count + 1
    }
    [IO.File]::WriteAllText(
        $OwnerStatePath,
        ($owner | ConvertTo-Json -Depth 8) + [Environment]::NewLine,
        [Text.UTF8Encoding]::new($false))
    return [bool]$owner.recovered
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
if ([string]$truth.Status -cne "complete") {
    throw "The fresh-process cleanup recovery did not become complete."
}
$state.cleanup.checks = $checks
$state.cleanup.status = $truth.Status
$state.status = "complete"
$state.phase = "cleanup"
Write-SanitizedState -Context $context -State $state
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
} | ConvertTo-Json -Compress
