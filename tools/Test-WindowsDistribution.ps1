# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$distributionTests = @(
    "..\packaging\windows\tests\Test-WindowsDistribution.ps1",
    "..\packaging\windows\tests\Test-WindowsReleaseDescriptor.ps1"
)
foreach ($relative in $distributionTests) {
    $distributionTest = Join-Path $PSScriptRoot $relative
    if (-not (Test-Path -LiteralPath $distributionTest -PathType Leaf)) {
        throw "Windows distribution validation entrypoint is missing: $relative"
    }
    & $distributionTest
}
