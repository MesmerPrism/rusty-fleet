# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet("Plan", "Preflight", "Execute", "Resume", "Cleanup", "Status")]
    [string] $Action,

    [Parameter(Mandatory)]
    [string] $RunConfig,

    [switch] $ConfirmMutation,
    [switch] $ConfirmCurrentCheckpoint
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($PSVersionTable.PSEdition -ne "Core" -or
    $PSVersionTable.PSVersion -lt [version]"7.6") {
    throw "Rusty Fleet acceptance requires PowerShell 7.6 Core or newer."
}

Import-Module (Join-Path $PSScriptRoot "FleetWifiAdbTwoQuestAcceptance.psm1") `
    -Force

try {
    $result = Invoke-FleetWifiAdbTwoQuestAcceptance `
        -Action $Action `
        -RunConfig $RunConfig `
        -ConfirmMutation:$ConfirmMutation `
        -ConfirmCurrentCheckpoint:$ConfirmCurrentCheckpoint
    $result | ConvertTo-Json -Depth 32
} catch {
    $message = [string]$_.Exception.Message
    $code = if ($message -match '^([a-z0-9_]+):') {
        $Matches[1]
    } else {
        "acceptance_runner_failed"
    }
    [ordered]@{
        schema = "rusty.fleet.wifi_adb_two_quest_error.v1"
        status = "rejected"
        reason_code = $code
        message = "The private acceptance transaction failed without exposing private details."
    } | ConvertTo-Json -Depth 4
    exit 2
}
