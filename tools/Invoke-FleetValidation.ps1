# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [ValidateSet("focused", "standard", "release", "Quick", "Standard", "Deep")]
    [string] $Profile,
    [string] $BaseCommit,
    [switch] $AllowDirtySource,
    [switch] $Execute
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$repoRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot "Fleet.ValidationGuardrails.psm1") -Force

$parameters = @{
    RepositoryRoot = $repoRoot
    ConfigPath = Join-Path $repoRoot "config/fleet-validation-risk.v1.json"
    Execute = $Execute
    AllowDirtySource = $AllowDirtySource
}
if ($PSBoundParameters.ContainsKey("Profile")) { $parameters.Profile = $Profile }
if ($BaseCommit) { $parameters.BaseCommit = $BaseCommit }

$receipt = Invoke-FleetValidationGuardrail @parameters
$receipt | ConvertTo-Json -Depth 100
if ($Execute -and $receipt.result -ne "passed") { exit 1 }
